//! Last uses: a local whose last use stores it somewhere (`node.next = x`, `return x`) hands
//! its reference over there instead of retaining a new one and releasing its own later.
//!
//! A use is the last when no other use of the local comes after it in the source and no loop
//! that doesn't also hold the declaration surrounds it: control can't come back to a use
//! before it. (The local is set to NULL as it's handed over, so its release at the end of its
//! scope, on any path, finds nothing.)

use super::Gen;
use crate::ast::{Ast, ExprId, ExprKind, StmtId, StmtKind};
use crate::check::IdentFact;
use crate::hash::{FxMap, FxSet};

#[derive(Default)]
struct Walk {
    /// Per local: the loops around its declaration.
    decl: FxMap<u32, Vec<StmtId>>,
    /// Per local: its uses (where, the loops around it, whether it's handed over there).
    uses: FxMap<u32, Vec<(u32, Vec<StmtId>, Option<ExprId>)>>,
    /// Locals used inside a closure.
    captured: FxSet<u32>,
    loops: Vec<StmtId>,
}

impl Gen<'_, '_> {
    /// The uses (identifier expressions) of the current function's locals that are their last
    /// and hand them over.
    pub(crate) fn last_use_moves(&self, m: u32, root: StmtId, params: &[u32]) -> FxSet<ExprId> {
        let mut w = Walk::default();
        // (parameters: declared at the top, outside every loop)
        for &k in params {
            w.decl.insert(k, Vec::new());
        }
        self.mv_stmt(m, root, &mut w);
        let mut out = FxSet::default();
        for (k, uses) in &w.uses {
            if w.captured.contains(k) {
                continue;
            }
            let Some(decl) = w.decl.get(k) else { continue };
            let Some(last) = uses.iter().max_by_key(|u| u.0) else { continue };
            if let (Some(e), true) = (last.2, &last.1 == decl) {
                out.insert(e);
            }
        }
        out
    }

    fn mv_stmt(&self, m: u32, s: StmtId, w: &mut Walk) {
        let ast: &Ast = self.ast(m);
        match &ast.stmt(s).kind {
            StmtKind::Let { name_span, init, .. } => {
                if let Some(i) = init {
                    self.mv_expr(m, *i, w, false);
                }
                w.decl.insert(name_span.start, w.loops.clone());
            }
            StmtKind::Expr(e) | StmtKind::Throw(e) => self.mv_expr(m, *e, w, false),
            StmtKind::Return(e) => {
                if let Some(e) = e {
                    self.mv_expr(m, *e, w, true);
                }
            }
            StmtKind::If(c, t, e) => {
                self.mv_expr(m, *c, w, false);
                self.mv_stmt(m, *t, w);
                if let Some(e) = e {
                    self.mv_stmt(m, *e, w);
                }
            }
            StmtKind::While(c, b) | StmtKind::DoWhile(b, c) => {
                w.loops.push(s);
                self.mv_expr(m, *c, w, false);
                self.mv_stmt(m, *b, w);
                w.loops.pop();
            }
            StmtKind::For { init, cond, step, body } => {
                if let Some(i) = init {
                    self.mv_stmt(m, *i, w);
                }
                w.loops.push(s);
                for x in cond.iter().chain(step.iter()) {
                    self.mv_expr(m, *x, w, false);
                }
                self.mv_stmt(m, *body, w);
                w.loops.pop();
            }
            StmtKind::ForOf { iter, body, .. } => {
                self.mv_expr(m, *iter, w, false);
                w.loops.push(s);
                self.mv_stmt(m, *body, w);
                w.loops.pop();
            }
            StmtKind::Switch(d, cases) => {
                self.mv_expr(m, *d, w, false);
                for c in cases {
                    if let Some(t) = c.test {
                        self.mv_expr(m, t, w, false);
                    }
                    for &b in &c.body {
                        self.mv_stmt(m, b, w);
                    }
                }
            }
            StmtKind::Block(ss) => {
                for &b in ss {
                    self.mv_stmt(m, b, w);
                }
            }
            StmtKind::Try { body, catch, finally } => {
                // (a `catch` or `finally` may run after anything in the body: no moves around one)
                w.loops.push(s);
                self.mv_stmt(m, *body, w);
                if let Some(c) = catch {
                    self.mv_stmt(m, c.body, w);
                }
                if let Some(f) = finally {
                    self.mv_stmt(m, *f, w);
                }
                w.loops.pop();
            }
            _ => {}
        }
    }

    fn mv_local(&self, m: u32, e: ExprId) -> Option<IdentFact> {
        match &self.ast(m).expr(e).kind {
            ExprKind::Paren(x) | ExprKind::NonNull(x) | ExprKind::As(x, _) => self.mv_local(m, *x),
            ExprKind::Ident(_) => self.ident_fact(m, e),
            _ => None,
        }
    }

    /// `handed`: `e` is a value stored or returned as it is.
    fn mv_expr(&self, m: u32, e: ExprId, w: &mut Walk, handed: bool) {
        let ast: &Ast = self.ast(m);
        match &ast.expr(e).kind {
            ExprKind::Ident(_) => {
                if let Some(IdentFact::Local(k)) = self.ident_fact(m, e) {
                    let at = ast.expr(e).span.start;
                    w.uses.entry(k).or_default().push((at, w.loops.clone(), handed.then_some(e)));
                }
            }
            // (`return t!`, `x as C`: the same reference, handed on)
            ExprKind::Paren(x) | ExprKind::NonNull(x) => self.mv_expr(m, *x, w, handed),
            ExprKind::As(x, _) if {
                let (a, b) = (self.facts(m).expr_ty[e as usize], self.facts(m).expr_ty[*x as usize]);
                matches!(self.tget(a), crate::types::Ty::Class(..)) && (matches!(self.tget(b), crate::types::Ty::Class(..)) || self.niche(b).is_some())
            } => self.mv_expr(m, *x, w, handed),
            ExprKind::Assign(op, t, v) => {
                // (the value first: `a.next = x` hands x over; `x = x.next` reads x before its
                // own store)
                let mut to_place = *op == crate::ast::AssignOp::Assign && !matches!(ast.expr(*t).kind, ExprKind::Ident(_));
                // (`x.f = x`: the place is reached through x, after the value)
                if to_place && let Some(IdentFact::Local(k)) = self.mv_local(m, *v) {
                    let mut ts = Vec::new();
                    super::body::collect_exprs_expr(ast, *t, &mut ts);
                    to_place = !ts.iter().any(|&x| matches!(self.ident_fact(m, x), Some(IdentFact::Local(j)) if j == k));
                }
                self.mv_expr(m, *v, w, to_place);
                self.mv_expr(m, *t, w, false);
            }
            ExprKind::Arrow(f) => {
                let mut inner = Walk::default();
                match f.body {
                    crate::ast::ArrowBody::Expr(x) => self.mv_expr(m, x, &mut inner, false),
                    crate::ast::ArrowBody::Block(s) => self.mv_stmt(m, s, &mut inner),
                }
                w.captured.extend(inner.uses.keys().copied());
                w.captured.extend(inner.captured);
            }
            _ => {
                for x in super::quiet::expr_parts(ast, e) {
                    self.mv_expr(m, x, w, false);
                }
            }
        }
    }
}
