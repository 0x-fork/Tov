//! Interface values: a fat pointer `{ data, itab }`.
//!
//! `data` is the object for a class instance, a reference-counted box holding a copy for a record
//! (values stay values), or a cell holding another interface value. The itab starts with
//! retain/release/print functions, then has one thunk per interface member (sorted by name): a
//! getter returning an owned value, or, for function-typed members, a direct call.

use super::{Gen, Val};
use crate::check::class::ClassMemberRef;
use crate::types::*;
use std::fmt::Write;

impl<'c, 'a> Gen<'c, 'a> {
    /// Members of an interface type, sorted by name (the itab order).
    pub(crate) fn iface_members(&mut self, t: TyId) -> Vec<Field> {
        let Ty::Interface(i, args) = self.tget(t) else { return Vec::new() };
        let args = self.c.types.tys(args).to_vec();
        let mut fs = self.c.iface_fields_inst(i, &args);
        fs.sort_by_key(|f| f.name);
        fs
    }

    /// The itab struct type for an interface type (defined on first use).
    pub(crate) fn itab_type(&mut self, t: TyId) -> String {
        let name = format!("IT{}", t.0);
        if self.helpers_done.insert((t, 102)) {
            let mut fields = String::new();
            for (k, f) in self.iface_members(t).iter().enumerate() {
                match self.tget(f.ty) {
                    Ty::Func(ps, ret, _) => {
                        let ps = self.c.types.params(ps).to_vec();
                        let rct = if ret == VOID { "void".to_string() } else { self.ctype(ret) };
                        let mut args = vec!["void *".to_string()];
                        for p in &ps {
                            let ct = self.ctype(p.ty);
                            args.push(if p.inout { format!("{ct} *") } else { ct });
                        }
                        let _ = write!(fields, " {rct} (*m{k})({});", args.join(", "));
                    }
                    _ => {
                        let ct = self.ctype(f.ty);
                        let _ = write!(fields, " {ct} (*m{k})(void *);");
                    }
                }
            }
            // (then, per member, its address in the value for assigning through the interface;
            // NULL for functions)
            for k in 0..self.iface_members(t).len() {
                let _ = write!(fields, " void *(*f{k})(void *);");
            }
            let _ = writeln!(self.typedefs, "typedef struct {name} {{ tvg_itab h;{fields} }} {name};");
        }
        name
    }

    /// Converts a record, class instance or interface value to an interface value.
    pub(crate) fn wrap_iface(&mut self, v: Val, to: TyId) -> Option<Val> {
        let src = v.ty;
        let m = self.cur_m();
        let itab = self.itab_for(src, to, m)?;
        let data = match self.tget(src) {
            Ty::Class(..) => {
                let code = self.consume(v);
                format!("(void *)({code})")
            }
            // (an object: itself, so identity holds through the interface)
            Ty::Record(_) if self.is_obj(src) => {
                let code = self.consume(v);
                format!("(void *)({code})")
            }
            Ty::Record(_) | Ty::Interface(..) => {
                let bn = self.iface_box(src);
                let code = self.consume(v);
                let b = self.fresh("ib");
                self.line(format!("{bn} *{b} = tvg_alloc_small(sizeof({bn})); {b}->rc = 1; {b}->v = {code};"));
                format!("(void *){b}")
            }
            _ => return None,
        };
        Some(self.tmp(to, &format!("((tv_iface){{ {data}, (const tvg_itab *)&{itab} }})"), true))
    }

    /// A heap box holding a value inside an interface value.
    fn iface_box(&mut self, src: TyId) -> String {
        let bn = format!("IB{}", src.0);
        if self.helpers_done.insert((src, 103)) {
            let ct = self.ctype(src);
            let _ = writeln!(self.typedefs, "typedef struct {bn} {bn};");
            let _ = writeln!(self.class_structs, "struct {bn} {{ int64_t rc; {ct} v; }};");
        }
        bn
    }

    /// The itab for values of type `src` seen as interface `to`.
    fn itab_for(&mut self, src: TyId, to: TyId, m: u32) -> Option<String> {
        if let Some(n) = self.itabs.get(&(src, to)) {
            return Some(n.clone());
        }
        let it = self.itab_type(to);
        let name = format!("itab_{}_{}", src.0, to.0);
        self.itabs.insert((src, to), name.clone());
        let members = self.iface_members(to);
        let is_class = matches!(self.tget(src), Ty::Class(..));
        let is_obj = self.is_obj(src);
        let sct = self.ctype(src);
        // How thunks reach the value: the object, or the boxed copy.
        let (self_decl, value) = if is_class || is_obj { (format!("{sct} x = self;"), "x".to_string()) } else {
            let bn = self.iface_box(src);
            (format!("{bn} *x = self;"), "x->v".to_string())
        };
        let mut out = String::new();
        // Retain / release / print.
        let (rt, rl) = if is_class {
            ("tvg_obj_retain".to_string(), self.class_release_code(src, "p").replace("(p)", ""))
        } else if is_obj {
            let rel = self.release_code(src, "o");
            let _ = writeln!(out, "static void {name}_rt(void *p) {{ (({sct})p)->rc++; }}");
            let _ = writeln!(out, "static void {name}_rl(void *p) {{ {sct} o = p; {rel}; }}");
            (format!("{name}_rt"), format!("{name}_rl"))
        } else {
            let bn = self.iface_box(src);
            let rel = self.release_code(src, "b->v");
            let _ = writeln!(out, "static void {name}_rt(void *p) {{ (({bn} *)p)->rc++; }}");
            let _ = writeln!(out, "static void {name}_rl(void *p) {{ {bn} *b = p; if (--b->rc == 0) {{ {rel}; tvg_free_small(b, sizeof(*b)); }} }}");
            (format!("{name}_rt"), format!("{name}_rl"))
        };
        let (rt, rl) = if is_class {
            let _ = writeln!(out, "static void {name}_rt(void *p) {{ {rt}(p); }}");
            let rlc = self.release_code(src, "p");
            let _ = writeln!(out, "static void {name}_rl(void *p) {{ {rlc}; }}");
            (format!("{name}_rt"), format!("{name}_rl"))
        } else {
            (rt, rl)
        };
        let (decl, place) = if is_class || is_obj { (format!("{sct} o = p; "), "o".to_string()) } else { (String::new(), format!("(({} *)p)->v", self.iface_box(src))) };
        let ins = self.inspect_code(src, &place, "depth");
        let ts = self.string_code(src, &place);
        let _ = writeln!(out, "static void {name}_in(tv_sb *sb, void *p, int depth) {{ (void)depth; {decl}{ins}; }}");
        let _ = writeln!(out, "static void {name}_ts(tv_sb *sb, void *p) {{ {decl}{ts}; }}");
        // Member thunks.
        let mut entries = Vec::new();
        for (k, f) in members.iter().enumerate() {
            let tname = format!("{name}_m{k}");
            match self.tget(f.ty) {
                Ty::Func(ps, ret, _) => {
                    let ps = self.c.types.params(ps).to_vec();
                    let rct = if ret == VOID { "void".to_string() } else { self.ctype(ret) };
                    let mut decl_params = vec!["void *self".to_string()];
                    for (i, p) in ps.iter().enumerate() {
                        let ct = self.ctype(p.ty);
                        decl_params.push(if p.inout { format!("{ct} *a{i}") } else { format!("{ct} a{i}") });
                    }
                    self.begin_scratch(m);
                    self.line(&self_decl);
                    let args: Vec<Val> = ps.iter().enumerate().map(|(i, p)| Val::plain(format!("a{i}"), p.ty)).collect();
                    let result = match (is_class, self.tget(src)) {
                        (true, _) => {
                            let Some(ClassMemberRef::Method(mm)) = self.c.class_member(src, f.name) else {
                                self.end_scratch();
                                return None;
                            };
                            let mut argv = Vec::new();
                            for (a, mp) in args.iter().zip(&mm.params) {
                                let cv = self.coerce(a.clone(), mp.ty);
                                argv.push(cv.code);
                            }
                            let mret = self.c.method_ret(&mm);
                            let call = self.method_call_code("x", src, f.name, false, argv);
                            if mret == VOID {
                                self.line(format!("{call};"));
                                Val::plain("0", VOID)
                            } else {
                                self.tmp(mret, &call, true)
                            }
                        }
                        (false, Ty::Record(fs)) => {
                            let Some(rf) = self.c.types.fields(fs).iter().find(|x| x.name == f.name).copied() else {
                                self.end_scratch();
                                return None;
                            };
                            let Ty::Func(fps, fret, _) = self.tget(rf.ty) else {
                                self.end_scratch();
                                return None;
                            };
                            let fps = self.c.types.params(fps).to_vec();
                            let fv = self.fresh("fn");
                            let fc = self.fld(src, &value, self.sym(f.name));
                            self.line(format!("tv_fn {fv} = {fc};"));
                            let mut all = vec![format!("{fv}.env")];
                            for (a, fp) in args.iter().zip(&fps) {
                                let cv = self.coerce(a.clone(), fp.ty);
                                all.push(cv.code);
                            }
                            let cast = {
                                let rct = if fret == VOID { "void".to_string() } else { self.ctype(fret) };
                                let mut parts = vec!["tv_env *".to_string()];
                                for p in &fps {
                                    let ct = self.ctype(p.ty);
                                    parts.push(if p.inout { format!("{ct} *") } else { ct });
                                }
                                format!("{rct} (*)({})", parts.join(", "))
                            };
                            let call = format!("(({cast}){fv}.fn)({})", all.join(", "));
                            if fret == VOID {
                                self.line(format!("{call};"));
                                Val::plain("0", VOID)
                            } else {
                                self.tmp(fret, &call, true)
                            }
                        }
                        (false, Ty::Interface(..)) => {
                            let sm = self.iface_members(src);
                            let Some(sk) = sm.iter().position(|x| x.name == f.name) else {
                                self.end_scratch();
                                return None;
                            };
                            let sit = self.itab_type(src);
                            let Ty::Func(sps, sret, _) = self.tget(sm[sk].ty) else {
                                self.end_scratch();
                                return None;
                            };
                            let sps = self.c.types.params(sps).to_vec();
                            let mut all = vec![format!("({value}).p")];
                            for (a, sp) in args.iter().zip(&sps) {
                                let cv = self.coerce(a.clone(), sp.ty);
                                all.push(cv.code);
                            }
                            let call = format!("((const {sit} *)({value}).t)->m{sk}({})", all.join(", "));
                            if sret == VOID {
                                self.line(format!("{call};"));
                                Val::plain("0", VOID)
                            } else {
                                self.tmp(sret, &call, true)
                            }
                        }
                        _ => {
                            self.end_scratch();
                            return None;
                        }
                    };
                    self.scratch_return(result, ret);
                    let body = self.end_scratch();
                    let _ = writeln!(out, "static {rct} {tname}({}) {{\n{body}}}", decl_params.join(", "));
                }
                _ => {
                    // A field (or getter): returns an owned value of the member's type.
                    let ct = self.ctype(f.ty);
                    self.begin_scratch(m);
                    self.line(&self_decl);
                    let v = match self.tget(src) {
                        Ty::Class(..) => {
                            let mt = self.class_member_ty(src, f.name)?;
                            let span = crate::source::Span::new(self.c.modules[m as usize].file, 0, 0);
                            self.class_field_read(Val::plain("x", src), f.name, mt, span)
                        }
                        Ty::Record(fs) => match self.c.types.fields(fs).iter().find(|x| x.name == f.name).copied() {
                            Some(rf) => Val::plain(self.fld(src, &value, self.sym(f.name)), rf.ty),
                            None => self.coerce(Val::plain("0", UNDEFINED), f.ty),
                        },
                        Ty::Interface(..) => {
                            let sm = self.iface_members(src);
                            let sk = sm.iter().position(|x| x.name == f.name)?;
                            let sit = self.itab_type(src);
                            let sty = sm[sk].ty;
                            self.tmp(sty, &format!("((const {sit} *)({value}).t)->m{sk}(({value}).p)"), true)
                        }
                        _ => {
                            self.end_scratch();
                            return None;
                        }
                    };
                    self.scratch_return(v, f.ty);
                    let body = self.end_scratch();
                    let _ = writeln!(out, "static {ct} {tname}(void *self) {{\n{body}}}");
                }
            }
            entries.push(tname);
        }
        // Field addresses.
        for (k, f) in members.iter().enumerate() {
            if matches!(self.tget(f.ty), Ty::Func(..)) {
                entries.push("NULL".into());
                continue;
            }
            let ct = self.ctype(f.ty);
            let addr = match self.tget(src) {
                Ty::Class(..) => match self.class_field(src, f.name) {
                    Some(cf) if !cf.weak && self.ctype(cf.ty) == ct => Some(format!("&{}", self.field_lv_of("x", src, &cf))),
                    _ => None,
                },
                Ty::Record(fs) => match self.c.types.fields(fs).iter().find(|x| x.name == f.name).copied() {
                    Some(rf) if self.ctype(rf.ty) == ct => Some(format!("&{}", self.fld(src, &value, self.sym(f.name)))),
                    _ => None,
                },
                Ty::Interface(..) => {
                    let sm = self.iface_members(src);
                    match sm.iter().position(|x| x.name == f.name) {
                        Some(sk) if self.ctype(sm[sk].ty) == ct => {
                            let sit = self.itab_type(src);
                            Some(format!("((const {sit} *)({value}).t)->f{sk}(({value}).p)"))
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            // (a getter, or a field of another representation: can't be assigned this way)
            let body = match addr {
                Some(a) => format!("{self_decl} return (void *){a};"),
                None => {
                    let msg = format!("setting `{}` of an object made without it isn't supported yet (give it the field when it's made)", self.sym(f.name));
                    format!("(void)self; tv_trap({}, NULL);", super::c_string(msg.as_bytes()))
                }
            };
            let tname = format!("{name}_f{k}");
            let _ = writeln!(out, "static void *{tname}(void *self) {{ {body} }}");
            entries.push(tname);
        }
        let mut init = format!("{{ {rt}, {rl}, TVG_INS({name}_in), {name}_ts }}");
        for e in &entries {
            let _ = write!(init, ", {e}");
        }
        let _ = writeln!(out, "static const {it} {name} = {{ {init} }};");
        self.helpers_after.push(out);
        let _ = writeln!(self.protos, "static const {it} {name};");
        Some(name)
    }

    fn class_member_ty(&mut self, cls: TyId, name: crate::intern::Sym) -> Option<TyId> {
        let mr = self.c.class_member(cls, name)?;
        Some(self.c.member_type(&mr))
    }

    /// `v.name` on an interface value: calls the member's getter thunk (owned result).
    pub(crate) fn iface_field(&mut self, v: Val, name: crate::intern::Sym, ty: TyId) -> Option<Val> {
        let members = self.iface_members(v.ty);
        let k = members.iter().position(|f| f.name == name)?;
        if matches!(self.tget(members[k].ty), Ty::Func(..)) {
            return None;
        }
        let it = self.itab_type(v.ty);
        let ft = members[k].ty;
        let r = self.tmp(ft, &format!("((const {it} *)({0}).t)->m{k}(({0}).p)", v.code), true);
        // (narrowed, `if (n.ch !== undefined) n.ch`: the member of its union, as for a record's field)
        if self.c.assignable_pub(ty, ft) && ty != ft && !self.c.assignable_pub(ft, ty) {
            return Some(self.project(r, ty));
        }
        Some(self.coerce(r, ty))
    }

    /// `v.name` on an interface value as an lvalue, through the member's address thunk.
    pub(crate) fn iface_field_lv(&mut self, v: &Val, name: crate::intern::Sym) -> Option<(String, TyId)> {
        let members = self.iface_members(v.ty);
        let k = members.iter().position(|f| f.name == name)?;
        if matches!(self.tget(members[k].ty), Ty::Func(..)) {
            return None;
        }
        let (it, ct) = (self.itab_type(v.ty), self.ctype(members[k].ty));
        Some((format!("(*({ct} *)((const {it} *)({0}).t)->f{k}(({0}).p))", v.code), members[k].ty))
    }

    /// `v.name(args)` on an interface value, for a function-typed member.
    pub(crate) fn iface_call(&mut self, v: &Val, name: crate::intern::Sym, argv: Vec<String>) -> Option<(String, TyId)> {
        let members = self.iface_members(v.ty);
        let k = members.iter().position(|f| f.name == name)?;
        let Ty::Func(_, ret, _) = self.tget(members[k].ty) else { return None };
        let it = self.itab_type(v.ty);
        let mut all = vec![format!("({}).p", v.code)];
        all.extend(argv);
        Some((format!("((const {it} *)({}).t)->m{k}({})", v.code, all.join(", ")), ret))
    }
}
