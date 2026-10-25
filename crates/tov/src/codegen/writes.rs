//! What a call can release: the types of the references it overwrites in the heap (a field of
//! type `Packet | null` stored to drops a `Packet`), with everything it calls. An object reached
//! through a field, passed to a call, only needs retaining for it when one of those could be
//! what keeps it alive: a type that can reach (through strong fields) the object's class.
//! (`quiet` is the case where nothing is overwritten at all.)

use super::Gen;
use crate::ast::{ArrowBody, ExprId, ExprKind, ItemKind, MemberKind, StmtId, StmtKind};
use crate::check::{Callee, IdentFact};
use crate::hash::FxSet;
use crate::types::*;

/// The types a body may drop references of; `None`: anything (a call it can't see into).
pub(crate) type Writes = Option<FxSet<TyId>>;

impl Gen<'_, '_> {
    /// Call `e` (module `m`) can't release an object of class type `t` (one reached through a
    /// field, borrowed for the call).
    pub(crate) fn call_spares(&mut self, m: u32, e: ExprId, t: TyId) -> bool {
        let t = self.c.types.without_nullish(t);
        let Some((decl, _)) = self.c.class_of(t) else { return false };
        let Some(ws) = self.writes_of_call(m, e) else { return false };
        // (and the call's own callbacks: written out here, see `writes_of_call`)
        !ws.iter().any(|&w| self.reaches(w, decl))
    }

    /// A reference of type `w` dropped could free an object of class `decl` (`w` can hold one,
    /// directly or through strong fields of what it holds).
    fn reaches(&mut self, w: TyId, decl: u32) -> bool {
        match self.reach_set(w) {
            None => true,
            Some(r) => r.iter().any(|&d| self.c.class_descends(d, decl) || self.c.class_descends(decl, d)),
        }
    }

    /// The classes a value of type `w` can hold strongly (through every field); `None`: any.
    fn reach_set(&mut self, w: TyId) -> Option<FxSet<u32>> {
        if let Some(r) = self.reach_memo.get(&w) {
            return r.clone();
        }
        let mut out = FxSet::default();
        let mut seen: FxSet<TyId> = FxSet::default();
        let ok = self.reach_into(w, &mut out, &mut seen, 0);
        let r = ok.then_some(out);
        self.reach_memo.insert(w, r.clone());
        r
    }

    fn reach_into(&mut self, t: TyId, out: &mut FxSet<u32>, seen: &mut FxSet<TyId>, depth: u32) -> bool {
        if depth > 64 || !seen.insert(t) {
            return depth <= 64;
        }
        if self.c.types.has_param(t) {
            return false;
        }
        match self.tget(t) {
            Ty::Class(decl, _) => {
                // (and what a subclass's instance, given as one, can hold)
                let decls: Vec<u32> = (0..self.c.classes.len() as u32).filter(|&d| d == decl || self.c.class_descends(d, decl)).collect();
                for d in decls {
                    if !out.insert(d) {
                        continue;
                    }
                    if !self.c.classes[d as usize].params.is_empty() {
                        return false;
                    }
                    self.c.resolve_class(d);
                    let fields: Vec<(TyId, bool)> = self.c.classes[d as usize].fields.iter().map(|f| (f.ty, f.weak)).collect();
                    for (ft, weak) in fields {
                        if !weak && !self.reach_into(ft, out, seen, depth + 1) {
                            return false;
                        }
                    }
                }
                true
            }
            Ty::Array(e) | Ty::Set(e) => self.reach_into(e, out, seen, depth + 1),
            Ty::Map(k, v) => self.reach_into(k, out, seen, depth + 1) && self.reach_into(v, out, seen, depth + 1),
            Ty::Union(ms) => {
                let ms = self.c.types.tys(ms).to_vec();
                ms.into_iter().all(|x| self.reach_into(x, out, seen, depth + 1))
            }
            Ty::Record(fs) => {
                let tys: Vec<TyId> = self.c.types.fields(fs).iter().map(|f| f.ty).collect();
                tys.into_iter().all(|x| self.reach_into(x, out, seen, depth + 1))
            }
            Ty::Rec(..) => {
                let u = self.c.unfold(t);
                self.reach_into(u, out, seen, depth + 1)
            }
            // (values that hold nothing)
            Ty::Int | Ty::F64 | Ty::F32 | Ty::I8 | Ty::I16 | Ty::I32 | Ty::U8 | Ty::U8C | Ty::U16 | Ty::U32 | Ty::U64 | Ty::Bool | Ty::Str | Ty::StrLit(_) | Ty::BigInt | Ty::Decimal | Ty::Undefined | Ty::Null | Ty::Void | Ty::BoolLit(_) => true,
            // (closures, interfaces, promises, dynamic values: anything)
            _ => false,
        }
    }

    /// What call (or `new`) `e` in module `m` may drop.
    pub(crate) fn writes_of_call(&mut self, m: u32, e: ExprId) -> Writes {
        let fact = self.facts(m).calls.get(&e).cloned()?;
        let mut acc = FxSet::default();
        let ok = match fact.callee {
            Callee::Fn(fm, fi) => match &self.c.modules[fm as usize].ast.items[fi as usize].kind {
                ItemKind::Function(f) if !f.is_async && !f.is_gen => self.writes_body(fm, f.body, false).map(|w| acc.extend(w)).is_some(),
                _ => false,
            },
            Callee::ClassMethod { recv, name, sup } => {
                let Some((decl, _)) = self.c.class_of(recv) else { return None };
                let mut impls: Vec<(u32, u32)> = Vec::new();
                let direct = sup || !self.quiet_overridden_pub(decl, name);
                for d in 0..self.c.classes.len() as u32 {
                    if d != decl && (direct || !self.c.class_descends(d, decl)) {
                        continue;
                    }
                    self.c.resolve_class(d);
                    if let Some(mm) = self.c.classes[d as usize].methods.iter().find(|mm| mm.name == name) {
                        let k = (mm.owner, mm.member);
                        if !impls.contains(&k) {
                            impls.push(k);
                        }
                    }
                }
                !impls.is_empty() && impls.into_iter().all(|(o, mi)| self.writes_member(o, mi).map(|w| acc.extend(w)).is_some())
            }
            Callee::StaticMethod(c, mi) => self.writes_member(c, mi).map(|w| acc.extend(w)).is_some(),
            Callee::New(c) => self.writes_ctor(c).map(|w| acc.extend(w)).is_some(),
            Callee::SuperCtor(t) => self.c.class_of(t).and_then(|(c, _)| self.writes_ctor(c)).map(|w| acc.extend(w)).is_some(),
            Callee::Builtin { ns, name } => match ns.map(|n| self.sym(n).to_string()) {
                None => !self.sym(name).starts_with("__"),
                // (`process.stdout.write(text)`: given no callback)
                Some(n) if matches!(n.as_str(), "stdout" | "stderr") => self.sym(name) == "write" && self.no_fn_args(m, e),
                Some(n) => matches!(n.as_str(), "Math" | "console" | "JSON" | "Number" | "String"),
            },
            Callee::Method(recv) => {
                let ExprKind::Call { callee, args, .. } = &self.ast(m).expr(e).kind else { return None };
                let ExprKind::Member { name, .. } = &self.ast(m).expr(*callee).kind else { return None };
                let n = self.sym(*name).to_string();
                let args: Vec<ExprId> = args.iter().map(|a| a.expr).collect();
                let collection = matches!(self.tget(recv), Ty::Array(_) | Ty::Map(..) | Ty::Set(_));
                if !collection && !matches!(self.tget(recv), Ty::Str | Ty::StrLit(_) | Ty::Int | Ty::F64 | Ty::Bool) {
                    return None;
                }
                if collection && matches!(n.as_str(), "push" | "pop" | "shift" | "unshift" | "splice" | "sort" | "reverse" | "fill" | "copyWithin" | "set" | "delete" | "clear" | "add") {
                    match self.tget(recv) {
                        Ty::Array(x) | Ty::Set(x) => {
                            acc.insert(x);
                        }
                        Ty::Map(k, v) => {
                            acc.insert(k);
                            acc.insert(v);
                        }
                        _ => {}
                    }
                }
                // (callbacks: written out here)
                args.into_iter().all(|a| {
                    let t = self.facts(m).expr_ty[a as usize];
                    match &self.ast(m).expr(a).kind {
                        ExprKind::Arrow(f) if !f.is_async => match f.body {
                            ArrowBody::Expr(x) => self.writes_expr(m, x, false, &mut acc),
                            ArrowBody::Block(s) => self.writes_stmt(m, s, false, &mut acc),
                        },
                        _ => !matches!(self.c.types.get(t), Ty::Func(..)),
                    }
                })
            }
            _ => false,
        };
        ok.then_some(acc)
    }

    fn writes_member(&mut self, c: u32, mi: u32) -> Writes {
        let m = self.c.classes[c as usize].module;
        match &self.c.class_decl(c).members[mi as usize].kind {
            MemberKind::Method(f) | MemberKind::Getter(f) if !f.is_async && !f.is_gen => self.writes_body(m, f.body, false),
            _ => None,
        }
    }

    fn writes_ctor(&mut self, c: u32) -> Writes {
        let mut acc = FxSet::default();
        let mut k = Some(c);
        let mut depth = 0;
        while let Some(cls) = k {
            depth += 1;
            if depth > 16 {
                return None;
            }
            let m = self.c.classes[cls as usize].module;
            let decl = self.c.class_decl(cls);
            for mem in &decl.members {
                if mem.is_static {
                    continue;
                }
                let ok = match &mem.kind {
                    MemberKind::Field { init: Some(i), .. } => self.writes_expr(m, *i, true, &mut acc),
                    MemberKind::Constructor(f) => self.writes_body(m, f.body, true).map(|w| acc.extend(w)).is_some(),
                    _ => true,
                };
                if !ok {
                    return None;
                }
            }
            k = self.c.classes[cls as usize].base.and_then(|b| self.c.class_of(b)).map(|(b, _)| b);
        }
        Some(acc)
    }

    fn writes_body(&mut self, m: u32, body: StmtId, ctor: bool) -> Writes {
        let key = (m, body, ctor);
        if let Some(w) = self.writes_memo.get(&key) {
            return w.clone();
        }
        if self.writes_stack.contains(&key) {
            // (a call back into a body being looked at: what the rest adds)
            self.quiet_tainted = true;
            return Some(FxSet::default());
        }
        if self.writes_stack.len() > 48 {
            return None;
        }
        self.writes_stack.push(key);
        let saved = std::mem::replace(&mut self.quiet_tainted, false);
        let mut acc = FxSet::default();
        let ok = self.writes_stmt(m, body, ctor, &mut acc);
        let tainted = self.quiet_tainted;
        self.writes_stack.pop();
        let w = ok.then_some(acc);
        // (a recursive body's: its own uses, taken in once the outermost is done)
        if w.is_none() || !tainted || self.writes_stack.is_empty() {
            self.writes_memo.insert(key, w.clone());
        }
        self.quiet_tainted = saved || (tainted && w.is_some());
        w
    }

    fn writes_stmt(&mut self, m: u32, s: StmtId, ctor: bool, acc: &mut FxSet<TyId>) -> bool {
        let ast = self.ast(m);
        if matches!(ast.stmt(s).kind, StmtKind::InitGlobal(_) | StmtKind::Error) {
            return false;
        }
        if let StmtKind::ForOf { iter, .. } = &ast.stmt(s).kind {
            let t = self.facts(m).expr_ty[*iter as usize];
            let t = self.c.types.without_undefined(t);
            if !matches!(self.c.types.get(t), Ty::Array(_) | Ty::Str | Ty::Map(..) | Ty::Set(_)) {
                return false;
            }
        }
        let (es, ss) = super::quiet::stmt_parts(ast, s);
        es.into_iter().all(|e| self.writes_expr(m, e, ctor, acc)) && ss.into_iter().all(|x| self.writes_stmt(m, x, ctor, acc))
    }

    fn writes_expr(&mut self, m: u32, e: ExprId, ctor: bool, acc: &mut FxSet<TyId>) -> bool {
        let ast = self.ast(m);
        match &ast.expr(e).kind {
            ExprKind::Assign(_, t, v) => {
                let (t, v) = (*t, *v);
                let all = [self.facts(m).expr_ty[t as usize], self.facts(m).expr_ty[v as usize], self.facts(m).expr_ty[e as usize]];
                let rc = all.iter().any(|&x| x == ERROR || self.c.types.has_param(x) || self.is_rc(x));
                // (an assignment's target has no type of its own recorded: the value's, and the
                // place's declared one below, stand for it)
                let tys: Vec<TyId> = all.into_iter().filter(|&x| x != ERROR).collect();
                if rc && tys.is_empty() {
                    return false;
                }
                let ok = match &ast.expr(t).kind {
                    _ if !rc => true,
                    ExprKind::Ident(_) => match self.ident_fact(m, t) {
                        Some(IdentFact::Local(_)) => true,
                        _ => {
                            acc.extend(tys.iter().copied());
                            true
                        }
                    },
                    // (a new object's own fields, set by its constructor)
                    ExprKind::Member { obj, .. } if ctor && matches!(ast.expr(*obj).kind, ExprKind::This) => true,
                    ExprKind::Member { obj, name, .. } => {
                        let ot = self.facts(m).expr_ty[*obj as usize];
                        let ot = self.c.types.without_nullish(ot);
                        if self.c.class_of(ot).is_some()
                            && let Some(f) = self.class_field(ot, *name)
                        {
                            acc.insert(f.ty);
                        }
                        acc.extend(tys.iter().copied());
                        self.writes_expr(m, *obj, ctor, acc)
                    }
                    ExprKind::Index { obj, index, .. } => {
                        let ot = self.facts(m).expr_ty[*obj as usize];
                        match self.tget(ot) {
                            Ty::Array(x) => {
                                acc.insert(x);
                                acc.extend(tys.iter().copied());
                                self.writes_expr(m, *obj, ctor, acc) && self.writes_expr(m, *index, ctor, acc)
                            }
                            _ => false,
                        }
                    }
                    _ => false,
                };
                ok && self.writes_expr(m, v, ctor, acc)
            }
            ExprKind::Call { callee, args, .. } => {
                let (callee, args): (ExprId, Vec<ExprId>) = (*callee, args.iter().map(|a| a.expr).collect());
                let recv_ok = match &ast.expr(callee).kind {
                    ExprKind::Member { obj, .. } => self.writes_expr(m, *obj, ctor, acc),
                    ExprKind::Ident(_) | ExprKind::Super => true,
                    _ => false,
                };
                if !recv_ok {
                    return false;
                }
                let called = match self.facts(m).calls.get(&e).map(|f| f.callee.clone()) {
                    // (through a callback parameter: what its closures write)
                    Some(Callee::Value) => self.writes_through_param(m, callee, acc) || self.writes_through_field(m, callee, acc),
                    _ => match self.writes_of_call(m, e) {
                        Some(w) => {
                            acc.extend(w);
                            true
                        }
                        None => false,
                    },
                };
                called && args.into_iter().all(|a| matches!(ast.expr(a).kind, ExprKind::Arrow(_)) || self.writes_expr(m, a, ctor, acc))
            }
            ExprKind::New { args, .. } => {
                let args: Vec<ExprId> = args.iter().map(|a| a.expr).collect();
                match self.writes_of_call(m, e) {
                    Some(w) => acc.extend(w),
                    None => return false,
                }
                args.into_iter().all(|a| self.writes_expr(m, a, ctor, acc))
            }
            ExprKind::Member { obj, name, .. } => {
                let ot = self.facts(m).expr_ty[*obj as usize];
                let ot = self.c.types.without_undefined(ot);
                if self.c.class_of(ot).is_some()
                    && let Some(crate::check::class::ClassMemberRef::Getter(g)) = self.c.class_member(ot, *name)
                {
                    match self.writes_member(g.owner, g.member) {
                        Some(w) => acc.extend(w),
                        None => return false,
                    }
                }
                self.writes_expr(m, *obj, ctor, acc)
            }
            ExprKind::Template(_, xs) => {
                let xs = xs.clone();
                xs.into_iter().all(|x| {
                    let t = self.facts(m).expr_ty[x as usize];
                    let t = self.c.types.without_undefined(t);
                    !matches!(self.c.types.get(t), Ty::Class(..) | Ty::Interface(..)) && self.writes_expr(m, x, ctor, acc)
                })
            }
            ExprKind::Await(_) | ExprKind::Yield(..) | ExprKind::Error => false,
            ExprKind::Arrow(_) => true,
            _ => {
                let parts = super::quiet::expr_parts(ast, e);
                parts.into_iter().all(|x| self.writes_expr(m, x, ctor, acc))
            }
        }
    }

    /// Call `e` is given no functions (a callback that could run later).
    pub(crate) fn no_fn_args(&mut self, m: u32, e: ExprId) -> bool {
        let ExprKind::Call { args, .. } = &self.ast(m).expr(e).kind else { return false };
        let args: Vec<ExprId> = args.iter().map(|a| a.expr).collect();
        args.into_iter().all(|a| {
            let t = self.facts(m).expr_ty[a as usize];
            !matches!(self.c.types.get(t), Ty::Func(..)) && !matches!(self.ast(m).expr(a).kind, ExprKind::Arrow(_))
        })
    }

    /// `obj.f(...)` with `f` a class field of a function type: what the closures it can hold
    /// write (see `field_closures`).
    fn writes_through_field(&mut self, m: u32, callee: ExprId, acc: &mut FxSet<TyId>) -> bool {
        let mut c = callee;
        while let ExprKind::Paren(x) = self.ast(m).expr(c).kind {
            c = x;
        }
        let ExprKind::Member { obj, name, .. } = self.ast(m).expr(c).kind else { return false };
        let ot = self.facts(m).expr_ty[obj as usize];
        let ot = self.c.types.without_nullish(ot);
        if self.c.class_of(ot).is_none() || self.class_field(ot, name).is_none() {
            return false;
        }
        let Some(arrows) = self.field_closures(name) else { return false };
        arrows.into_iter().all(|(am, a)| {
            if self.field_arrows.contains(&(am, a)) {
                // (a closure calling itself through the field: what the rest adds)
                self.quiet_tainted = true;
                return true;
            }
            self.field_arrows.push((am, a));
            let ok = match &self.ast(am).expr(a).kind {
                ExprKind::Arrow(f) if !f.is_async => match f.body {
                    ArrowBody::Expr(x) => self.writes_expr(am, x, false, acc),
                    ArrowBody::Block(s) => self.writes_stmt(am, s, false, acc),
                },
                _ => false,
            };
            self.field_arrows.pop();
            ok
        })
    }

    /// `fn(...)` with `fn` a parameter of the body being looked at: what the closures every
    /// call gives it write (see `param_quiet`).
    fn writes_through_param(&mut self, m: u32, callee: ExprId, acc: &mut FxSet<TyId>) -> bool {
        let mut c = callee;
        while let ExprKind::Paren(x) = self.ast(m).expr(c).kind {
            c = x;
        }
        let Some(IdentFact::Local(k)) = self.ident_fact(m, c) else { return false };
        let Some(&(om, ob, _)) = self.writes_stack.last() else { return false };
        if om != m {
            return false;
        }
        match self.param_closures(om, ob, k) {
            Some(arrows) => arrows.into_iter().all(|(am, a)| match &self.ast(am).expr(a).kind {
                ExprKind::Arrow(f) if !f.is_async => match f.body {
                    ArrowBody::Expr(x) => self.writes_expr(am, x, false, acc),
                    ArrowBody::Block(s) => self.writes_stmt(am, s, false, acc),
                },
                _ => false,
            }),
            None => false,
        }
    }
}
