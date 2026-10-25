//! A program's regular expression literals compiled to C: QuickJS's libregexp (linked into the
//! compiler, see build.rs) compiles each pattern to the bytecode the program's runtime would make,
//! and that bytecode becomes a C function doing what the engine's interpreter (lre_exec_backtrack)
//! does for it, on 8-bit (ASCII) text: no opcode dispatch, operands constants, character classes
//! bit tables. The runtime uses one only for the very bytecode it was made from (tv_rejit_table),
//! so where the compiler and the runtime disagree the interpreter still runs.

use std::ffi::{c_char, c_int, c_void};
use std::fmt::Write as _;

unsafe extern "C" {
    fn lre_compile(plen: *mut c_int, error_msg: *mut c_char, error_msg_size: c_int, buf: *const c_char, buf_len: usize, re_flags: c_int, opaque: *mut c_void) -> *mut u8;
    fn lre_canonicalize(c: u32, is_unicode: c_int) -> c_int;
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

// (what libregexp asks of the program it's in)
#[unsafe(no_mangle)]
extern "C" fn lre_realloc(_opaque: *mut c_void, ptr: *mut c_void, size: usize) -> *mut c_void {
    unsafe {
        if size == 0 {
            free(ptr);
            std::ptr::null_mut()
        } else if ptr.is_null() {
            malloc(size)
        } else {
            realloc(ptr, size)
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn lre_check_timeout(_opaque: *mut c_void) -> c_int {
    0
}

#[unsafe(no_mangle)]
extern "C" fn lre_check_stack_overflow(_opaque: *mut c_void, _size: usize) -> c_int {
    0
}

const FLAG_GLOBAL: c_int = 1 << 0;
const FLAG_IGNORECASE: c_int = 1 << 1;
const FLAG_MULTILINE: c_int = 1 << 2;
const FLAG_DOTALL: c_int = 1 << 3;
const FLAG_UNICODE: c_int = 1 << 4;
const FLAG_STICKY: c_int = 1 << 5;
const FLAG_INDICES: c_int = 1 << 6;
const FLAG_UNICODE_SETS: c_int = 1 << 8;

/// `flags` as the engine's bits and in RegExp.prototype.flags' order, as the runtime reads them
/// (tv_native_reCompile); None for flags it would reject.
fn parse_flags(flags: &str) -> Option<(c_int, String)> {
    const ORDER: &str = "dgimsuvy";
    let mut seen = [false; 8];
    let mut bits = 0;
    for ch in flags.chars() {
        let k = ORDER.find(ch)?;
        if seen[k] {
            return None;
        }
        seen[k] = true;
        bits |= match ch {
            'd' => FLAG_INDICES,
            'g' => FLAG_GLOBAL,
            'i' => FLAG_IGNORECASE,
            'm' => FLAG_MULTILINE,
            's' => FLAG_DOTALL,
            'u' => FLAG_UNICODE,
            'v' => FLAG_UNICODE_SETS,
            'y' => FLAG_STICKY,
            _ => return None,
        };
    }
    let text: String = ORDER.chars().zip(seen).filter(|&(_, s)| s).map(|(c, _)| c).collect();
    Some((bits, text))
}

/// The engine's bytecode for `source` with `bits` (header and code, no group names).
fn compile(source: &str, bits: c_int) -> Option<Vec<u8>> {
    let mut len: c_int = 0;
    let mut err = [0 as c_char; 128];
    let bc = unsafe { lre_compile(&mut len, err.as_mut_ptr(), err.len() as c_int, source.as_ptr() as *const c_char, source.len(), bits, std::ptr::null_mut()) };
    if bc.is_null() {
        return None;
    }
    let all = unsafe { std::slice::from_raw_parts(bc, len as usize) }.to_vec();
    lre_realloc(std::ptr::null_mut(), bc as *mut c_void, 0);
    if all.len() < 8 {
        return None;
    }
    let code_len = u32::from_le_bytes([all[4], all[5], all[6], all[7]]) as usize;
    (8 + code_len <= all.len()).then(|| all[..8 + code_len].to_vec())
}

/// The opcodes, in libregexp-opcode.h's order: (name, size).
fn opcodes() -> Vec<(&'static str, usize)> {
    const H: &str = include_str!("../../../../vendor/quickjs-regexp/libregexp-opcode.h");
    H.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix("DEF(")?;
            let (name, size) = rest.split_once(',')?;
            let size = size.trim().trim_end_matches(|c: char| !c.is_ascii_digit()).trim_end_matches(')');
            let size: String = size.chars().take_while(|c| c.is_ascii_digit()).collect();
            Some((name.trim(), size.parse().ok()?))
        })
        .collect()
}

fn u16_at(b: &[u8], i: usize) -> u32 {
    u16::from_le_bytes([b[i], b[i + 1]]) as u32
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// The bytes 0..256 a one-character atom at `at` (an opcode and its operands) takes.
fn atom_set(code: &[u8], at: usize, name: &str, unicode: bool) -> Option<[bool; 256]> {
    let canon = |c: u32| unsafe { lre_canonicalize(c, unicode as c_int) } as u32;
    let mut set = [false; 256];
    match name {
        "char" | "char_i" => {
            let v = u16_at(code, at + 1);
            for (b, slot) in set.iter_mut().enumerate() {
                let c = if name == "char_i" { canon(b as u32) } else { b as u32 };
                *slot = c == v;
            }
        }
        "char32" | "char32_i" => {}
        "dot" => {
            for (b, slot) in set.iter_mut().enumerate() {
                *slot = b != 10 && b != 13;
            }
        }
        "any" => set = [true; 256],
        "space" | "not_space" => {
            for (b, slot) in set.iter_mut().enumerate() {
                let sp = is_space_byte(b as u8);
                *slot = if name == "space" { sp } else { !sp };
            }
        }
        "range" | "range_i" | "range32" | "range32_i" => {
            let n = u16_at(code, at + 1) as usize;
            let wide = name.starts_with("range32");
            let fold = name.ends_with("_i");
            for (b, slot) in set.iter_mut().enumerate() {
                let c = if fold { canon(b as u32) } else { b as u32 };
                *slot = (0..n).any(|k| {
                    let (lo, hi) = if wide { (u32_at(code, at + 3 + k * 8), u32_at(code, at + 3 + k * 8 + 4)) } else { (u16_at(code, at + 3 + k * 4), u16_at(code, at + 3 + k * 4 + 2)) };
                    // (16-bit ranges: a last bound of 0xffff is no bound)
                    let hi = if !wide && k == n - 1 && hi == 0xffff { u32::MAX } else { hi };
                    c >= lo && c <= hi
                });
            }
        }
        _ => return None,
    }
    Some(set)
}

/// lre_is_space_byte: the Latin-1 space characters.
fn is_space_byte(b: u8) -> bool {
    matches!(b, 9..=13 | 32 | 0xA0)
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// C for "byte `c` is in `set`": a comparison or two, else a bit table (emitted into `tables`).
fn set_test(set: &[bool; 256], tables: &mut String, name: &str) -> String {
    let members: Vec<usize> = (0..256).filter(|&b| set[b]).collect();
    match members.len() {
        0 => "0".into(),
        256 => "1".into(),
        1 => format!("(c == {})", members[0]),
        2 => format!("(c == {} || c == {})", members[0], members[1]),
        _ => {
            // (one run: a range check)
            if members.last().unwrap() - members[0] + 1 == members.len() {
                return format!("((uint32_t)(c - {}) <= {})", members[0], members.len() - 1);
            }
            let mut bytes = [0u8; 32];
            for &b in &members {
                bytes[b >> 3] |= 1 << (b & 7);
            }
            let list: Vec<String> = bytes.iter().map(|b| b.to_string()).collect();
            let _ = writeln!(tables, "static const uint8_t {name}[32] = {{{}}};", list.join(","));
            format!("(({name}[c >> 3] >> (c & 7)) & 1)")
        }
    }
}

/// The C function `fname` matching as lre_exec_backtrack does with bytecode `bc` (header first),
/// on 8-bit text; None for bytecode it doesn't know.
fn translate(bc: &[u8], fname: &str, unicode: bool, ops: &[(&str, usize)]) -> Option<String> {
    let ncap = bc[2] as u32;
    let code = &bc[8..];
    let size_at = |p: usize| -> Option<(usize, &str)> {
        let op = *code.get(p)? as usize;
        let (name, size) = *ops.get(op)?;
        let len = match name {
            "range" | "range_i" => 3 + u16_at(code, p + 1) as usize * 4,
            "range32" | "range32_i" => 3 + u16_at(code, p + 1) as usize * 8,
            "back_reference" | "back_reference_i" | "backward_back_reference" | "backward_back_reference_i" => 2 + code[p + 1] as usize,
            "greedy_char" => 13 + (u32_at(code, p + 9) & 0xffff) as usize,
            _ => size,
        };
        Some((len, name))
    };
    // (where control can go: labels)
    let mut targets = std::collections::BTreeSet::new();
    let mut p = 0;
    while p < code.len() {
        let (len, name) = size_at(p)?;
        let q = p + len;
        let rel = |at: usize| (q as i64 + u32_at(code, at) as i32 as i64) as usize;
        match name {
            "goto" | "split_goto_first" | "split_next_first" | "lookahead" | "negative_lookahead" => {
                targets.insert(rel(p + 1));
                targets.insert(q);
            }
            "loop" => {
                targets.insert(rel(p + 2));
            }
            "loop_split_goto_first" | "loop_split_next_first" | "loop_check_adv_split_goto_first" | "loop_check_adv_split_next_first" => {
                targets.insert(rel(p + 6));
                targets.insert(q);
            }
            "greedy_char" => {
                targets.insert(q);
            }
            _ => {}
        }
        p = q;
    }
    let mut tables = String::new();
    let mut body = String::new();
    let mut ntab = 0;
    let mut canon_used = false;
    let mut p = 0;
    while p < code.len() {
        let (len, name) = size_at(p)?;
        let q = p + len;
        if targets.contains(&p) {
            let _ = writeln!(body, "L{p}: __attribute__((unused));");
        }
        let rel = |at: usize| (q as i64 + u32_at(code, at) as i32 as i64) as usize;
        let mut tab = |set: &[bool; 256], tables: &mut String| {
            ntab += 1;
            set_test(set, tables, &format!("{fname}_t{ntab}"))
        };
        match name {
            "char" | "char_i" | "char32" | "char32_i" | "dot" | "any" | "space" | "not_space" | "range" | "range_i" | "range32" | "range32_i" => {
                let set = atom_set(code, p, name, unicode)?;
                let t = tab(&set, &mut tables);
                let _ = writeln!(body, "    if (cptr >= cbuf_end) goto no_match; c = *cptr++; if (!{t}) goto no_match;");
            }
            "line_start" => body.push_str("    if (cptr != cbuf) goto no_match;\n"),
            "line_start_m" => body.push_str("    if (cptr != cbuf && cptr[-1] != 10 && cptr[-1] != 13) goto no_match;\n"),
            "line_end" => body.push_str("    if (cptr != cbuf_end) goto no_match;\n"),
            "line_end_m" => body.push_str("    if (cptr != cbuf_end && *cptr != 10 && *cptr != 13) goto no_match;\n"),
            "goto" => {
                let _ = writeln!(body, "    goto L{};", rel(p + 1));
            }
            "split_goto_first" => {
                let _ = writeln!(body, "    TVJ_PUSH(&&L{q}, RE_EXEC_STATE_SPLIT); goto L{};", rel(p + 1));
            }
            "split_next_first" => {
                let _ = writeln!(body, "    TVJ_PUSH(&&L{}, RE_EXEC_STATE_SPLIT);", rel(p + 1));
            }
            "match" => body.push_str("    return 1;\n"),
            "lookahead" => {
                let _ = writeln!(body, "    TVJ_PUSH(&&L{}, RE_EXEC_STATE_LOOKAHEAD);", rel(p + 1));
            }
            "negative_lookahead" => {
                let _ = writeln!(body, "    TVJ_PUSH(&&L{}, RE_EXEC_STATE_NEGATIVE_LOOKAHEAD);", rel(p + 1));
            }
            "lookahead_match" => body.push_str(
                "    { StackElem *sp1, *sp_top = sp, *next_sp; REExecStateEnum type;\n      for (;;) { sp1 = sp; sp = bp; pc = sp[-3].ptr; cptr = sp[-2].ptr; type = sp[-1].bp.type; bp = s->stack_buf + sp[-1].bp.val; sp[-1].ptr = (void *)sp1; sp -= 3; if (type == RE_EXEC_STATE_LOOKAHEAD) break; }\n      if (sp != s->stack_buf) { sp1 = sp; while (sp1 < sp_top) { next_sp = (void *)sp1[2].ptr; sp1 += 3; while (sp1 < next_sp) *sp++ = *sp1++; } }\n      goto *pc; }\n",
            ),
            "negative_lookahead_match" => body.push_str(
                "    for (;;) { REExecStateEnum type; while (sp > bp) { capture[sp[-2].val] = sp[-1].ptr; sp -= 2; } pc = sp[-3].ptr; cptr = sp[-2].ptr; type = sp[-1].bp.type; bp = s->stack_buf + sp[-1].bp.val; sp -= 3; if (type == RE_EXEC_STATE_NEGATIVE_LOOKAHEAD) break; }\n    goto no_match;\n",
            ),
            "save_start" | "save_end" => {
                let idx = 2 * code[p + 1] as u32 + (name == "save_end") as u32;
                if code[p + 1] as u32 >= ncap {
                    return None;
                }
                let _ = writeln!(body, "    TVJ_SAVE({idx}, cptr);");
            }
            "save_reset" => {
                let (a, b) = (code[p + 1] as u32, code[p + 2] as u32);
                if b >= ncap {
                    return None;
                }
                let _ = write!(body, "    TVJ_GROW({});", 2 * (b.saturating_sub(a) + 1));
                for k in a..=b {
                    let _ = write!(body, " TVJ_SAVE({}, NULL); TVJ_SAVE({}, NULL);", 2 * k, 2 * k + 1);
                }
                body.push('\n');
            }
            "set_i32" => {
                let idx = 2 * ncap + code[p + 1] as u32;
                let _ = writeln!(body, "    TVJ_SAVE_CHECK({idx}, (uintptr_t){}u);", u32_at(code, p + 2));
            }
            "loop" => {
                let idx = 2 * ncap + code[p + 1] as u32;
                let _ = writeln!(body, "    {{ uint32_t v2 = (uint32_t)((uintptr_t)capture[{idx}] - 1); TVJ_SAVE_CHECK({idx}, (uintptr_t)v2); if (v2 != 0) goto L{}; }}", rel(p + 2));
            }
            "loop_split_goto_first" | "loop_split_next_first" | "loop_check_adv_split_goto_first" | "loop_check_adv_split_next_first" => {
                let idx = 2 * ncap + code[p + 1] as u32;
                let limit = u32_at(code, p + 2);
                let t = rel(p + 6);
                let adv = if name.contains("check_adv") { format!(" if (capture[{}] == cptr && v2 != {limit}u) goto no_match;", idx + 1) } else { String::new() };
                let split = if name.ends_with("next_first") { format!("TVJ_PUSH(&&L{t}, RE_EXEC_STATE_SPLIT);") } else { format!("TVJ_PUSH(&&L{q}, RE_EXEC_STATE_SPLIT); goto L{t};") };
                let _ = writeln!(body, "    {{ uint32_t v2 = (uint32_t)((uintptr_t)capture[{idx}] - 1); TVJ_SAVE_CHECK({idx}, (uintptr_t)v2); if (v2 > {limit}u) goto L{t};{adv} if (v2 != 0) {{ {split} }} }}");
            }
            "set_char_pos" => {
                let idx = 2 * ncap + code[p + 1] as u32;
                let _ = writeln!(body, "    TVJ_SAVE_CHECK({idx}, cptr);");
            }
            "check_advance" => {
                let idx = 2 * ncap + code[p + 1] as u32;
                let _ = writeln!(body, "    if (capture[{idx}] == cptr) goto no_match;");
            }
            "word_boundary" | "word_boundary_i" | "not_word_boundary" | "not_word_boundary_i" => {
                let set: [bool; 256] = std::array::from_fn(|b| is_word_byte(b as u8));
                let t = tab(&set, &mut tables);
                let b = name.starts_with("word") as u32;
                let _ = writeln!(body, "    {{ int v1 = 0, v2 = 0; if (cptr != cbuf) {{ c = cptr[-1]; v1 = {t}; }} if (cptr < cbuf_end) {{ c = *cptr; v2 = {t}; }} if (v1 ^ v2 ^ {b}) goto no_match; }}");
            }
            "back_reference" | "back_reference_i" | "backward_back_reference" | "backward_back_reference_i" => {
                let n = code[p + 1] as usize;
                let fold = name.ends_with("_i");
                if fold {
                    canon_used = true;
                }
                let cv = |x: &str| if fold { format!("{fname}_canon[{x}]") } else { x.to_string() };
                let groups: Vec<u32> = code[p + 2..p + 2 + n].iter().map(|&g| g as u32).collect();
                body.push_str("    {\n");
                let mut first = true;
                for g in groups {
                    if g >= ncap {
                        let _ = writeln!(body, "        goto no_match;");
                        continue;
                    }
                    let (a, z) = (2 * g, 2 * g + 1);
                    let test = if name.starts_with("backward") {
                        format!("const uint8_t *c1 = capture[{z}]; while (c1 > capture[{a}]) {{ if (cptr == cbuf) goto no_match; c1--; cptr--; if ({} != {}) goto no_match; }}", cv("*c1"), cv("*cptr"))
                    } else {
                        format!("const uint8_t *c1 = capture[{a}]; while (c1 < capture[{z}]) {{ if (cptr >= cbuf_end) goto no_match; if ({} != {}) goto no_match; c1++; cptr++; }}", cv("*c1"), cv("*cptr"))
                    };
                    let _ = writeln!(body, "        {}if (capture[{a}] && capture[{z}]) {{ {test} }}", if first { "" } else { "else " });
                    first = false;
                }
                body.push_str("    }\n");
            }
            "prev" => body.push_str("    if (cptr == cbuf) goto no_match; cptr--;\n"),
            "greedy_char" => {
                let (qmin, qmax, alen) = (u32_at(code, p + 1), u32_at(code, p + 5), u32_at(code, p + 9));
                let possessive = alen >> 31 != 0;
                let at = p + 13;
                let (_, aname) = size_at(at)?;
                let set = atom_set(code, at, aname, unicode)?;
                let t = tab(&set, &mut tables);
                let push = if possessive {
                    String::new()
                } else {
                    format!(" if (cptr != least_) {{ TVJ_GROW(4); sp[0].ptr = (uint8_t *)least_; sp[1].ptr = (uint8_t *)&&L{q}; sp[2].ptr = (uint8_t *)cptr; sp[3].bp.val = bp - s->stack_buf; sp[3].bp.type = RE_EXEC_STATE_GREEDY; sp += 4; bp = sp; }}")
                };
                let _ = writeln!(
                    body,
                    "    {{ uint32_t n_ = 0; const uint8_t *least_ = cptr; while (n_ < {qmax}u && cptr < cbuf_end) {{ c = *cptr; if (!{t}) break; cptr++; n_++; if (n_ == {qmin}u) least_ = cptr; }} if (n_ < {qmin}u) goto no_match;{push} }}"
                );
            }
            _ => return None,
        }
        p = q;
    }
    // (falling off the end: the engine never does; a match fails)
    body.push_str("    goto no_match;\n");
    let mut out = String::new();
    out.push_str(&tables);
    if canon_used {
        let list: Vec<String> = (0..256u32).map(|b| (unsafe { lre_canonicalize(b, unicode as c_int) } as u32).to_string()).collect();
        let _ = writeln!(out, "static const uint32_t {fname}_canon[256] = {{{}}};", list.join(","));
    }
    let _ = writeln!(
        out,
        "static intptr_t {fname}(REExecContext *s, uint8_t **capture, const uint8_t *cptr) {{\n    const uint8_t *const cbuf = s->cbuf, *const cbuf_end = s->cbuf_end;\n    StackElem *sp = s->stack_buf, *bp = s->stack_buf, *stack_end = s->stack_buf + s->stack_size;\n    void *pc = &&no_match; uint32_t c; (void)c; (void)pc;\n{body}no_match:\n    for (;;) {{\n        REExecStateEnum type;\n        if (bp == s->stack_buf) return 0;\n        while (sp > bp) {{ capture[sp[-2].val] = sp[-1].ptr; sp -= 2; }}\n        pc = sp[-3].ptr; cptr = sp[-2].ptr; type = sp[-1].bp.type; bp = s->stack_buf + sp[-1].bp.val; sp -= 3;\n        if (type == RE_EXEC_STATE_GREEDY) {{\n            const uint8_t *least = sp[-1].ptr; sp -= 1; cptr--;\n            if (cptr != least) {{ sp[0].ptr = (uint8_t *)least; sp[1].ptr = pc; sp[2].ptr = (uint8_t *)cptr; sp[3].bp.val = bp - s->stack_buf; sp[3].bp.type = RE_EXEC_STATE_GREEDY; sp += 4; bp = sp; }}\n            break;\n        }}\n        if (type != RE_EXEC_STATE_LOOKAHEAD) break;\n    }}\n    goto *pc;\n}}"
    );
    Some(out)
}

/// The matcher's state (the engine's own header).
pub const EXEC_H: &str = include_str!("../../../../vendor/quickjs-regexp/libregexp-exec.h");

/// The macros compiled matchers use (after libregexp-exec.h).
pub const PRELUDE: &str = "#define TVJ_GROW(n) if (__builtin_expect((stack_end - sp) < (n), 0)) { size_t o_ = sp - s->stack_buf, b_ = bp - s->stack_buf; if (lre_jit_stack_grow(s, o_ + (n))) return -1; stack_end = s->stack_buf + s->stack_size; sp = s->stack_buf + o_; bp = s->stack_buf + b_; }\n\
#define TVJ_SAVE(idx, v) { TVJ_GROW(2); sp[0].val = (idx); sp[1].ptr = capture[idx]; sp += 2; capture[idx] = (uint8_t *)(v); }\n\
#define TVJ_SAVE_CHECK(idx, v) { StackElem *s1_ = sp; for (;;) { if (s1_ > bp) { if (s1_[-2].val == (idx)) break; s1_ -= 2; } else { TVJ_GROW(2); sp[0].val = (idx); sp[1].ptr = capture[idx]; sp += 2; break; } } capture[idx] = (uint8_t *)(v); }\n\
#define TVJ_PUSH(lbl, type_) { TVJ_GROW(3); sp[0].ptr = (uint8_t *)(lbl); sp[1].ptr = (uint8_t *)cptr; sp[2].bp.val = bp - s->stack_buf; sp[2].bp.type = (type_); sp += 3; bp = sp; }\n";

/// The matchers for a program's pattern literals (source, flags): their C (functions and the
/// table tv_rejit_table takes, `tvg_rejits`), and how many there are.
pub fn matchers(patterns: &[(String, String)]) -> Option<(String, usize)> {
    let ops = opcodes();
    let mut out = String::new();
    let mut rows = Vec::new();
    for (k, (source, flags)) in patterns.iter().enumerate() {
        // (ASCII sources only: others the runtime reads as CESU-8 first)
        if !source.is_ascii() {
            continue;
        }
        let Some((bits, flag_text)) = parse_flags(flags) else { continue };
        let unicode = bits & (FLAG_UNICODE | FLAG_UNICODE_SETS) != 0;
        let Some(bc) = compile(source, bits) else { continue };
        let Some(f) = translate(&bc, &format!("tvj{k}"), unicode, &ops) else { continue };
        out.push_str(&f);
        out.push('\n');
        let lit = |b: &[u8]| -> String { format!("(const uint8_t *)\"{}\"", b.iter().map(|x| format!("\\x{x:02x}")).collect::<String>()) };
        let bc1 = match compile(source, bits | FLAG_STICKY).and_then(|b1| translate(&b1, &format!("tvj{k}s"), unicode, &ops).map(|f| (b1, f))) {
            Some((b1, f1)) => {
                out.push_str(&f1);
                out.push('\n');
                format!("{}, {}, tvj{k}s", lit(&b1), b1.len())
            }
            None => "NULL, 0, NULL".to_string(),
        };
        let key = format!("{flag_text}/{source}");
        rows.push(format!("    {{ {}, {}, {}, {}, tvj{k}, {bc1} }},", c_str(&key), key.len(), lit(&bc), bc.len()));
    }
    if rows.is_empty() {
        return None;
    }
    let _ = writeln!(out, "typedef struct {{ const char *key; size_t klen; const uint8_t *bc; size_t bclen; lre_jit_fn fn; const uint8_t *bc1; size_t bc1len; lre_jit_fn fn1; }} tvg_rejit;");
    let _ = writeln!(out, "static const tvg_rejit tvg_rejits[] = {{\n{}\n}};", rows.join("\n"));
    Some((out, rows.len()))
}

fn c_str(s: &str) -> String {
    let mut o = String::from("\"");
    for b in s.bytes() {
        match b {
            b'"' | b'\\' => {
                o.push('\\');
                o.push(b as char);
            }
            32..=126 => o.push(b as char),
            _ => {
                let _ = write!(o, "\\x{b:02x}\"\"");
            }
        }
    }
    o.push('"');
    o
}
