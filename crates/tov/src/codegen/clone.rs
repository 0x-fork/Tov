//! `structuredClone(v)`: a deep copy, by a function generated per type. Strings and other
//! immutable values are shared (retained); arrays, objects, maps and sets are made anew, with
//! copies of what they hold.

use super::{Gen, Val};
use crate::types::*;
use std::fmt::Write;

impl<'c, 'a> Gen<'c, 'a> {
    /// `structuredClone(v)`: an owned copy of `v`.
    pub(crate) fn structured_clone(&mut self, v: Val) -> Val {
        if !self.needs_clone(v.ty, &mut Vec::new()) {
            return self.own(v);
        }
        let f = self.clone_fn(v.ty);
        let ct = self.ctype(v.ty);
        let x = self.fresh("cv");
        self.line(format!("{ct} {x} = {};", v.code));
        self.tmp(v.ty, &format!("{f}(&{x})"), true)
    }

    /// Whether a copy of a `t` differs from sharing it: it holds an array, an object, a map or
    /// a set (strings and numbers can't change).
    fn needs_clone(&mut self, t: TyId, stack: &mut Vec<TyId>) -> bool {
        if stack.contains(&t) {
            return false;
        }
        stack.push(t);
        let r = match self.tget(t) {
            Ty::Array(_) | Ty::Map(..) | Ty::Set(_) => true,
            Ty::Record(_) => self.is_obj(t) || self.c.types.fields_in_order(t).into_iter().any(|f| self.needs_clone(f.ty, stack)),
            Ty::Union(ms) => self.c.types.tys(ms).to_vec().into_iter().any(|m| self.needs_clone(m, stack)),
            _ => false,
        };
        stack.pop();
        r
    }

    /// `static T cl_N(const T *v)`: an owned deep copy of `*v`.
    fn clone_fn(&mut self, t: TyId) -> String {
        let name = format!("cl_{}", t.0);
        if !self.helpers_done.insert((t, 120)) {
            return name;
        }
        let ct = self.ctype(t);
        let _ = writeln!(self.protos, "static {ct} {name}(const {ct} *v);");
        let body = self.clone_body(t);
        self.helpers_after.push(format!("static {ct} {name}(const {ct} *v) {{\n{body}}}\n"));
        name
    }

    /// An owned copy of the `t` at `place` (a C lvalue).
    fn clone_of(&mut self, t: TyId, place: &str) -> String {
        if self.needs_clone(t, &mut Vec::new()) {
            let f = self.clone_fn(t);
            return format!("{f}(&{place})");
        }
        if self.is_rc(t) {
            let r = self.retain_code(t, place);
            return format!("({{ {r}; {place}; }})");
        }
        place.to_string()
    }

    fn clone_body(&mut self, t: TyId) -> String {
        let ct = self.ctype(t);
        match self.tget(t) {
            Ty::Array(e) => {
                let ect = self.ctype(e);
                let el = self.clone_of(e, "s[i]");
                format!(
                    "    tv_int n = tv_arr_len(*v);\n    if (n == 0) return tv_arr_fresh();\n    tv_arr r = tv_arr_new_len(sizeof({ect}), n);\n    const {ect} *s = (const {ect} *)tv_arr_data(*v);\n    {ect} *d = ({ect} *)tv_arr_data(r);\n    for (tv_int i = 0; i < n; i++) d[i] = {el};\n    return r;\n"
                )
            }
            Ty::Map(k, val) => {
                let (kd, vd) = (self.desc(k), self.desc(val));
                let (kct, vct) = (self.ctype(k), self.ctype(val));
                let kc = self.clone_of(k, &format!("(*({kct} *)kp)"));
                let vc = self.clone_of(val, &format!("(*({vct} *)vp)"));
                format!(
                    "    tv_map r = tv_map_is_object(*v) ? tv_map_fresh_object() : tv_map_fresh();\n    tv_int i = 0; void *kp, *vp;\n    while (tv_map_next(*v, {kd}, {vd}, &i, &kp, &vp)) {{\n        {kct} k2 = {kc}; {vct} v2 = {vc};\n        tv_map_set(&r, {kd}, {vd}, &k2, &v2);\n    }}\n    return r;\n"
                )
            }
            Ty::Set(k) => {
                let kd = self.desc(k);
                let kct = self.ctype(k);
                let kc = self.clone_of(k, &format!("(*({kct} *)kp)"));
                format!(
                    "    tv_map r = tv_map_fresh();\n    tv_int i = 0; void *kp, *vp;\n    while (tv_map_next(*v, {kd}, &tv_type_undefined, &i, &kp, &vp)) {{\n        {kct} k2 = {kc};\n        tv_map_set(&r, {kd}, &tv_type_undefined, &k2, NULL);\n    }}\n    return r;\n"
                )
            }
            Ty::Record(_) => {
                let fields = self.c.types.fields_in_order(t);
                let obj = self.is_obj(t);
                let rs = self.rec_struct(t);
                let src = if obj { "(*v)->v" } else { "(*v)" };
                let mut b = format!("    {rs} r = {src};\n");
                for f in fields {
                    let n = self.sym(f.name).to_string();
                    let c = self.clone_of(f.ty, &format!("{src}.f_{n}"));
                    let _ = writeln!(b, "    r.f_{n} = {c};");
                }
                if obj {
                    let bn = self.box_name(t);
                    let _ = writeln!(b, "    return mk_{bn}(r);");
                } else {
                    b.push_str("    return r;\n");
                }
                b
            }
            Ty::Union(ms) => {
                let ms = self.c.types.tys(ms).to_vec();
                if let Some((pi, _)) = self.niche(t) {
                    let c = self.clone_of(ms[pi], "x");
                    let mct = self.ctype(ms[pi]);
                    let (there, none) = (self.niche_there("*v", t), self.niche_none(t));
                    return format!("    if (!{there}) return {none};\n    {mct} x = *v;\n    return {c};\n");
                }
                let mut b = format!("    {ct} r = *v;\n    switch (v->tag) {{\n");
                for (i, &m) in ms.iter().enumerate() {
                    if self.is_unit(m) {
                        continue;
                    }
                    let c = self.clone_of(m, &format!("v->u.m{i}"));
                    let _ = writeln!(b, "    case {i}: r.u.m{i} = {c}; break;");
                }
                b.push_str("    default: break;\n    }\n    return r;\n");
                b
            }
            _ => {
                let c = self.clone_of(t, "(*v)");
                format!("    return {c};\n")
            }
        }
    }
}
