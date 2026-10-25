//! `any`: an `unknown`'s value (a `tv_dyn`) that every operation is allowed on. What the runtime
//! knows of (strings, numbers, JSON's arrays and objects) it handles (runtime `tv_any_*`); for
//! the program's own types, every type put into an `unknown` (`dyn_types`) gets its cases in
//! switches made after everything else (`any_dispatch`): reading and setting a property,
//! indexing, calling, calling a method. A value of the wrong type where a type is wanted traps.

use super::{Gen, Val};
use crate::ast::{Arg, BinOp, ExprKind};
use crate::check::class::ClassMemberRef;
use crate::types::*;
use std::fmt::Write;

const UNDEF: &str = "((tv_dyn){&tv_type_undefined, NULL})";

impl<'c, 'a> Gen<'c, 'a> {
    /// Code works on an `any`: the dispatch functions are declared (and made at the end).
    pub(crate) fn any_used(&mut self) {
        if !self.any_tables {
            self.any_tables = true;
            let _ = writeln!(
                self.protos,
                "static tv_dyn tvg_any_get(tv_dyn d, tv_str k, const char *loc);\n\
                 static tv_dyn tvg_any_at(tv_dyn d, tv_dyn k, const char *loc);\n\
                 static void tvg_any_set(tv_dyn d, tv_str k, tv_dyn v, const char *loc);\n\
                 static void tvg_any_set_at(tv_dyn d, tv_dyn k, tv_dyn v, const char *loc);\n\
                 static tv_dyn tvg_any_call(tv_dyn f, int n, const tv_dyn *a, const char *loc);\n\
                 static tv_dyn tvg_any_invoke(tv_dyn d, tv_str k, int n, const tv_dyn *a, const char *loc);\n\
                 static tv_int tvg_any_len(tv_dyn d);"
            );
        }
    }

    /// Where the statement being made is, for a trap (`"loc"` inside a dispatch helper, which
    /// is given its caller's).
    pub(crate) fn any_loc(&mut self) -> String {
        match self.stmt_span {
            Some(s) => self.loc(s),
            None => "\"?\"".to_string(),
        }
    }

    /// `v` (an `any`) as a `tv_dyn` C expression, evaluated once.
    fn any_hold(&mut self, v: Val) -> Val {
        if v.owned || v.code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return v;
        }
        let d = self.fresh("ad");
        self.line(format!("tv_dyn {d} = {};", v.code));
        Val { code: d, ..v }
    }

    /// `obj.name` (`obj?.name`) of an `any`.
    pub(crate) fn any_member(&mut self, obj: Val, name: &str, optional: bool, ty: TyId, loc: &str) -> Val {
        self.any_used();
        let obj = self.any_hold(obj);
        let key = self.lit(name.as_bytes());
        let get = format!("tvg_any_get({}, {key}, {loc})", obj.code);
        let code = if optional { format!("(({o}.t == &tv_type_undefined || {o}.t == &tv_type_null) ? {UNDEF} : {get})", o = obj.code) } else { get };
        let r = self.tmp(ANY, &code, true);
        self.coerce(r, ty)
    }

    /// `obj[index]` of an `any`.
    pub(crate) fn any_index(&mut self, obj: Val, index: Val, optional: bool, ty: TyId, loc: &str) -> Val {
        self.any_used();
        let obj = self.any_hold(obj);
        let k = self.coerce(index, ANY);
        let get = format!("tvg_any_at({}, {}, {loc})", obj.code, k.code);
        let code = if optional { format!("(({o}.t == &tv_type_undefined || {o}.t == &tv_type_null) ? {UNDEF} : {get})", o = obj.code) } else { get };
        let r = self.tmp(ANY, &code, true);
        self.coerce(r, ty)
    }

    /// The arguments of a call on an `any`, as a C array of `tv_dyn`s (and its length); a
    /// spread array's elements each.
    fn any_argv(&mut self, args: &[Arg]) -> (String, String) {
        let m = self.cur_m();
        let spread = args.iter().any(|a| matches!(self.ast(m).expr(a.expr).kind, ExprKind::Spread(_)));
        if !spread {
            let mut vals = Vec::new();
            for a in args {
                let v = self.expr(a.expr);
                let d = self.coerce(v, ANY);
                let d = self.own(d);
                vals.push(d.code);
            }
            let argv = if vals.is_empty() { "NULL".to_string() } else { format!("(tv_dyn[]){{{}}}", vals.join(", ")) };
            return (argv, vals.len().to_string());
        }
        // (`f(...xs)`: an array of them all)
        let anys = self.c.types.array(ANY);
        let arr = self.tmp(anys, "tv_arr_fresh()", true);
        for a in args {
            match self.ast(m).expr(a.expr).kind {
                ExprKind::Spread(x) => {
                    let v = self.expr(x);
                    let v = self.coerce(v, anys);
                    let i = self.fresh("i");
                    self.open(&format!("for (tv_int {i} = 0; {i} < tv_arr_len({}); {i}++) {{", v.code));
                    self.line(format!("tv_dyn e_ = ((tv_dyn *)tv_arr_data({}))[{i}]; tv_dyn_retain(e_); tv_arr_push(&{}, &tv_type_dyn, &e_);", v.code, arr.code));
                    self.close("}");
                }
                _ => {
                    let v = self.expr(a.expr);
                    let d = self.coerce(v, ANY);
                    let c = self.consume(d);
                    self.line(format!("{{ tv_dyn e_ = {c}; tv_arr_push(&{}, &tv_type_dyn, &e_); }}", arr.code));
                }
            }
        }
        (format!("((const tv_dyn *)tv_arr_data({}))", arr.code), format!("(int)tv_arr_len({})", arr.code))
    }

    /// A call on an `any`: `f(...)`, or `obj.name(...)` (`target` is `f` or `obj`).
    pub(crate) fn any_call(&mut self, target: Val, method: Option<&str>, args: &[Arg], ty: TyId, loc: &str) -> Val {
        self.any_used();
        let target = self.any_hold(target);
        let (argv, n) = self.any_argv(args);
        let call = match method {
            Some(name) => {
                let key = self.lit(name.as_bytes());
                format!("tvg_any_invoke({}, {key}, {n}, {argv}, {loc})", target.code)
            }
            None => format!("tvg_any_call({}, {n}, {argv}, {loc})", target.code),
        };
        let r = self.tmp(ANY, &call, true);
        self.coerce(r, ty)
    }

    /// `obj.name = v` / `obj[k] op= v` of an `any` (`value`: the new value, as an `any`).
    pub(crate) fn any_store(&mut self, obj: Val, key: AnyKey, value: Val, loc: &str) {
        self.any_used();
        let obj = self.any_hold(obj);
        let v = self.coerce(value, ANY);
        let v = self.own(v);
        let vc = self.consume(v);
        match key {
            AnyKey::Name(name) => {
                let k = self.lit(name.as_bytes());
                self.line(format!("tvg_any_set({}, {k}, {vc}, {loc});", obj.code));
            }
            AnyKey::Value(k) => {
                let k = self.coerce(k, ANY);
                self.line(format!("tvg_any_set_at({}, {}, {vc}, {loc});", obj.code, k.code));
            }
        }
    }

    /// `o.name op= v`, `o[k] op= v` of an `any` (the assignment's value).
    pub(crate) fn any_assign(&mut self, op: crate::ast::AssignOp, target: crate::ast::ExprId, value: crate::ast::ExprId, span: crate::source::Span) -> Val {
        let m = self.cur_m();
        let loc = self.loc(span);
        let (obj, index, name) = match self.ast(m).expr(target).kind {
            ExprKind::Member { obj, name, .. } => (obj, None, Some(self.sym(name).to_string())),
            ExprKind::Index { obj, index, .. } => (obj, Some(index), None),
            _ => unreachable!("an `any` store"),
        };
        let o = self.expr(obj);
        let o = self.any_hold(o);
        let k = match (name, index) {
            (Some(n), _) => AnyKey::Name(n),
            (None, Some(i)) => {
                let iv = self.expr(i);
                let iv = self.coerce(iv, ANY);
                let iv = self.own(iv);
                AnyKey::Value(iv)
            }
            _ => unreachable!(),
        };
        let ty = self.ty(target);
        let read = |g: &mut Self, k: &AnyKey| -> Val {
            match k {
                AnyKey::Name(n) => {
                    let n = n.clone();
                    g.any_member(o.clone(), &n, false, ANY, &loc)
                }
                AnyKey::Value(iv) => g.any_index(o.clone(), Val { owned: false, ..iv.clone() }, false, ANY, &loc),
            }
        };
        let nv = match op {
            crate::ast::AssignOp::Assign => {
                let v = self.expr(value);
                let v = self.own(v);
                let d = self.coerce(Val { owned: false, ..v.clone() }, ANY);
                let kc = k.borrowed();
                self.any_store(o.clone(), kc, d, &loc);
                return v;
            }
            crate::ast::AssignOp::Op(b @ (BinOp::Nullish | BinOp::Or | BinOp::And)) => {
                // (`o.x ??= v`: set when it's nullish (`||=` falsy, `&&=` truthy))
                let cur = read(self, &k);
                let test = match b {
                    BinOp::Nullish => format!("({c}.t == &tv_type_undefined || {c}.t == &tv_type_null)", c = cur.code),
                    BinOp::Or => format!("!tv_dyn_truthy({})", cur.code),
                    _ => format!("tv_dyn_truthy({})", cur.code),
                };
                self.open(&format!("if ({test}) {{"));
                self.push_temps();
                let v = self.expr(value);
                let d = self.coerce(v, ANY);
                let kc = k.borrowed();
                self.any_store(o.clone(), kc, d, &loc);
                self.pop_temps();
                self.close("}");
                let r = read(self, &k);
                return self.coerce(r, ty);
            }
            crate::ast::AssignOp::Op(b) => {
                let cur = read(self, &k);
                let v = self.expr(value);
                let rt = self.c.any_binary(b, ANY, v.ty);
                self.any_binop(b, cur, v, rt)
            }
        };
        let nv = self.own(nv);
        let d = self.coerce(Val { owned: false, ..nv.clone() }, ANY);
        let kc = k.borrowed();
        self.any_store(o, kc, d, &loc);
        self.coerce(nv, ty)
    }

    /// `x++` / `--o.n` of an `any` (or a place holding one): its number, one more or less.
    pub(crate) fn any_update(&mut self, target: crate::ast::ExprId, inc: bool, prefix: bool, span: crate::source::Span) -> Val {
        let m = self.cur_m();
        let loc = self.loc(span);
        let delta = if inc { "+ 1" } else { "- 1" };
        match self.ast(m).expr(target).kind {
            ExprKind::Member { obj, name, .. } => {
                let o = self.expr(obj);
                let o = self.any_hold(o);
                let n = self.sym(name).to_string();
                let cur = self.any_member(o.clone(), &n, false, ANY, &loc);
                let old = self.fresh("o");
                self.line(format!("double {old} = tv_any_num({});", cur.code));
                let nv = Val::plain(format!("({old} {delta})"), F64);
                let d = self.coerce(nv, ANY);
                self.any_store(o, AnyKey::Name(n), d, &loc);
                Val::plain(if prefix { format!("({old} {delta})") } else { old }, F64)
            }
            ExprKind::Index { obj, index, .. } => {
                let o = self.expr(obj);
                let o = self.any_hold(o);
                let iv = self.expr(index);
                let iv = self.coerce(iv, ANY);
                let iv = self.own(iv);
                let cur = self.any_index(o.clone(), Val { owned: false, ..iv.clone() }, false, ANY, &loc);
                let old = self.fresh("o");
                self.line(format!("double {old} = tv_any_num({});", cur.code));
                let nv = Val::plain(format!("({old} {delta})"), F64);
                let d = self.coerce(nv, ANY);
                self.any_store(o, AnyKey::Value(iv), d, &loc);
                Val::plain(if prefix { format!("({old} {delta})") } else { old }, F64)
            }
            _ => {
                // (a variable holding an `any`)
                let p = self.place(target);
                let lv = self.place_lvalue(&p);
                let old = self.fresh("o");
                self.line(format!("double {old} = tv_any_num({lv});"));
                self.line(format!("tv_dyn_release({lv});"));
                self.line(format!("{lv} = tv_dyn_box(&tv_type_f64, &(double){{{old} {delta}}});"));
                Val::plain(if prefix { format!("({old} {delta})") } else { old }, F64)
            }
        }
    }

    /// `a op b` with an `any` operand (as the checker's `any_binary`).
    pub(crate) fn any_binop(&mut self, op: BinOp, a: Val, b: Val, ty: TyId) -> Val {
        use BinOp::*;
        let num = |g: &mut Self, v: Val| -> String {
            if g.tget(v.ty) == Ty::Unknown {
                format!("tv_any_num({})", v.code)
            } else if g.c.types.is_num_like(v.ty) {
                format!("((double)({}))", v.code)
            } else {
                let d = g.coerce(v, ANY);
                format!("tv_any_num({})", d.code)
            }
        };
        match op {
            Add if ty == STR => {
                let (x, y) = (self.any_string(a), self.any_string(b));
                self.tmp(STR, &format!("tv_str_concat({}, {})", x.code, y.code), true)
            }
            Add => {
                let (x, y) = (self.coerce(a, ANY), self.coerce(b, ANY));
                let r = self.tmp(ANY, &format!("tv_any_add({}, {})", x.code, y.code), true);
                self.coerce(r, ty)
            }
            Sub | Mul | Div | Rem | Pow => {
                let (x, y) = (num(self, a), num(self, b));
                let code = match op {
                    Sub => format!("({x} - {y})"),
                    Mul => format!("({x} * {y})"),
                    Div => format!("({x} / {y})"),
                    Rem => format!("fmod({x}, {y})"),
                    _ => format!("pow({x}, {y})"),
                };
                self.coerce(Val::plain(code, F64), ty)
            }
            BitAnd | BitOr | BitXor | Shl | Shr | UShr => {
                let (x, y) = (num(self, a), num(self, b));
                let (x, y) = (format!("tv_f64_to_i32({x})"), format!("tv_f64_to_i32({y})"));
                let code = match op {
                    BitAnd => format!("(tv_int)({x} & {y})"),
                    BitOr => format!("(tv_int)({x} | {y})"),
                    BitXor => format!("(tv_int)({x} ^ {y})"),
                    Shl => format!("(tv_int)(int32_t)((uint32_t){x} << ({y} & 31))"),
                    Shr => format!("(tv_int)({x} >> ({y} & 31))"),
                    _ => format!("(tv_int)((uint32_t){x} >> ({y} & 31))"),
                };
                self.coerce(Val::plain(code, INT), ty)
            }
            Lt | Gt | Le | Ge => {
                let (x, y) = (self.coerce(a, ANY), self.coerce(b, ANY));
                let code = match op {
                    Lt => format!("tv_any_lt({}, {}, false)", x.code, y.code),
                    Le => format!("tv_any_lt({}, {}, true)", x.code, y.code),
                    Gt => format!("tv_any_lt({}, {}, false)", y.code, x.code),
                    _ => format!("tv_any_lt({}, {}, true)", y.code, x.code),
                };
                Val::plain(code, BOOL)
            }
            _ => unreachable!("any_binop of {op:?}"),
        }
    }

    /// `String(v)` of a value, for concatenation.
    fn any_string(&mut self, v: Val) -> Val {
        if self.c.types.is_stringish(v.ty) {
            return self.coerce(v, STR);
        }
        let d = self.coerce(v, ANY);
        self.tmp(STR, &format!("tv_any_str({})", d.code), true)
    }

    /// The `any` `v` as a `to`: checked (it traps if it holds something else). Numbers convert
    /// between widths; an object or array of other types (JSON's, an `any[]`) converts field by
    /// field, element by element.
    pub(crate) fn any_to(&mut self, v: Val, to: TyId, loc: &str) -> Val {
        let v = self.any_hold(v);
        let d = v.code.clone();
        match self.tget(to) {
            Ty::Unknown => return Val { ty: to, ..v },
            _ if self.c.types.is_numeric(to) => {
                self.line(format!("if (__builtin_expect(!tv_dyn_is_num({d}.t), 0)) tv_any_type_trap(\"a number\", {d}, {loc});"));
                let n = Val::plain(format!("tv_dyn_number({d})"), F64);
                return self.coerce(n, to);
            }
            Ty::Str => {
                self.line(format!("if (__builtin_expect({d}.t != &tv_type_str, 0)) tv_any_type_trap(\"a string\", {d}, {loc});"));
                return self.unbox_dyn(v, to);
            }
            Ty::Bool => {
                // (`undefined` as `false`: an option that isn't given)
                return Val::plain(format!("({d}.t == &tv_type_bool ? *(bool *)tv_dyn_data({d}) : ({d}.t == &tv_type_undefined || {d}.t == &tv_type_null) ? false : (tv_any_type_trap(\"a boolean\", {d}, {loc}), false))"), BOOL);
            }
            Ty::Undefined | Ty::Void => return Val::plain("0", to),
            _ => {}
        }
        // (records, arrays and unions holding them: a helper converting what can be)
        if self.any_converts(to) {
            let f = self.any_converter(to);
            return self.tmp(to, &format!("{f}({d}, {loc})"), true);
        }
        let ok = self.dyn_is(&d, to);
        let shown = super::c_string(self.c.show(to).as_bytes());
        self.line(format!("if (__builtin_expect(!{ok}, 0)) tv_any_type_trap({shown}, {d}, {loc});"));
        self.unbox_dyn(v, to)
    }

    /// Whether a value of type `to` can be made of an `any` holding another type (a record of
    /// an object's keys, an array of another's elements).
    fn any_converts(&mut self, to: TyId) -> bool {
        match self.tget(to) {
            Ty::Record(_) | Ty::Array(_) => true,
            Ty::Union(ms) => self.c.types.tys(ms).to_vec().iter().any(|&m| matches!(self.tget(m), Ty::Record(_) | Ty::Array(_))),
            _ => false,
        }
    }

    /// `static CT tvg_any_to_N(tv_dyn d, const char *loc)`: `d` as a `to` (owned).
    fn any_converter(&mut self, to: TyId) -> String {
        if let Some(n) = self.any_convs.get(&to) {
            return n.clone();
        }
        let name = format!("tvg_any_to_{}", to.0);
        self.any_convs.insert(to, name.clone());
        self.any_used();
        let ct = self.ctype(to);
        let _ = writeln!(self.protos, "static {ct} {name}(tv_dyn d, const char *loc);");
        let m = self.main_module();
        self.begin_scratch(m);
        let members: Vec<TyId> = match self.tget(to) {
            Ty::Union(ms) => self.c.types.tys(ms).to_vec(),
            _ => vec![to],
        };
        // (what it holds is one of them: as itself)
        for &mt in &members {
            let test = self.dyn_is("d", mt);
            self.open(&format!("if ({test}) {{"));
            self.push_temps();
            let u = self.unbox_dyn(Val::plain("d", UNKNOWN), mt);
            self.scratch_return(u, to);
            self.pop_temps();
            self.close("}");
        }
        for &mt in &members {
            match self.tget(mt) {
                // (an object: a record of its keys)
                Ty::Record(fs) if self.tuple_of(mt).is_none() => {
                    let fs = self.c.types.fields(fs).to_vec();
                    self.open("if (!(tv_dyn_is_num(d.t) || d.t == &tv_type_str || d.t == &tv_type_bool || d.t == &tv_type_undefined || d.t == &tv_type_null || tvg_any_len(d) >= 0)) {");
                    self.push_temps();
                    let mut parts = Vec::new();
                    for f in &fs {
                        let fname = self.sym(f.name).to_string();
                        let k = self.lit(fname.as_bytes());
                        let fv = self.tmp(ANY, &format!("tvg_any_get(d, {k}, loc)"), true);
                        let cv = self.any_to(fv, f.ty, "loc");
                        let code = self.consume(cv);
                        parts.push(format!(".f_{fname} = {code}"));
                    }
                    let lit = self.rec_lit(mt, &parts.join(", "));
                    let r = self.tmp(mt, &lit, true);
                    self.scratch_return(r, to);
                    self.pop_temps();
                    self.close("}");
                }
                // (an array: one of its elements)
                Ty::Array(e) => {
                    self.open("if (tvg_any_len(d) >= 0 && d.t != &tv_type_str) {");
                    self.push_temps();
                    let ed = self.desc(e);
                    let ect = self.ctype(e);
                    let r = self.tmp(mt, &format!("tv_arr_with_capacity({ed}, tvg_any_len(d))"), true);
                    self.open("for (tv_int i = 0, n = tvg_any_len(d); i < n; i++) {");
                    self.push_temps();
                    let ev = self.tmp(ANY, "({ tv_dyn k_ = tv_dyn_box(&tv_type_int, &i); tv_dyn r_ = tvg_any_at(d, k_, loc); tv_dyn_release(k_); r_; })", true);
                    let cv = self.any_to(ev, e, "loc");
                    let code = self.consume(cv);
                    self.line(format!("TVG_PUSH({ect}, &{}, {ed}, {code});", r.code));
                    self.pop_temps();
                    self.close("}");
                    self.scratch_return(r, to);
                    self.pop_temps();
                    self.close("}");
                }
                _ => {}
            }
        }
        let shown = super::c_string(self.c.show(to).as_bytes());
        self.line(format!("tv_any_type_trap({shown}, d, loc);"));
        let body = self.end_scratch();
        self.helpers_after.push(format!("static {ct} {name}(tv_dyn d, const char *loc) {{\n{body}}}\n"));
        name
    }

    /// The module code made outside any function belongs to (for its names).
    fn main_module(&self) -> u32 {
        self.bodies.last().map(|b| b.m).unwrap_or(0)
    }

    /// The dispatch functions over every type put into an `unknown` (made last: boxing a field
    /// can add types, so until there are no new ones). Made again while that changes; true when
    /// made (what they call may need making).
    pub(crate) fn any_dispatch(&mut self) -> bool {
        if !self.any_tables || self.any_covered == Some(self.dyn_types.len()) {
            return false;
        }
        let mut get = String::new();
        let mut at = String::new();
        let mut set = String::new();
        let mut set_at = String::new();
        let mut call = String::new();
        let mut invoke = String::new();
        let mut len = String::new();
        let mut i = 0;
        let m = self.main_module();
        while i < self.dyn_types.len() {
            let t = self.dyn_types[i];
            i += 1;
            let d = self.desc(t);
            let ct = self.ctype(t);
            let p = format!("(*({ct} *)tv_dyn_data(d))");
            match self.tget(t) {
                Ty::Record(fs) => {
                    let fs = self.c.types.fields(fs).to_vec();
                    let tuple = self.tuple_of(t).is_some();
                    self.begin_scratch(m);
                    for f in &fs {
                        let fname = self.sym(f.name).to_string();
                        let k = self.lit(fname.as_bytes());
                        self.open(&format!("if (tv_str_eq(k, {k})) {{"));
                        self.push_temps();
                        let fv = Val { code: self.fld(t, &p, &fname), ty: f.ty, owned: false };
                        let b = self.box_dyn(fv);
                        self.scratch_return(b, UNKNOWN);
                        self.pop_temps();
                        self.close("}");
                    }
                    if tuple {
                        let l = self.lit(b"length");
                        self.line(format!("if (tv_str_eq(k, {l})) return tv_dyn_box(&tv_type_int, &(tv_int){{{}}});", fs.len()));
                    }
                    let body = self.end_scratch();
                    let _ = write!(get, "    if (d.t == {d}) {{\n{body}        return {UNDEF};\n    }}\n");
                    // (setting a field: the new value as its type)
                    self.begin_scratch(m);
                    for f in &fs {
                        let fname = self.sym(f.name).to_string();
                        let k = self.lit(fname.as_bytes());
                        self.open(&format!("if (tv_str_eq(k, {k})) {{"));
                        self.push_temps();
                        let nv = self.any_to(Val { code: "v".into(), ty: ANY, owned: false }, f.ty, "loc");
                        let nv = self.own(nv);
                        let code = self.consume(nv);
                        let place = self.fld(t, &p, &fname);
                        let rel = self.release_code(f.ty, &place);
                        if !rel.is_empty() {
                            self.line(format!("{rel};"));
                        }
                        self.line(format!("{place} = {code};"));
                        self.line("tv_dyn_release(v);");
                        self.release_all_temps();
                        self.line("return;");
                        self.pop_temps();
                        self.close("}");
                    }
                    let body = self.end_scratch();
                    let _ = write!(set, "    if (d.t == {d}) {{\n{body}        tv_dyn_release(v);\n        tv_trap(\"TypeError: can't add a property to an object of a fixed type\", loc);\n    }}\n");
                }
                Ty::Class(..) => {
                    let fields = self.c.class_as_fields(t);
                    self.begin_scratch(m);
                    let mut setb = String::new();
                    for f in &fields {
                        let fname = self.sym(f.name).to_string();
                        let k = self.lit(fname.as_bytes());
                        match self.c.class_member(t, f.name) {
                            Some(ClassMemberRef::Field(cf)) => {
                                self.open(&format!("if (tv_str_eq(k, {k})) {{"));
                                self.push_temps();
                                let v = self.class_field_read(Val::plain(p.clone(), t), f.name, cf.ty, crate::source::Span::new(crate::source::FileId(0), 0, 0));
                                let b = self.box_dyn(v);
                                self.scratch_return(b, UNKNOWN);
                                self.pop_temps();
                                self.close("}");
                                // (and set)
                                self.begin_scratch(m);
                                self.open(&format!("if (tv_str_eq(k, {k})) {{"));
                                self.push_temps();
                                let nv = self.any_to(Val { code: "v".into(), ty: ANY, owned: false }, cf.ty, "loc");
                                let nv = self.own(nv);
                                let code = self.consume(nv);
                                let lv = self.class_field_lv_pub(&p, t, &cf);
                                let rel = self.release_code(cf.ty, &lv);
                                if !rel.is_empty() {
                                    self.line(format!("{rel};"));
                                }
                                self.line(format!("{lv} = {code};"));
                                self.line("tv_dyn_release(v);");
                                self.release_all_temps();
                                self.line("return;");
                                self.pop_temps();
                                self.close("}");
                                setb.push_str(&self.end_scratch());
                            }
                            Some(ClassMemberRef::Getter(_)) => {
                                self.open(&format!("if (tv_str_eq(k, {k})) {{"));
                                self.push_temps();
                                let v = self.class_field_read(Val::plain(p.clone(), t), f.name, f.ty, crate::source::Span::new(crate::source::FileId(0), 0, 0));
                                let b = self.box_dyn(v);
                                self.scratch_return(b, UNKNOWN);
                                self.pop_temps();
                                self.close("}");
                            }
                            _ => {}
                        }
                    }
                    let body = self.end_scratch();
                    let _ = write!(get, "    if (d.t == {d}) {{\n{body}        return {UNDEF};\n    }}\n");
                    let _ = write!(set, "    if (d.t == {d}) {{\n{setb}        tv_dyn_release(v);\n        tv_trap(\"TypeError: can't add a property to an object of a fixed type\", loc);\n    }}\n");
                    // (its methods)
                    self.begin_scratch(m);
                    for f in &fields {
                        let Some(ClassMemberRef::Method(mm)) = self.c.class_member(t, f.name) else { continue };
                        if mm.params.iter().any(|q| q.inout) || !mm.tparams.is_empty() {
                            continue;
                        }
                        let fname = self.sym(f.name).to_string();
                        let k = self.lit(fname.as_bytes());
                        self.open(&format!("if (tv_str_eq(k, {k})) {{"));
                        self.push_temps();
                        let mut argv = Vec::new();
                        for (j, q) in mm.params.iter().enumerate() {
                            let a = Val::plain(format!("({j} < n ? a[{j}] : {UNDEF})"), ANY);
                            let cv = self.any_to(a, q.ty, "loc");
                            argv.push(cv.code);
                        }
                        let ret = self.c.method_ret(&mm);
                        let code = self.method_call_code(&p, t, f.name, false, argv);
                        let r = if ret == VOID {
                            self.line(format!("{code};"));
                            Val::plain("0", UNDEFINED)
                        } else {
                            self.tmp(ret, &code, true)
                        };
                        let b = self.box_dyn(r);
                        self.scratch_return(b, UNKNOWN);
                        self.pop_temps();
                        self.close("}");
                    }
                    let body = self.end_scratch();
                    let _ = write!(invoke, "    if (d.t == {d}) {{\n{body}    }}\n");
                }
                Ty::Array(e) => {
                    let ect = self.ctype(e);
                    let ed = self.desc(e);
                    let _ = writeln!(len, "    if (d.t == {d}) return tv_arr_len({p});");
                    self.begin_scratch(m);
                    self.open(&format!("if (i >= 0 && i < tv_arr_len({p})) {{"));
                    self.push_temps();
                    let ev = Val { code: format!("(({ect} *)tv_arr_data({p}))[i]"), ty: e, owned: false };
                    let b = self.box_dyn(ev);
                    self.scratch_return(b, UNKNOWN);
                    self.pop_temps();
                    self.close("}");
                    let body = self.end_scratch();
                    let _ = write!(at, "        if (d.t == {d}) {{\n{body}            return {UNDEF};\n        }}\n");
                    let _ = writeln!(get, "    if (d.t == {d}) {{ if (tv_str_eq(k, {})) return tv_dyn_box(&tv_type_int, &(tv_int){{tv_arr_len({p})}}); return {UNDEF}; }}", self.lit(b"length"));
                    // (a[i] = v: replaced, or appended at the end)
                    self.begin_scratch(m);
                    self.push_temps();
                    let nv = self.any_to(Val { code: "v".into(), ty: ANY, owned: false }, e, "loc");
                    let nv = self.own(nv);
                    let code = self.consume(nv);
                    self.line(format!("{ect} x_ = {code};"));
                    self.line("tv_dyn_release(v);");
                    self.release_all_temps();
                    self.pop_temps();
                    let rel = self.release_code(e, &format!("(({ect} *)tv_arr_data({p}))[i]"));
                    self.line(format!("if (i < tv_arr_len({p})) {{ tv_arr_make_unique(&{p}, {ed}, tv_arr_len({p})); {}; (({ect} *)tv_arr_data({p}))[i] = x_; }}", if rel.is_empty() { "(void)0".to_string() } else { rel }));
                    self.line(format!("else if (i == tv_arr_len({p})) TVG_PUSH({ect}, &{p}, {ed}, x_);"));
                    self.line("else tv_trap(\"TypeError: a hole in an array of a fixed type\", loc);");
                    self.line("return;");
                    let body = self.end_scratch();
                    let _ = write!(set_at, "        if (d.t == {d}) {{\n{body}        }}\n");
                    // (push, pop, join, indexOf, includes, slice)
                    self.begin_scratch(m);
                    let l = self.lit(b"push");
                    self.open(&format!("if (tv_str_eq(k, {l})) {{"));
                    self.open("for (int j = 0; j < n; j++) {");
                    self.push_temps();
                    let nv = self.any_to(Val { code: "a[j]".into(), ty: ANY, owned: false }, e, "loc");
                    let nv = self.own(nv);
                    let code = self.consume(nv);
                    self.line(format!("TVG_PUSH({ect}, &{p}, {ed}, {code});"));
                    self.pop_temps();
                    self.close("}");
                    self.line(format!("return tv_dyn_box(&tv_type_int, &(tv_int){{tv_arr_len({p})}});"));
                    self.close("}");
                    let l = self.lit(b"pop");
                    self.open(&format!("if (tv_str_eq(k, {l})) {{"));
                    self.line(format!("{ect} x_;"));
                    self.line(format!("if (!tv_arr_pop(&{p}, {ed}, &x_)) return {UNDEF};"));
                    self.push_temps();
                    let b = self.box_dyn(Val { code: "x_".into(), ty: e, owned: true });
                    self.scratch_return(b, UNKNOWN);
                    self.pop_temps();
                    self.close("}");
                    let body = self.end_scratch();
                    let (jn, io, inc, sl) = (self.lit(b"join"), self.lit(b"indexOf"), self.lit(b"includes"), self.lit(b"slice"));
                    let _ = write!(
                        invoke,
                        "    if (d.t == {d}) {{\n{body}        if (tv_str_eq(k, {jn})) {{ tv_str sep = n > 0 && a[0].t != &tv_type_undefined ? tv_any_str(a[0]) : tv_str_from(\",\", 1); tv_str r = tv_arr_join({p}, {ed}, sep); tv_str_release(sep); return tv_dyn_box(&tv_type_str, &r); }}\n\
                         \x20       if (tv_str_eq(k, {io}) || tv_str_eq(k, {inc})) {{ tv_int r = -1; tv_dyn x = n > 0 ? a[0] : {UNDEF}; for (tv_int i = 0; i < tv_arr_len({p}); i++) if (tv_any_eq_raw({ed}, &(({ect} *)tv_arr_data({p}))[i], x)) {{ r = i; break; }} if (tv_str_eq(k, {inc})) {{ bool b = r >= 0; return tv_dyn_box(&tv_type_bool, &b); }} return tv_dyn_box(&tv_type_int, &r); }}\n\
                         \x20       if (tv_str_eq(k, {sl})) {{ tv_int s = n > 0 && a[0].t != &tv_type_undefined ? (tv_int)tv_any_num(a[0]) : 0, e = n > 1 && a[1].t != &tv_type_undefined ? (tv_int)tv_any_num(a[1]) : 0; tv_arr r = tv_arr_slice({p}, {ed}, s, e, n > 0, n > 1 && a[1].t != &tv_type_undefined); return tv_dyn_box({d}, &r); }}\n    }}\n"
                    );
                }
                Ty::Map(k, v) if self.c.types.is_string(k) => {
                    let (kd, vd) = (self.desc(k), self.desc(v));
                    let vct = self.ctype(v);
                    self.begin_scratch(m);
                    self.line(format!("{vct} *vp_ = tv_map_get({p}, {kd}, {vd}, &k);"));
                    self.line(format!("if (!vp_) return {UNDEF};"));
                    self.push_temps();
                    let b = self.box_dyn(Val { code: "(*vp_)".into(), ty: v, owned: false });
                    self.scratch_return(b, UNKNOWN);
                    self.pop_temps();
                    let body = self.end_scratch();
                    let _ = write!(get, "    if (d.t == {d}) {{\n{body}    }}\n");
                    self.begin_scratch(m);
                    self.push_temps();
                    let nv = self.any_to(Val { code: "v".into(), ty: ANY, owned: false }, v, "loc");
                    let nv = self.own(nv);
                    let code = self.consume(nv);
                    self.line(format!("{vct} x_ = {code};"));
                    self.line("tv_dyn_release(v);");
                    self.release_all_temps();
                    self.pop_temps();
                    self.line(format!("tv_str_retain(k); tv_map_set(&{p}, {kd}, {vd}, &k, &x_);"));
                    self.line("return;");
                    let body = self.end_scratch();
                    let _ = write!(set, "    if (d.t == {d}) {{\n{body}    }}\n");
                }
                Ty::Func(ps, ret, throws) => {
                    let ps = self.c.types.params(ps).to_vec();
                    if ps.iter().any(|q| q.inout) {
                        continue;
                    }
                    self.begin_scratch(m);
                    self.push_temps();
                    let mut argv = vec!["f_.env".to_string()];
                    for (j, q) in ps.iter().enumerate() {
                        let a = Val::plain(format!("({j} < n ? a[{j}] : {UNDEF})"), ANY);
                        let cv = self.any_to(a, q.ty, "loc");
                        argv.push(cv.code);
                    }
                    let cast = self.fn_ptr_type_pub(&ps, ret);
                    let code = format!("(({cast})f_.fn)({})", argv.join(", "));
                    let r = if ret == VOID {
                        self.line(format!("{code};"));
                        Val::plain("0", UNDEFINED)
                    } else {
                        self.tmp(ret, &code, true)
                    };
                    if throws != NEVER {
                        self.line(format!("if (tvg_err) {{ return {UNDEF}; }}"));
                    }
                    let b = self.box_dyn(r);
                    self.scratch_return(b, UNKNOWN);
                    self.pop_temps();
                    let body = self.end_scratch();
                    let _ = write!(call, "    if (f.t == {d}) {{\n        tv_fn f_ = *(tv_fn *)tv_dyn_data(f);\n{body}    }}\n");
                }
                _ => {}
            }
        }
        let fix = "tv_trap(\"TypeError: this value is not a function\", loc);";
        let code = format!(
            "static tv_dyn tvg_any_get(tv_dyn d, tv_str k, const char *loc) {{\n    tv_dyn r;\n    if (tv_any_get_rt(d, k, &r, loc)) return r;\n{get}    return {UNDEF};\n}}\n\
             static tv_dyn tvg_any_at(tv_dyn d, tv_dyn key, const char *loc) {{\n    tv_int i;\n    if (tv_any_int_key(key, &i)) {{\n        tv_dyn r;\n        if (tv_any_at_rt(d, i, &r)) return r;\n{at}    }}\n    tv_str k = tv_any_key(key);\n    tv_dyn r = tvg_any_get(d, k, loc);\n    tv_str_release(k);\n    return r;\n}}\n\
             static void tvg_any_set(tv_dyn d, tv_str k, tv_dyn v, const char *loc) {{\n    if (tv_any_set_rt(d, k, v, loc)) return;\n{set}    tv_dyn_release(v);\n}}\n\
             static void tvg_any_set_at(tv_dyn d, tv_dyn key, tv_dyn v, const char *loc) {{\n    tv_int i;\n    if (tv_any_int_key(key, &i)) {{\n        if (tv_any_set_at_rt(d, i, v)) return;\n{set_at}    }}\n    tv_str k = tv_any_key(key);\n    tvg_any_set(d, k, v, loc);\n    tv_str_release(k);\n}}\n\
             static tv_dyn tvg_any_call(tv_dyn f, int n, const tv_dyn *a, const char *loc) {{\n    (void)n; (void)a;\n{call}    {fix}\n}}\n\
             static tv_dyn tvg_any_invoke(tv_dyn d, tv_str k, int n, const tv_dyn *a, const char *loc) {{\n    tv_dyn r;\n    if (tv_any_invoke_rt(d, k, n, a, &r, loc)) return r;\n{invoke}    tv_dyn f = tvg_any_get(d, k, loc);\n    r = tvg_any_call(f, n, a, loc);\n    tv_dyn_release(f);\n    return r;\n}}\n\
             static tv_int tvg_any_len(tv_dyn d) {{\n    tv_int r = tv_any_len_rt(d);\n    if (r >= 0) return r;\n{len}    return -1;\n}}\n"
        );
        self.any_code = code;
        self.any_covered = Some(self.dyn_types.len());
        true
    }
}

/// The key of a store to an `any`'s property.
pub(crate) enum AnyKey {
    Name(String),
    Value(Val),
}

impl AnyKey {
    /// The same key, not given away.
    fn borrowed(&self) -> AnyKey {
        match self {
            AnyKey::Name(n) => AnyKey::Name(n.clone()),
            AnyKey::Value(v) => AnyKey::Value(Val { owned: false, ..v.clone() }),
        }
    }
}
