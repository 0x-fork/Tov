//! Generators. A `function* f(...)` compiles like an async function (see `asyncfn.rs`): its body
//! keeps its state in a frame `AF_f` (`frame::lower`), with the last yielded value in `F->yv`
//! (`F->yset` once there is one). `f_resume(F)` runs from `F->pc` to the next `yield` (false) or
//! to the end (true); `f(...)` makes the frame and returns a `tv_gen` over it, which `next`,
//! `for...of`, spreads and `Array.from` step through.
//!
//! A generator dropped before it finishes releases its parameters and last value; what its
//! locals held at the time isn't released.

use super::{Gen, Instance, Val};
use crate::ast::{self, ExprId};
use crate::types::*;
use std::fmt::Write;

impl<'c, 'a> Gen<'c, 'a> {
    pub(crate) fn is_gen_fn(&self, m: u32, item: u32) -> bool {
        matches!(&self.ast(m).items[item as usize].kind, ast::ItemKind::Function(f) if f.is_gen)
    }

    pub(super) fn emit_gen_function(&mut self, inst: Instance) {
        let ast::ItemKind::Function(f) = &self.ast(inst.m).items[inst.item as usize].kind else { return };
        let sig = self.c.sigs[&(inst.m, inst.item)].clone();
        let cname = inst.cname.clone();
        let params: Vec<(u32, TyId, bool)> = f.params.iter().zip(&sig.params).map(|(p, fp)| (p.span.start, self.c.types.subst(fp.ty, &inst.subst), false)).collect();
        let ret = self.c.types.subst(sig.ret, &inst.subst);
        let (elem, is_async) = match self.tget(ret) {
            Ty::Gen(e) => (e, false),
            Ty::AsyncGen(e) => (e, true),
            _ => {
                self.internal.push(format!("generator `{cname}` without a generator type"));
                return;
            }
        };
        self.pending_gen = Some(elem);
        // (an async generator's body awaits as an async function's does)
        if is_async {
            self.pending_async = Some((inst.m, inst.item));
        }
        let code = self.function_body(inst.m, inst.subst.clone(), &params, VOID, super::body::FnBodyKind::Block(f.body), None, None, None);
        let proto = self.fn_proto(&cname, &sig.params, sig.ret, &inst.subst, false);
        let ptys: Vec<TyId> = params.iter().map(|p| p.1).collect();
        self.gen_parts_of(&cname, &proto, &code, &ptys, elem, &[], "", is_async);
    }

    /// A generator method (`*items()`, `*[Symbol.iterator]()`): the frame also holds `self_`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_gen_method(&mut self, cname: &str, proto: &str, m: u32, subst: crate::hash::FxMap<u32, TyId>, params: &[(u32, TyId, bool)], ret: TyId, body: ast::StmtId, this: (u32, TyId, String)) {
        let Ty::Gen(elem) = self.tget(ret) else {
            self.internal.push(format!("generator method `{cname}` without a generator type"));
            return;
        };
        self.pending_gen = Some(elem);
        let code = self.function_body(m, subst, params, VOID, super::body::FnBodyKind::Block(body), None, Some((this.0, this.1)), None);
        let ptys: Vec<TyId> = params.iter().map(|p| p.1).collect();
        // (the generator holds its own reference to the object, released through the class id)
        self.rc_roots.insert(this.1);
        let extra = vec![(format!("{} *", this.2), "self_".to_string(), "tvg_obj_release(F->self_);".to_string())];
        self.gen_parts(cname, proto, &code, &ptys, elem, &extra, " F->self_ = self_; tvg_obj_retain(self_);");
    }

    /// A generator's frame, resume and drop functions, and its constructor `proto` (taking the
    /// parameters `p0`... and, `extra`, more frame fields: (C type, name, release code)).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn gen_parts(&mut self, cname: &str, proto: &str, code: &str, params: &[TyId], elem: TyId, extra: &[(String, String, String)], init: &str) {
        self.gen_parts_of(cname, proto, code, params, elem, extra, init, false);
    }

    /// As gen_parts; an async generator's frame starts `pc, yielded` (what `next()`'s task reads:
    /// whether a resume stopped at a `yield` or an `await`) and holds the frames of the async
    /// calls it awaits.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn gen_parts_of(&mut self, cname: &str, proto: &str, code: &str, params: &[TyId], elem: TyId, extra: &[(String, String, String)], init: &str, is_async: bool) {
        let children = if is_async { std::mem::take(&mut self.last_children) } else { Vec::new() };
        let frame = format!("AF_{cname}");
        let resume = format!("{cname}_resume");
        let drop = format!("{cname}_drop");
        let ect = self.ctype(elem);
        let _ = writeln!(self.typedefs, "typedef struct {frame} {frame};");
        let _ = writeln!(self.protos, "static bool {resume}({frame} *F);\nstatic void {drop}(void *F);");
        let mut fields: Vec<super::frame::Field> = params.iter().enumerate().map(|(i, ty)| super::frame::Field { ty: self.ctype(*ty), name: format!("p{i}") }).collect();
        fields.push(super::frame::Field { ty: ect.clone(), name: "yv".into() });
        fields.push(super::frame::Field { ty: "bool".into(), name: "yset".into() });
        fields.extend(extra.iter().map(|e| super::frame::Field { ty: e.0.trim_end().to_string(), name: e.1.clone() }));
        let lowered = match super::frame::lower(code, &fields) {
            Ok(l) => l,
            Err(msg) => {
                self.internal.push(format!("generator `{cname}`: {msg}"));
                return;
            }
        };
        let mut def = format!("struct {frame} {{\n    void *pc;\n{}    {ect} yv;\n    bool yset;\n", if is_async { "    bool yielded;\n" } else { "" });
        for (ct, name, _) in extra {
            let _ = writeln!(def, "    {ct} {name};");
        }
        for (i, ty) in params.iter().enumerate() {
            let ct = self.ctype(*ty);
            let _ = writeln!(def, "    {ct} p{i};");
        }
        for fl in &lowered.fields {
            let _ = writeln!(def, "    {} {};", fl.ty, fl.name);
        }
        let mut deps = Vec::new();
        if !children.is_empty() {
            def.push_str("    union {\n");
            for (member, ty) in &children {
                let _ = writeln!(def, "        {ty} {member};");
                deps.push(ty.clone());
            }
            def.push_str("    } u;\n");
        }
        def.push_str("};\n");
        self.frame_defs.push(super::asyncfn::FrameDef { name: frame.clone(), deps, text: def });
        let dispatch = if lowered.body.contains("&&aw") { "    if (F->pc) goto *F->pc;\n" } else { "    (void)F;\n" };
        let _ = writeln!(self.funcs, "static bool {resume}({frame} *F) {{\n{}{dispatch}{}    return true;\n}}\n", lowered.prologue, lowered.body);
        let mut release = String::new();
        for (i, ty) in params.iter().enumerate() {
            if self.is_rc(*ty) {
                let r = self.release_code(*ty, &format!("F->p{i}"));
                let _ = write!(release, " {r};");
            }
        }
        if self.is_rc(elem) {
            let r = self.release_code(elem, "F->yv");
            let _ = write!(release, " if (F->yset) {{ {r}; }}");
        }
        for (_, _, rel) in extra {
            let _ = write!(release, " {rel}");
        }
        let _ = writeln!(self.funcs, "static void {drop}(void *p) {{\n    {frame} *F = p;{release}\n    free(F);\n}}\n");
        let mut set = init.to_string();
        for (i, ty) in params.iter().enumerate() {
            let r = if self.is_rc(*ty) { format!(" {};", self.retain_code(*ty, &format!("p{i}"))) } else { String::new() };
            let _ = write!(set, " F->p{i} = p{i};{r}");
        }
        let _ = writeln!(self.funcs, "static {proto} {{\n    {frame} *F = calloc(1, sizeof({frame}));\n    if (!F) tv_trap(\"out of memory\", NULL);{set}\n    return tv_gen_new(F, (bool (*)(void *)){resume}, {drop}, &F->yv);\n}}\n");
    }

    /// `yield x` / `yield* xs` in a generator's body: the value (each of xs's) into `F->yv`, then
    /// suspend until the next `next`. Its own value is `undefined`.
    pub(crate) fn yield_expr(&mut self, v: Option<ExprId>, delegate: bool, span: crate::source::Span) -> Val {
        let Some(elem) = self.b().gen_elem else {
            self.unsupported(span, "`yield` here");
            return Val::plain("0", UNDEFINED);
        };
        match (v, delegate) {
            (Some(x), true) => {
                let xs = self.expr(x);
                let xs = self.own(xs);
                let ty = xs.ty;
                let ct = self.ctype(ty);
                let code = self.consume(xs);
                let it = self.fresh("dg");
                self.line(format!("{ct} {it} = {code};"));
                match self.tget(ty) {
                    Ty::Gen(inner) => {
                        let ict = self.ctype(inner);
                        self.open("for (;;) {");
                        self.line(format!("if (tv_gen_next({it})) break;"));
                        let item = Val::plain(format!("(*({ict} *){it}->yv)"), inner);
                        self.store_yield(item, elem);
                        self.close("}");
                    }
                    Ty::Array(inner) => {
                        let (ict, i) = (self.ctype(inner), self.fresh("di"));
                        self.line(format!("tv_int {i} = 0;"));
                        self.open(&format!("for (; {i} < tv_arr_len({it}); {i}++) {{"));
                        let item = Val::plain(format!("((({ict} *)tv_arr_data({it}))[{i}])"), inner);
                        self.store_yield(item, elem);
                        self.close("}");
                    }
                    _ => self.unsupported(span, "`yield*` of this"),
                }
                let rel = self.release_code(ty, &it);
                self.line(format!("{rel};"));
            }
            (Some(x), false) => {
                let val = self.expr(x);
                self.store_yield(val, elem);
            }
            (None, _) => {
                let d = self.default_value(elem);
                self.store_yield(Val::plain(d, elem), elem);
            }
        }
        Val::plain("0", UNDEFINED)
    }

    /// The value (borrowed or owned) as the frame's last yielded one, then the suspension.
    fn store_yield(&mut self, v: Val, elem: TyId) {
        let v = self.coerce(v, elem);
        let v = self.own(v);
        let code = self.consume(v);
        // (`yv`/`yset` by name: the frame fields, kept in locals meanwhile as parameters are)
        let rel = if self.is_rc(elem) { format!("if (yset) {{ {}; }} ", self.release_code(elem, "yv")) } else { String::new() };
        let mark = if self.b().async_fn.is_some() { " F->yielded = true;" } else { "" };
        self.line(format!("{rel}yv = {code}; yset = true;{mark}"));
        let n = self.fresh("");
        self.line(format!("F->pc = &&aw{n}; TV_SUSPEND;"));
        self.line(format!("aw{n}:;"));
    }

    /// `g.next()` of an async generator of `elem`s: a promise of its result, from a task that
    /// resumes the generator (and, where it awaits, waits with it) until it yields or ends.
    pub(crate) fn agen_next(&mut self, g: Val, elem: TyId, ty: TyId) -> Val {
        let Ty::Promise(res, _) = self.tget(ty) else { return Val::plain("NULL", ty) };
        let key = res.0;
        let f = match self.agen_helpers.get(&key) {
            Some(f) => f.clone(),
            None => {
                let f = format!("tvg_agen_next{}", self.agen_helpers.len());
                self.agen_helpers.insert(key, f.clone());
                let _ = writeln!(self.protos, "static tv_promise *{f}(tv_gen *g);");
                // (the result, `{ value, done: false } | { value: undefined, done: true }`)
                let Ty::Union(ms) = self.tget(res) else { return Val::plain("NULL", ty) };
                let ms = self.c.types.tys(ms).to_vec();
                let field = |g2: &Self, m: TyId, n: &str| g2.c.types.fields_in_order(m).into_iter().find(|f| g2.sym(f.name) == n);
                let (Some(more), Some(end)) = (
                    ms.iter().position(|&m| field(self, m, "done").is_some_and(|f| matches!(*self.c.types.get(f.ty), Ty::BoolLit(false)))),
                    ms.iter().position(|&m| field(self, m, "done").is_some_and(|f| matches!(*self.c.types.get(f.ty), Ty::BoolLit(true)))),
                ) else {
                    return Val::plain("NULL", ty);
                };
                let (mt, et) = (ms[more], ms[end]);
                let ect = self.ctype(elem);
                let lit = self.rec_lit(et, ".f_value = 0, .f_done = true");
                let ended = self.u_make(res, end, Some(&lit));
                let lit = self.rec_lit(mt, ".f_value = v_, .f_done = false");
                let given = self.u_make(res, more, Some(&lit));
                let keep = if self.is_rc(elem) { format!(" {};", self.retain_code(elem, "v_")) } else { String::new() };
                let rct = self.ctype(res);
                let desc = self.desc(res);
                let _ = writeln!(
                    self.funcs,
                    "static bool {f}_step(tv_task *t) {{\n    tv_gen *g = *(tv_gen **)tv_task_frame(t);\n    struct {{ void *pc; bool yielded; }} *gf = g->F;\n    if (!g->done) {{\n        gf->yielded = false;\n        if (g->resume(g->F)) g->done = true;\n        else if (!gf->yielded) return false;\n    }}\n    if (tvg_err) {{ g->done = true; tv_promise_reject(t->promise, tvg_err); tvg_err = NULL; tv_gen_release(g); return true; }}\n    {rct} r_;\n    if (g->done) r_ = {ended};\n    else {{ {ect} v_ = *({ect} *)g->yv;{keep} r_ = {given}; }}\n    tv_promise_resolve_move(t->promise, &r_);\n    tv_gen_release(g);\n    return true;\n}}\nstatic tv_promise *{f}(tv_gen *g) {{\n    tv_task *t = tv_task_new(sizeof(tv_gen *), {f}_step, {desc});\n    *(tv_gen **)tv_task_frame(t) = g; tv_gen_retain(g);\n    return tv_task_spawn(t);\n}}\n"
                );
                f
            }
        };
        self.tmp(ty, &format!("{f}({})", g.code), true)
    }

    /// `g.next()`: `{ value, done }`.
    pub(crate) fn gen_next(&mut self, g: Val, elem: TyId, ty: TyId) -> Val {
        // (`{ value, done: false } | { value: undefined, done: true }`: the member it is)
        if let Ty::Union(ms) = self.tget(ty) {
            let ms = self.c.types.tys(ms).to_vec();
            let field = |g2: &Self, m: TyId, n: &str| g2.c.types.fields_in_order(m).into_iter().find(|f| g2.sym(f.name) == n);
            let (Some(more), Some(end)) = (
                ms.iter().position(|&m| field(self, m, "done").is_some_and(|f| matches!(*self.c.types.get(f.ty), Ty::BoolLit(false)))),
                ms.iter().position(|&m| field(self, m, "done").is_some_and(|f| matches!(*self.c.types.get(f.ty), Ty::BoolLit(true)))),
            ) else {
                return Val::plain("0", ty);
            };
            let (mt, et) = (ms[more], ms[end]);
            let vt = field(self, mt, "value").map(|f| f.ty).unwrap_or(elem);
            let ect = self.ctype(elem);
            let d = self.fresh("gd");
            self.line(format!("bool {d} = tv_gen_next({});", g.code));
            let ct = self.ctype(ty);
            let res = self.fresh("gr");
            self.line(format!("{ct} {res};"));
            self.open_block(&format!("if ({d}) {{"));
            let lit = self.rec_lit(et, ".f_value = 0, .f_done = true");
            let made = self.u_make(ty, end, Some(&lit));
            self.line(format!("{res} = {made};"));
            self.mid_block("} else {");
            let present = self.coerce(Val::plain(format!("(*({ect} *)({}->yv))", g.code), elem), vt);
            let present = self.own(present);
            let pc = self.consume(present);
            let lit = self.rec_lit(mt, &format!(".f_value = {pc}, .f_done = false"));
            let made = self.u_make(ty, more, Some(&lit));
            self.line(format!("{res} = {made};"));
            self.close_block("}");
            let owned = self.is_rc(ty);
            if owned {
                self.b().temps.last_mut().unwrap().push((res.clone(), ty));
            }
            return Val { code: res, ty, owned };
        }
        let Ty::Record(fs) = self.tget(ty) else { return Val::plain("0", ty) };
        let fields = self.c.types.fields(fs).to_vec();
        let (Some(vf), Some(df)) = (fields.iter().find(|f| self.sym(f.name) == "value"), fields.iter().find(|f| self.sym(f.name) == "done")) else { return Val::plain("0", ty) };
        let (vt, ect) = (vf.ty, self.ctype(elem));
        let d = self.fresh("gd");
        self.line(format!("bool {d} = tv_gen_next({});", g.code));
        let present = self.coerce(Val::plain(format!("(*({ect} *)({}->yv))", g.code), elem), vt);
        let present = self.own(present);
        let pc = self.consume(present);
        let absent = self.coerce(Val::plain("0", UNDEFINED), vt);
        let vct = self.ctype(vt);
        let val = self.fresh("gv");
        self.line(format!("{vct} {val};"));
        self.line(format!("if ({d}) {val} = {}; else {val} = {pc};", absent.code));
        let (vn, dn) = (self.sym(vf.name).to_string(), self.sym(df.name).to_string());
        let lit = self.rec_lit(ty, &format!(".f_{vn} = {val}, .f_{dn} = {d}"));
        let r = self.tmp(ty, &lit, true);
        r
    }
}
