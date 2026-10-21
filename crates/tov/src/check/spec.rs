//! `number` checked as `int` where it only ever holds integers.
//!
//! TypeScript has one number type; Tov has `int` and `f64`, and integer code is much faster
//! (`fib(n: number)` runs twice as fast on `int`). Each `number` in a user module's type
//! annotations is a *site*, checked as `int` until a float flows into it. A mismatch then blames
//! the sites the value's type came from (its sources) and the annotation it was checked against
//! (the flow's destination); they become `f64` and the program is checked again, until nothing
//! changes. A program that still has errors is checked once more with every `number` an `f64`,
//! as written, so speculation never rejects a program that compiles without it.

use super::*;
use crate::ast::{BinOp, ExprKind};

/// A `number` in a type annotation: (module, type node).
pub(crate) type Site = (u32, ast::TypeId);

/// Rounds of checking with more floats before giving up on speculation.
const MAX_ROUNDS: u32 = 8;

/// After a round checked with speculation: `Some(true)` to check again with the floats it
/// found, `Some(false)` to check again without speculation (it failed), `None` when done.
pub(crate) fn next_round(floats: &mut HashSet<Site>, hits: &[Site], failed: bool, rounds: u32) -> Option<bool> {
    let before = floats.len();
    floats.extend(hits.iter().copied());
    if floats.len() > before && rounds < MAX_ROUNDS {
        return Some(true);
    }
    if floats.len() > before || failed {
        return Some(false);
    }
    None
}

#[derive(Default)]
pub(crate) struct Spec {
    pub(crate) on: bool,
    /// Sites a float flows into: checked as `f64`.
    pub(crate) floats: HashSet<Site>,
}

impl<'a> Checker<'a> {
    /// `te` (in the current module) is a `number` checked as `int`.
    pub(super) fn spec_int(&self, te: ast::TypeId) -> bool {
        self.spec_site(self.cur, te)
    }

    fn spec_site(&self, m: u32, te: ast::TypeId) -> bool {
        let module = &self.modules[m as usize];
        if !self.spec.on || module.std || module.builtin || module.npm.is_some() || self.spec.floats.contains(&(m, te)) {
            return false;
        }
        matches!(&module.ast.ty(te).kind, TypeExprKind::Named { ns: None, name, args, .. } if *name == self.syms.number && args.is_empty())
    }

    /// The speculated `number`s written in an annotation (record fields reached through an alias
    /// are found by `spec_diff` instead).
    pub(super) fn type_sites(&self, m: u32, te: ast::TypeId) -> Vec<Site> {
        let mut out = Vec::new();
        if self.spec.on {
            self.type_sites_into(m, te, &mut out);
        }
        out
    }

    fn type_sites_into(&self, m: u32, te: ast::TypeId, out: &mut Vec<Site>) {
        if self.spec_site(m, te) {
            out.push((m, te));
            return;
        }
        match &self.modules[m as usize].ast.ty(te).kind {
            TypeExprKind::Array(e) => self.type_sites_into(m, *e, out),
            TypeExprKind::Union(ms) => ms.iter().for_each(|&t| self.type_sites_into(m, t, out)),
            TypeExprKind::Record(fields) => fields.iter().for_each(|f| self.type_sites_into(m, f.ty, out)),
            TypeExprKind::Func(params, ret, _) => {
                for p in params {
                    if let Some(t) = p.ty {
                        self.type_sites_into(m, t, out);
                    }
                }
                self.type_sites_into(m, *ret, out);
            }
            TypeExprKind::Named { args, .. } => args.iter().for_each(|&t| self.type_sites_into(m, t, out)),
            _ => {}
        }
    }

    /// The sites of a function's parameters and of its return type.
    pub(super) fn decl_sites(&self, m: u32, f: &ast::FnDecl) -> (Vec<Vec<Site>>, Vec<Site>) {
        let params = f.params.iter().map(|p| p.ty.map(|t| self.type_sites(m, t)).unwrap_or_default()).collect();
        let ret = f.ret.map(|t| self.type_sites(m, t)).unwrap_or_default();
        (params, ret)
    }

    /// Records where an expression's value type came from.
    pub(super) fn note_sites(&mut self, span: Span, sites: Vec<Site>) {
        if !sites.is_empty() {
            self.spec_srcs.entry(span).or_default().extend(sites);
        }
    }

    /// Where the value of the expression at `span` got its type.
    pub(super) fn sources(&self, span: Span) -> Vec<Site> {
        let mut out = self.spec_srcs.get(&span).cloned().unwrap_or_default();
        if let Some(b) = self.spec_srcs_base.get(&span) {
            out.extend_from_slice(b);
        }
        out
    }

    pub(super) fn blame(&mut self, sites: &[Site]) {
        self.spec_hits.extend_from_slice(sites);
    }

    /// Checks `f` with `sites` as the destination of the values it checks.
    pub(super) fn flow<R>(&mut self, sites: Vec<Site>, f: impl FnOnce(&mut Self) -> R) -> R {
        if !self.spec.on {
            return f(self);
        }
        self.spec_flow.push(sites);
        let r = f(self);
        self.spec_flow.pop();
        r
    }

    /// A value of type `found` (the expression at `span`) doesn't fit `expected`.
    pub(super) fn spec_mismatch(&mut self, found: TyId, expected: TyId, span: Span) {
        if !self.spec.on {
            return;
        }
        let mut hits = self.sources(span);
        if let Some(top) = self.spec_flow.last() {
            hits.extend_from_slice(top);
        }
        self.spec_diff(found, expected, &mut hits, 0);
        self.spec_hits.extend(hits);
    }

    /// Record fields where `found` has a float and `expected` an `int` (or the other way round,
    /// where that's no widening: inside arrays and maps).
    fn spec_diff(&mut self, found: TyId, expected: TyId, out: &mut Vec<Site>, depth: u32) {
        if depth > 8 || found == expected {
            return;
        }
        let (f, e) = (self.types.without_undefined(found), self.types.without_undefined(expected));
        match (self.types.get(f).clone(), self.types.get(e).clone()) {
            (Ty::Record(ff), Ty::Record(ef)) => {
                let (ff, ef) = (self.types.fields(ff).to_vec(), self.types.fields(ef).to_vec());
                for x in &ef {
                    if let Some(y) = ff.iter().find(|y| y.name == x.name)
                        && y.ty != x.ty
                    {
                        self.field_sites(e, x.name, out);
                        self.field_sites(f, x.name, out);
                        self.spec_diff(y.ty, x.ty, out, depth + 1);
                    }
                }
            }
            (Ty::Array(a), Ty::Array(b)) => self.spec_diff(a, b, out, depth + 1),
            (Ty::Map(k1, v1), Ty::Map(k2, v2)) => {
                self.spec_diff(k1, k2, out, depth + 1);
                self.spec_diff(v1, v2, out, depth + 1);
            }
            (Ty::Set(a), Ty::Set(b)) => self.spec_diff(a, b, out, depth + 1),
            (Ty::Promise(a, _), Ty::Promise(b, _)) => self.spec_diff(a, b, out, depth + 1),
            (Ty::Union(a), Ty::Union(b)) => {
                let (a, b) = (self.types.tys(a).to_vec(), self.types.tys(b).to_vec());
                for &x in &a {
                    for &y in &b {
                        self.spec_diff(x, y, out, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }

    fn field_sites(&self, record: TyId, name: Sym, out: &mut Vec<Site>) {
        if let Some(s) = self.spec_fields.get(&(record, name)).or_else(|| self.spec_fields_base.get(&(record, name))) {
            out.extend_from_slice(s);
        }
    }

    /// The sites of a record type's fields, for `p.x` on a record `p`.
    pub(super) fn record_field_sites(&mut self, record: TyId, name: Sym) -> Vec<Site> {
        let mut out = Vec::new();
        let record = self.types.without_undefined(record);
        self.field_sites(record, name, &mut out);
        out
    }

    /// Every speculated site inside `ty` reachable through record fields: data from outside the
    /// program (`JSON.parse`) may hold any number.
    pub(super) fn spec_poison(&mut self, ty: TyId) {
        if !self.spec.on {
            return;
        }
        let mut out = Vec::new();
        let mut seen = HashSet::default();
        self.poison_into(ty, &mut out, &mut seen);
        if let Some(top) = self.spec_flow.last() {
            out.extend_from_slice(top);
        }
        self.spec_hits.extend(out);
    }

    fn poison_into(&mut self, ty: TyId, out: &mut Vec<Site>, seen: &mut HashSet<TyId>) {
        if !seen.insert(ty) {
            return;
        }
        match self.types.get(ty).clone() {
            Ty::Record(fs) => {
                for f in self.types.fields(fs).to_vec() {
                    self.field_sites(ty, f.name, out);
                    self.poison_into(f.ty, out, seen);
                }
            }
            Ty::Array(e) | Ty::Set(e) => self.poison_into(e, out, seen),
            Ty::Map(k, v) => {
                self.poison_into(k, out, seen);
                self.poison_into(v, out, seen);
            }
            Ty::Union(ms) => {
                for m in self.types.tys(ms).to_vec() {
                    self.poison_into(m, out, seen);
                }
            }
            Ty::Promise(v, _) => self.poison_into(v, out, seen),
            Ty::Rec(..) => {
                let inner = self.unfold(ty);
                self.poison_into(inner, out, seen);
            }
            _ => {}
        }
    }

    /// A class field's sites (its annotation, or its constructor parameter's).
    pub(super) fn class_field_sites(&self, cf: &class::CField) -> Vec<Site> {
        if !self.spec.on {
            return Vec::new();
        }
        let info = &self.classes[cf.owner as usize];
        let m = info.module;
        let decl = self.class_decl(cf.owner);
        if let Some(mi) = cf.member
            && let ast::MemberKind::Field { ty: Some(t), .. } = &decl.members[mi as usize].kind
        {
            return self.type_sites(m, *t);
        }
        if let Some(pi) = cf.prop_param {
            for mem in &decl.members {
                if let ast::MemberKind::Constructor(f) = &mem.kind
                    && let Some(p) = f.params.get(pi as usize)
                    && let Some(t) = p.ty
                {
                    return self.type_sites(m, t);
                }
            }
        }
        Vec::new()
    }

    /// A method's (or constructor's) parameter and return sites.
    pub(super) fn member_sites(&self, owner: u32, member: u32) -> (Vec<Vec<Site>>, Vec<Site>) {
        if !self.spec.on {
            return Default::default();
        }
        let m = self.classes[owner as usize].module;
        match &self.class_decl(owner).members[member as usize].kind {
            ast::MemberKind::Method(f) | ast::MemberKind::Getter(f) | ast::MemberKind::Constructor(f) => self.decl_sites(m, f),
            _ => Default::default(),
        }
    }

    /// The constructor that `new C(...)` runs: its parameter sites.
    pub(super) fn ctor_sites(&self, c: u32) -> Vec<Vec<Site>> {
        if !self.spec.on {
            return Vec::new();
        }
        let mut k = Some(c);
        while let Some(cls) = k {
            for (i, mem) in self.class_decl(cls).members.iter().enumerate() {
                if matches!(mem.kind, ast::MemberKind::Constructor(_)) {
                    return self.member_sites(cls, i as u32).0;
                }
            }
            k = self.classes[cls as usize].base.and_then(|b| self.class_of(b)).map(|(b, _)| b);
        }
        Vec::new()
    }

    /// After checking `e`: the sources of values that pass one through (`(x)`, `c ? a : b`, an
    /// array literal's elements, an element of an array) and of annotated expressions.
    pub(super) fn derive_sites(&mut self, e: ExprId) {
        let m = self.cur;
        let ast = self.ast();
        let node = ast.expr(e);
        let mut out = Vec::new();
        let from = |c: &Self, x: ExprId, out: &mut Vec<Site>| out.extend(c.sources(c.ast().expr(x).span));
        match &node.kind {
            ExprKind::Paren(x) | ExprKind::NonNull(x) | ExprKind::Try(x) | ExprKind::Await(x) => from(self, *x, &mut out),
            ExprKind::Index { obj, .. } => from(self, *obj, &mut out),
            ExprKind::Cond(_, a, b) | ExprKind::Binary(BinOp::Nullish, a, b) => {
                from(self, *a, &mut out);
                from(self, *b, &mut out);
            }
            ExprKind::Array(xs) => xs.iter().for_each(|&x| from(self, x, &mut out)),
            ExprKind::As(x, t) => {
                from(self, *x, &mut out);
                out.extend(self.type_sites(m, *t));
            }
            ExprKind::New { type_args, .. } => type_args.iter().for_each(|&t| out.extend(self.type_sites(m, t))),
            ExprKind::Arrow(f) => {
                for p in &f.params {
                    if let Some(t) = p.ty {
                        out.extend(self.type_sites(m, t));
                    }
                }
                if let Some(t) = f.ret {
                    out.extend(self.type_sites(m, t));
                }
            }
            _ => return,
        }
        self.note_sites(node.span, out);
    }

    /// A module variable's annotation sites (or its initializer's, when it has none).
    pub(super) fn const_sites(&self, m: u32, i: u32) -> Vec<Site> {
        match &self.modules[m as usize].ast.items[i as usize].kind {
            ItemKind::Const { ty: Some(t), .. } => self.type_sites(m, *t),
            ItemKind::Const { init, .. } => self.sources(self.modules[m as usize].ast.expr(*init).span),
            _ => Vec::new(),
        }
    }

    /// A function item's parameter and return sites.
    pub(super) fn fn_item_sites(&self, m: u32, i: u32) -> (Vec<Vec<Site>>, Vec<Site>) {
        match &self.modules[m as usize].ast.items[i as usize].kind {
            ItemKind::Function(f) if self.spec.on => self.decl_sites(m, f),
            _ => Default::default(),
        }
    }

    /// Before `call_sig` checks a call: where its arguments go and its result comes from.
    pub(super) fn set_call_spec(&mut self, spec: (Vec<Vec<Site>>, Vec<Site>)) {
        if self.spec.on {
            self.call_spec = Some(spec);
        }
    }

    /// A call through a value (a function, or a method of an array or map): its arguments go,
    /// and its result comes, wherever the value's own type came from.
    pub(super) fn set_call_spec_from(&mut self, value: ExprId) {
        if self.spec.on {
            let s = self.sources(self.ast().expr(value).span);
            self.call_spec = Some((vec![s.clone()], s));
        }
    }

    /// `x.name`: the sites of the field (or getter) of each record or class `x` may be.
    pub(super) fn member_sites_of(&mut self, base: TyId, name: Sym) -> Vec<Site> {
        let mut out = Vec::new();
        if !self.spec.on {
            return out;
        }
        for m in self.flat_members(base) {
            if self.class_of(m).is_some() {
                match self.class_member(m, name) {
                    Some(class::ClassMemberRef::Field(f)) => out.extend(self.class_field_sites(&f)),
                    Some(class::ClassMemberRef::Getter(g)) => out.extend(self.member_sites(g.owner, g.member).1),
                    _ => {}
                }
            } else {
                self.field_sites(m, name, &mut out);
            }
        }
        out
    }
}
