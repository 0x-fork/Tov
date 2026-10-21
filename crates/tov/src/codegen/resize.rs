//! What can change an array's length while a loop runs. Arrays are shared (as in JavaScript), so
//! a loop that keeps an array's elements or length in locals (versioned loops, reserved pushes,
//! counted loops) must know that nothing else resizes it meanwhile: no push, pop, shift,
//! unshift, splice or appending store on another array that could be the same one, and no call
//! that might do so. Functions are judged by their bodies (and what they call); calls through
//! closures, class methods and JavaScript might do anything.

use super::*;

pub(crate) const RESIZING: &[&str] = &["push", "pop", "shift", "unshift", "splice"];

impl<'c, 'a> Gen<'c, 'a> {
    /// Something among `exprs` (of module `m`) may resize an array other than through the locals
    /// `own` (whose uses the caller checks itself).
    pub(crate) fn may_resize(&mut self, m: u32, exprs: &[ExprId], own: &[u32]) -> bool {
        let ast = self.ast(m);
        let fresh = self.fresh_locals(m, exprs);
        for &e in exprs {
            match &ast.expr(e).kind {
                ExprKind::Call { callee, .. } => {
                    let callee_fact = self.facts(m).calls.get(&e).map(|f| f.callee.clone());
                    match callee_fact {
                        Some(Callee::Method(recv)) => {
                            let ExprKind::Member { obj, name, .. } = &ast.expr(*callee).kind else { continue };
                            if !matches!(self.tget(recv), Ty::Array(_)) || !RESIZING.contains(&self.sym(*name)) {
                                continue;
                            }
                            match self.local_key(m, *obj) {
                                Some(k) if own.contains(&k) || fresh.contains(&k) => {}
                                _ => return true,
                            }
                        }
                        Some(Callee::Fn(fm, fi)) => {
                            if self.fn_may_resize(fm, fi) {
                                return true;
                            }
                        }
                        Some(Callee::Builtin { .. } | Callee::Matcher(_)) => {}
                        _ => return true,
                    }
                }
                ExprKind::New { .. } | ExprKind::Await(_) => {
                    if !matches!(self.facts(m).calls.get(&e).map(|f| &f.callee), Some(Callee::Builtin { .. }) | None) {
                        return true;
                    }
                    if matches!(ast.expr(e).kind, ExprKind::Await(_)) {
                        return true;
                    }
                }
                // `ys.length = n` shrinks an array: even one of `own` (its users count on the
                // length only growing)
                ExprKind::Assign(_, t, _) | ExprKind::Update { target: t, .. }
                    if matches!(&ast.expr(*t).kind, ExprKind::Member { obj, name, .. } if self.sym(*name) == "length" && self.is_array_expr(m, *obj) && !self.local_key(m, *obj).is_some_and(|k| fresh.contains(&k))) =>
                {
                    return true;
                }
                // `ys[i] = v` may append
                ExprKind::Assign(crate::ast::AssignOp::Assign, t, _) => {
                    if let ExprKind::Index { obj, .. } = ast.expr(*t).kind
                        && self.is_array_expr(m, obj)
                    {
                        match self.local_key(m, obj) {
                            Some(k) if own.contains(&k) || fresh.contains(&k) => {}
                            _ => return true,
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Module function (m, i) may resize an array it didn't make itself.
    pub(crate) fn fn_may_resize(&mut self, m: u32, i: u32) -> bool {
        if let Some(&r) = self.resizers.get(&(m, i)) {
            return r;
        }
        // (while it's being worked out: assume it may, for recursion)
        self.resizers.insert((m, i), true);
        let ast = self.ast(m);
        let ItemKind::Function(f) = &ast.items[i as usize].kind else { return true };
        if f.is_async {
            return true;
        }
        let mut exprs = Vec::new();
        body::collect_exprs_stmt_pub(ast, f.body, &mut exprs);
        let r = self.may_resize(m, &exprs, &[]);
        self.resizers.insert((m, i), r);
        r
    }

    /// The `const` locals of module `m` initialized with a new array (a literal, `new Array`, or
    /// `map`, `filter`, `slice`, `concat`, `flat`, `flatMap`, `Array.from`, `split` of something):
    /// no one else holds them.
    fn fresh_locals(&mut self, m: u32, _exprs: &[ExprId]) -> FxSet<u32> {
        if let Some(s) = self.fresh_arrays.get(&m) {
            return s.clone();
        }
        let ast = self.ast(m);
        let mut out = FxSet::default();
        for st in ast.stmts.iter() {
            let StmtKind::Let { mutable: false, name_span, init: Some(i), .. } = &st.kind else { continue };
            let fresh = match &ast.expr(*i).kind {
                ExprKind::Array(_) => true,
                ExprKind::New { callee, .. } => matches!(ast.expr(*callee).kind, ExprKind::Ident(s) if self.sym(s) == "Array"),
                ExprKind::Call { callee, .. } => match &ast.expr(*callee).kind {
                    ExprKind::Member { name, .. } => matches!(self.sym(*name), "map" | "filter" | "slice" | "concat" | "flat" | "flatMap" | "from" | "split"),
                    _ => false,
                },
                _ => false,
            };
            if fresh {
                out.insert(name_span.start);
            }
        }
        self.fresh_arrays.insert(m, out.clone());
        out
    }

    fn local_key(&self, m: u32, e: ExprId) -> Option<u32> {
        match (&self.ast(m).expr(e).kind, self.facts(m).idents.get(&e)) {
            (ExprKind::Ident(_), Some(IdentFact::Local(k))) => Some(*k),
            (ExprKind::Paren(x) | ExprKind::NonNull(x), _) => self.local_key(m, *x),
            _ => None,
        }
    }

    fn is_array_expr(&mut self, m: u32, e: ExprId) -> bool {
        let t = self.facts(m).expr_ty.get(e as usize).copied().unwrap_or(ERROR);
        let t = self.c.types.without_undefined(t);
        matches!(self.tget(t), Ty::Array(_))
    }
}
