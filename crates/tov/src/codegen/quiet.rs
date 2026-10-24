//! Quiet calls: a function or method that (with everything it calls) stores no reference into
//! the heap, so it can't release anything it didn't make. An object reached through a field
//! stays alive while one runs: it's passed borrowed, not retained for the call.

use super::Gen;
use crate::ast::{ArrowBody, ExprId, ExprKind, ItemKind, MemberKind, StmtId, StmtKind};
use crate::check::{Callee, IdentFact};
use crate::types::*;

impl Gen<'_, '_> {
    /// Call (or `new`) `e` in module `m` is quiet.
    pub(crate) fn quiet_call(&mut self, m: u32, e: ExprId) -> bool {
        let Some(fact) = self.facts(m).calls.get(&e).cloned() else { return false };
        match fact.callee {
            Callee::Fn(fm, fi) => match &self.c.modules[fm as usize].ast.items[fi as usize].kind {
                ItemKind::Function(f) if !f.is_async && !f.is_gen => self.quiet_body(fm, f.body, false),
                _ => false,
            },
            Callee::ClassMethod { recv, name, sup } => {
                let Some((decl, _)) = self.c.class_of(recv) else { return false };
                let mut impls: Vec<(u32, u32)> = Vec::new();
                let direct = sup || !self.quiet_overridden(decl, name);
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
                !impls.is_empty() && impls.into_iter().all(|(o, mi)| self.quiet_member(o, mi))
            }
            Callee::StaticMethod(c, mi) => self.quiet_member(c, mi),
            Callee::New(c) => self.quiet_ctor(c),
            Callee::SuperCtor(t) => self.c.class_of(t).is_some_and(|(c, _)| self.quiet_ctor(c)),
            // (not `Object.assign`, or `delete`)
            Callee::Builtin { ns, name } => match ns.map(|n| self.sym(n).to_string()) {
                None => !self.sym(name).starts_with("__"),
                Some(n) => matches!(n.as_str(), "Math" | "console" | "JSON" | "Number" | "String"),
            },
            Callee::Method(recv) => {
                let ExprKind::Call { callee, args, .. } = &self.ast(m).expr(e).kind else { return false };
                let ExprKind::Member { name, .. } = &self.ast(m).expr(*callee).kind else { return false };
                let n = self.sym(*name).to_string();
                let collection = matches!(self.tget(recv), Ty::Array(_) | Ty::Map(..) | Ty::Set(_));
                if collection && matches!(n.as_str(), "push" | "pop" | "shift" | "unshift" | "splice" | "sort" | "reverse" | "fill" | "copyWithin" | "set" | "delete" | "clear" | "add") {
                    return false;
                }
                if !collection && !matches!(self.tget(recv), Ty::Str | Ty::StrLit(_) | Ty::Int | Ty::F64 | Ty::Bool) {
                    return false;
                }
                // (callbacks: written out here, and quiet themselves)
                let args: Vec<ExprId> = args.iter().map(|a| a.expr).collect();
                args.into_iter().all(|a| {
                    let t = self.facts(m).expr_ty[a as usize];
                    match &self.ast(m).expr(a).kind {
                        ExprKind::Arrow(f) if !f.is_async => match f.body {
                            ArrowBody::Expr(x) => self.quiet_expr(m, x, false),
                            ArrowBody::Block(s) => self.quiet_stmt(m, s, false),
                        },
                        _ => !matches!(self.c.types.get(t), Ty::Func(..)),
                    }
                })
            }
            _ => false,
        }
    }

    fn quiet_overridden(&mut self, decl: u32, name: crate::intern::Sym) -> bool {
        let base = self.c.classes[decl as usize].methods.iter().find(|m| m.name == name).map(|m| (m.owner, m.member));
        (0..self.c.classes.len() as u32).any(|d| {
            d != decl && self.c.class_descends(d, decl) && {
                self.c.resolve_class(d);
                self.c.classes[d as usize].methods.iter().find(|m| m.name == name).map(|m| (m.owner, m.member)) != base
            }
        })
    }

    fn quiet_member(&mut self, c: u32, mi: u32) -> bool {
        let m = self.c.classes[c as usize].module;
        match &self.c.class_decl(c).members[mi as usize].kind {
            MemberKind::Method(f) | MemberKind::Getter(f) if !f.is_async && !f.is_gen => self.quiet_body(m, f.body, false),
            _ => false,
        }
    }

    /// `new C(...)`: its field initializers and constructors (and its bases'), which may set
    /// the new object's own fields.
    fn quiet_ctor(&mut self, c: u32) -> bool {
        let mut k = Some(c);
        let mut depth = 0;
        while let Some(cls) = k {
            depth += 1;
            if depth > 16 {
                return false;
            }
            let m = self.c.classes[cls as usize].module;
            let decl = self.c.class_decl(cls);
            for mem in &decl.members {
                if mem.is_static {
                    continue;
                }
                let ok = match &mem.kind {
                    MemberKind::Field { init: Some(i), .. } => self.quiet_expr(m, *i, true),
                    MemberKind::Constructor(f) => self.quiet_body(m, f.body, true),
                    _ => true,
                };
                if !ok {
                    return false;
                }
            }
            k = self.c.classes[cls as usize].base.and_then(|b| self.c.class_of(b)).map(|(b, _)| b);
        }
        true
    }

    fn quiet_body(&mut self, m: u32, body: StmtId, ctor: bool) -> bool {
        let key = (m, body, ctor);
        if let Some(&q) = self.quiet_memo.get(&key) {
            return q;
        }
        // (a call back into a body being looked at: quiet if the rest is)
        if self.quiet_stack.contains(&key) {
            self.quiet_tainted = true;
            return true;
        }
        if self.quiet_stack.len() > 48 {
            return false;
        }
        self.quiet_stack.push(key);
        let saved = std::mem::replace(&mut self.quiet_tainted, false);
        let q = self.quiet_stmt(m, body, ctor);
        let tainted = self.quiet_tainted;
        self.quiet_stack.pop();
        if !q || !tainted || self.quiet_stack.is_empty() {
            self.quiet_memo.insert(key, q);
        }
        self.quiet_tainted = saved || (tainted && q);
        q
    }

    fn quiet_stmt(&mut self, m: u32, s: StmtId, ctor: bool) -> bool {
        let ast = self.ast(m);
        match &ast.stmt(s).kind {
            StmtKind::Let { init, .. } => init.is_none_or(|i| self.quiet_expr(m, i, ctor)),
            StmtKind::Expr(e) | StmtKind::Throw(e) => self.quiet_expr(m, *e, ctor),
            StmtKind::Return(e) => e.is_none_or(|e| self.quiet_expr(m, e, ctor)),
            StmtKind::If(c, t, e) => self.quiet_expr(m, *c, ctor) && self.quiet_stmt(m, *t, ctor) && e.is_none_or(|e| self.quiet_stmt(m, e, ctor)),
            StmtKind::While(c, b) | StmtKind::DoWhile(b, c) => self.quiet_expr(m, *c, ctor) && self.quiet_stmt(m, *b, ctor),
            StmtKind::For { init, cond, step, body } => {
                init.is_none_or(|i| self.quiet_stmt(m, i, ctor))
                    && cond.is_none_or(|c| self.quiet_expr(m, c, ctor))
                    && step.is_none_or(|x| self.quiet_expr(m, x, ctor))
                    && self.quiet_stmt(m, *body, ctor)
            }
            StmtKind::ForOf { iter, body, .. } => {
                let t = self.facts(m).expr_ty[*iter as usize];
                let t = self.c.types.without_undefined(t);
                matches!(self.c.types.get(t), Ty::Array(_) | Ty::Str | Ty::Map(..) | Ty::Set(_)) && self.quiet_expr(m, *iter, ctor) && self.quiet_stmt(m, *body, ctor)
            }
            StmtKind::Switch(d, cases) => {
                let cases: Vec<(Option<ExprId>, Vec<StmtId>)> = cases.iter().map(|c| (c.test, c.body.clone())).collect();
                self.quiet_expr(m, *d, ctor) && cases.into_iter().all(|(t, b)| t.is_none_or(|t| self.quiet_expr(m, t, ctor)) && b.into_iter().all(|s| self.quiet_stmt(m, s, ctor)))
            }
            StmtKind::Block(ss) => {
                let ss = ss.clone();
                ss.into_iter().all(|s| self.quiet_stmt(m, s, ctor))
            }
            StmtKind::Try { body, catch, finally } => {
                let c = catch.as_ref().map(|c| c.body);
                self.quiet_stmt(m, *body, ctor) && c.is_none_or(|c| self.quiet_stmt(m, c, ctor)) && finally.is_none_or(|f| self.quiet_stmt(m, f, ctor))
            }
            StmtKind::Break(_) | StmtKind::Continue(_) | StmtKind::Empty => true,
            StmtKind::InitGlobal(_) | StmtKind::Error => false,
        }
    }

    /// A value of type `t` (in module code) may hold a reference: storing it may release one.
    fn quiet_rc(&mut self, t: TyId) -> bool {
        t == UNKNOWN || self.c.types.has_param(t) || self.is_rc(t)
    }

    fn quiet_expr(&mut self, m: u32, e: ExprId, ctor: bool) -> bool {
        let ast = self.ast(m);
        match &ast.expr(e).kind {
            ExprKind::Assign(_, t, v) => {
                // (what's stored: by the target's, the value's or the assignment's type, any that
                // may hold a reference)
                let tys = [self.facts(m).expr_ty[*t as usize], self.facts(m).expr_ty[*v as usize], self.facts(m).expr_ty[e as usize]];
                let rc = tys.into_iter().any(|t| t == ERROR || self.quiet_rc(t));
                let tt = if rc { UNKNOWN } else { INT };
                let ok = match &ast.expr(*t).kind {
                    ExprKind::Ident(_) => match self.ident_fact(m, *t) {
                        Some(IdentFact::Local(_)) => true,
                        _ => !self.quiet_rc(tt),
                    },
                    // (a new object's own fields, set by its constructor)
                    ExprKind::Member { obj, .. } if ctor && matches!(ast.expr(*obj).kind, ExprKind::This) => true,
                    ExprKind::Member { obj, .. } => {
                        let ot = self.facts(m).expr_ty[*obj as usize];
                        let ot = self.c.types.without_undefined(ot);
                        !self.quiet_rc(tt) && matches!(self.c.types.get(ot), Ty::Class(..) | Ty::Record(_)) && self.quiet_expr(m, *obj, ctor)
                    }
                    ExprKind::Index { obj, index, .. } => {
                        let ot = self.facts(m).expr_ty[*obj as usize];
                        !self.quiet_rc(tt) && matches!(self.c.types.get(ot), Ty::Array(_)) && self.quiet_expr(m, *obj, ctor) && self.quiet_expr(m, *index, ctor)
                    }
                    _ => false,
                };
                ok && self.quiet_expr(m, *v, ctor)
            }
            ExprKind::Update { target, .. } => match &ast.expr(*target).kind {
                ExprKind::Ident(_) => true,
                ExprKind::Member { obj, .. } => self.quiet_expr(m, *obj, ctor),
                ExprKind::Index { obj, index, .. } => self.quiet_expr(m, *obj, ctor) && self.quiet_expr(m, *index, ctor),
                _ => false,
            },
            ExprKind::Call { callee, args, .. } => {
                let (callee, args): (ExprId, Vec<ExprId>) = (*callee, args.iter().map(|a| a.expr).collect());
                let recv_ok = match &ast.expr(callee).kind {
                    ExprKind::Member { obj, .. } => self.quiet_expr(m, *obj, ctor),
                    ExprKind::Ident(_) | ExprKind::Super => true,
                    _ => false,
                };
                recv_ok && self.quiet_call(m, e) && args.into_iter().all(|a| matches!(ast.expr(a).kind, ExprKind::Arrow(_)) || self.quiet_expr(m, a, ctor))
            }
            ExprKind::New { args, .. } => {
                let args: Vec<ExprId> = args.iter().map(|a| a.expr).collect();
                self.quiet_call(m, e) && args.into_iter().all(|a| self.quiet_expr(m, a, ctor))
            }
            ExprKind::Member { obj, name, .. } => {
                let ot = self.facts(m).expr_ty[*obj as usize];
                let ot = self.c.types.without_undefined(ot);
                // (a getter runs)
                if self.c.class_of(ot).is_some()
                    && let Some(crate::check::class::ClassMemberRef::Getter(g)) = self.c.class_member(ot, *name)
                    && !self.quiet_member(g.owner, g.member)
                {
                    return false;
                }
                self.quiet_expr(m, *obj, ctor)
            }
            ExprKind::Template(_, xs) => {
                let xs = xs.clone();
                xs.into_iter().all(|x| {
                    let t = self.facts(m).expr_ty[x as usize];
                    let t = self.c.types.without_undefined(t);
                    !matches!(self.c.types.get(t), Ty::Class(..) | Ty::Interface(..)) && self.quiet_expr(m, x, ctor)
                })
            }
            ExprKind::Await(_) | ExprKind::Yield(..) | ExprKind::Error => false,
            ExprKind::Arrow(_) => true,
            ExprKind::Unary(_, x) | ExprKind::Paren(x) | ExprKind::NonNull(x) | ExprKind::Typeof(x) | ExprKind::Spread(x) | ExprKind::As(x, _) | ExprKind::Try(x) => self.quiet_expr(m, *x, ctor),
            ExprKind::Binary(_, l, r) => self.quiet_expr(m, *l, ctor) && self.quiet_expr(m, *r, ctor),
            ExprKind::Index { obj, index, .. } => self.quiet_expr(m, *obj, ctor) && self.quiet_expr(m, *index, ctor),
            ExprKind::Cond(a, b, c) => self.quiet_expr(m, *a, ctor) && self.quiet_expr(m, *b, ctor) && self.quiet_expr(m, *c, ctor),
            ExprKind::Array(xs) | ExprKind::Seq(xs) => {
                let xs = xs.clone();
                xs.into_iter().all(|x| self.quiet_expr(m, x, ctor))
            }
            ExprKind::Object(fs) => {
                let xs: Vec<ExprId> = fs.iter().map(|f| f.value).collect();
                xs.into_iter().all(|x| self.quiet_expr(m, x, ctor))
            }
            _ => true,
        }
    }
}

/// The expressions and statements directly inside statement `s`.
fn stmt_parts(ast: &crate::ast::Ast, s: StmtId) -> (Vec<ExprId>, Vec<StmtId>) {
    match &ast.stmt(s).kind {
        StmtKind::Let { init, .. } => (init.iter().copied().collect(), Vec::new()),
        StmtKind::Expr(e) | StmtKind::Throw(e) => (vec![*e], Vec::new()),
        StmtKind::Return(e) => (e.iter().copied().collect(), Vec::new()),
        StmtKind::If(c, t, e) => (vec![*c], std::iter::once(*t).chain(*e).collect()),
        StmtKind::While(c, b) | StmtKind::DoWhile(b, c) => (vec![*c], vec![*b]),
        StmtKind::For { init, cond, step, body } => (cond.iter().chain(step.iter()).copied().collect(), init.iter().copied().chain([*body]).collect()),
        StmtKind::ForOf { iter, body, .. } => (vec![*iter], vec![*body]),
        StmtKind::Switch(d, cases) => (std::iter::once(*d).chain(cases.iter().filter_map(|c| c.test)).collect(), cases.iter().flat_map(|c| c.body.iter().copied()).collect()),
        StmtKind::Block(ss) => (Vec::new(), ss.clone()),
        StmtKind::Try { body, catch, finally } => (Vec::new(), std::iter::once(*body).chain(catch.as_ref().map(|c| c.body)).chain(*finally).collect()),
        _ => (Vec::new(), Vec::new()),
    }
}

/// The expressions directly inside `e` (not an arrow's body).
fn expr_parts(ast: &crate::ast::Ast, e: ExprId) -> Vec<ExprId> {
    match &ast.expr(e).kind {
        ExprKind::Assign(_, t, v) => vec![*t, *v],
        ExprKind::Update { target, .. } => vec![*target],
        ExprKind::Call { callee, args, .. } => std::iter::once(*callee).chain(args.iter().map(|a| a.expr)).collect(),
        ExprKind::New { callee, args, .. } => std::iter::once(*callee).chain(args.iter().map(|a| a.expr)).collect(),
        ExprKind::Member { obj, .. } => vec![*obj],
        ExprKind::Index { obj, index, .. } => vec![*obj, *index],
        ExprKind::Unary(_, x) | ExprKind::Paren(x) | ExprKind::NonNull(x) | ExprKind::Typeof(x) | ExprKind::Spread(x) | ExprKind::As(x, _) | ExprKind::Try(x) | ExprKind::Await(x) | ExprKind::Yield(Some(x), _) => vec![*x],
        ExprKind::Binary(_, l, r) => vec![*l, *r],
        ExprKind::Cond(a, b, c) => vec![*a, *b, *c],
        ExprKind::Array(xs) | ExprKind::Template(_, xs) | ExprKind::Seq(xs) => xs.clone(),
        ExprKind::Object(fs) => fs.iter().map(|f| f.value).collect(),
        _ => Vec::new(),
    }
}

impl Gen<'_, '_> {
    /// `let x = <borrowed>` (a class instance, or `null`/`undefined`), in a quiet function whose
    /// every assignment to x gives it a field path from x itself or an unchanged parameter (or
    /// `null`): nothing can release the nodes x visits while it does, so it holds them borrowed.
    pub(crate) fn cursor_local(&mut self, key: u32, ty: TyId, init: ExprId) -> bool {
        let b = self.bodies.last().unwrap();
        if b.boxed.contains(&key) || b.async_fn.is_some() || b.gen_elem.is_some() || b.ctor_class.is_some() {
            return false;
        }
        let Some(root) = b.root else { return false };
        let class_like = matches!(self.tget(ty), Ty::Class(..)) || self.niche(ty).is_some_and(|(pi, _)| match self.tget(ty) {
            Ty::Union(ms) => matches!(self.tget(self.c.types.tys(ms)[pi]), Ty::Class(..)),
            _ => false,
        });
        if !class_like {
            return false;
        }
        let m = self.cur_m();
        if !self.borrowable(m, init, key) {
            return false;
        }
        let mut vals = Vec::new();
        let mut ok = true;
        self.assigned_to(m, None, Some(root), key, &mut vals, &mut ok);
        ok && vals.into_iter().all(|v| self.borrowable(m, v, key)) && self.quiet_body(m, root, false)
    }

    /// The values assigned to local `key` in `e` / `s` (`ok` false if it's changed otherwise).
    fn assigned_to(&self, m: u32, e: Option<ExprId>, s: Option<StmtId>, key: u32, vals: &mut Vec<ExprId>, ok: &mut bool) {
        let ast = self.ast(m);
        if let Some(s) = s {
            let (es, ss) = stmt_parts(ast, s);
            for x in es {
                self.assigned_to(m, Some(x), None, key, vals, ok);
            }
            for x in ss {
                self.assigned_to(m, None, Some(x), key, vals, ok);
            }
            return;
        }
        let Some(e) = e else { return };
        let is_key = |x: ExprId| matches!(self.ident_fact(m, x), Some(IdentFact::Local(k)) if k == key);
        match &ast.expr(e).kind {
            ExprKind::Assign(op, t, v) if is_key(*t) => {
                if *op != crate::ast::AssignOp::Assign {
                    *ok = false;
                }
                vals.push(*v);
            }
            ExprKind::Update { target, .. } if is_key(*target) => *ok = false,
            ExprKind::Call { args, .. } if args.iter().any(|a| a.by_ref.is_some() && is_key(a.expr)) => *ok = false,
            _ => {}
        }
        for x in expr_parts(ast, e) {
            self.assigned_to(m, Some(x), None, key, vals, ok);
        }
    }

    /// `e` reads a node borrowed (from `this`, an unchanged parameter, or cursor `key`), or is
    /// `null` / `undefined`.
    fn borrowable(&mut self, m: u32, e: ExprId, key: u32) -> bool {
        let ast = self.ast(m);
        match &ast.expr(e).kind {
            ExprKind::Null | ExprKind::Undefined | ExprKind::This => true,
            ExprKind::Paren(x) | ExprKind::NonNull(x) => self.borrowable(m, *x, key),
            ExprKind::Ident(_) => match self.ident_fact(m, e) {
                Some(IdentFact::Local(k)) => {
                    let b = self.bodies.last().unwrap();
                    k == key || b.params.contains(&k) && !b.mutated.contains(&k) && !b.boxed.contains(&k)
                }
                _ => false,
            },
            ExprKind::Member { obj, name, optional: false, .. } => {
                let ot = self.facts(m).expr_ty[*obj as usize];
                let ot = self.c.types.without_nullish(ot);
                self.c.class_of(ot).is_some() && matches!(self.c.class_member(ot, *name), Some(crate::check::class::ClassMemberRef::Field(f)) if !f.weak) && self.borrowable(m, *obj, key)
            }
            _ => false,
        }
    }
}
