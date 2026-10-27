//! A loop that is all JavaScript's to run: calls into npm packages and the numbers and booleans
//! around them, nothing of Tov's own (strings built, collections, closures). It runs as one
//! JavaScript function: one call into the engine for the whole loop rather than one or more each
//! round, and the engine's compilers see the loop with what it calls, as they would in Bun.

use super::{c_string, Gen, Val};
use crate::ast::{AssignOp, BinOp, ExprId, ExprKind, ItemKind, StmtId, StmtKind, UnOp};
use crate::check::{Callee, IdentFact};
use crate::hash::{FxMap, FxSet};
use crate::types::*;
use std::fmt::Write;

/// What's passed in: an outer local (by key), or a value evaluated before the loop.
enum Param {
    Local(u32),
    Expr(ExprId),
}

#[derive(Default)]
struct Lower {
    src: String,
    /// Locals declared in the loop.
    inner: FxSet<u32>,
    /// Outer locals the loop assigns: copied in, and back out when it ends.
    outs: Vec<u32>,
    params: Vec<Param>,
    /// An outer local's parameter, by key.
    param_of: FxMap<u32, usize>,
    /// JavaScript operations (calls, `new`, property reads, objects made).
    ops: usize,
    /// Inside a JavaScript operation's operands (part of its tree, not one of its own).
    in_js: usize,
    /// Integer counters the loop steps by constants, exact because of the check on entry (see
    /// `jl_guards`).
    guarded: FxSet<u32>,
}

/// What `js_loop` emitted.
#[derive(PartialEq, Eq)]
pub(crate) enum JsLoop {
    /// Nothing: the loop is native code's.
    No,
    /// The whole loop.
    Done,
    /// The loop when its counters' values on entry let it run in JavaScript (an `if` left open
    /// for the caller's native loop, then `}`).
    Guarded,
}

/// A Tov value JavaScript holds as the same value: a number, a boolean, or a JavaScript value.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Num,
    Bool,
    Js,
}

impl<'c, 'a> Gen<'c, 'a> {
    /// The loop `s` as one JavaScript function, if all of it is JavaScript's to run (nothing
    /// emitted otherwise).
    pub(crate) fn js_loop(&mut self, s: StmtId) -> JsLoop {
        let m = self.cur_m();
        let ast = self.ast(m);
        if !matches!(ast.stmt(s).kind, StmtKind::For { .. } | StmtKind::While(..) | StmtKind::DoWhile(..)) {
            return JsLoop::No;
        }
        // (an error leaves the function, so what the loop changed is never seen half done)
        if !self.b().handlers.is_empty() || ast.labels.iter().any(|(st, _)| *st == s) {
            return JsLoop::No;
        }
        let mut lw = Lower::default();
        let mut assigned = FxSet::default();
        if !self.jl_scan(s, &mut lw.inner, &mut assigned) {
            return JsLoop::No;
        }
        let mut outs: Vec<u32> = assigned.difference(&lw.inner).copied().collect();
        outs.sort_unstable();
        for &k in &outs {
            // (an outer local the loop changes: a number or boolean nothing else holds)
            if self.b().boxed.contains(&k) {
                return JsLoop::No;
            }
            let Some(l) = self.b().locals.get(&k) else { return JsLoop::No };
            let t = l.ty;
            if !matches!(self.tget(t), Ty::Int | Ty::F64 | Ty::Bool) {
                return JsLoop::No;
            }
        }
        let Some(guards) = self.jl_guards(s, &outs) else { return JsLoop::No };
        lw.guarded = guards.iter().map(|g| g.0).collect();
        for &k in &outs {
            let p = lw.params.len();
            lw.params.push(Param::Local(k));
            let _ = write!(lw.src, "let v{k} = a{p}; ");
        }
        lw.outs = outs;
        if !self.jl_stmt(s, &mut lw) || lw.ops == 0 {
            return JsLoop::No;
        }
        match lw.outs.len() {
            0 => {}
            1 => {
                let _ = write!(lw.src, " return v{};", lw.outs[0]);
            }
            _ => {
                let names: Vec<String> = lw.outs.iter().map(|k| format!("v{k}")).collect();
                let _ = write!(lw.src, " return [{}];", names.join(", "));
            }
        }
        self.js_used();
        if !guards.is_empty() {
            let checks: Vec<String> = guards.iter().map(|&(k, b)| format!("{0} >= -INT64_C({b}) && {0} <= INT64_C({b})", self.b().locals[&k].access)).collect();
            self.open(&format!("if ({}) {{", checks.join(" && ")));
        }
        self.push_temps();
        let mut argv = Vec::new();
        for p in std::mem::take(&mut lw.params) {
            let v = match p {
                Param::Local(k) => {
                    let l = self.b().locals[&k].clone();
                    Val::plain(l.access, l.ty)
                }
                Param::Expr(e) => self.expr(e),
            };
            let j = self.js_of(v);
            argv.push(j.code);
        }
        let slot = self.fresh("bmgjl");
        let _ = writeln!(self.lits, "static JSObjectRef {slot};");
        let argv_c = if argv.is_empty() { "NULL".to_string() } else { format!("(JSValueRef[]){{{}}}", argv.join(", ")) };
        let (x, r) = (self.fresh("jx"), self.fresh("jr"));
        let src = c_string(lw.src.as_bytes());
        self.line(format!("JSValueRef {x} = NULL; JSValueRef {r} = tv_js_thunk(&{slot}, {src}, {}, {argv_c}, &{x});", argv.len()));
        self.open(&format!("if (!{r}) tvg_err = tvg_js_make_error({x}); else {{"));
        for (i, &k) in lw.outs.iter().enumerate() {
            let l = self.b().locals[&k].clone();
            let v = if lw.outs.len() == 1 {
                r.clone()
            } else {
                let (ex, ev) = (self.fresh("jx"), self.fresh("j"));
                self.line(format!("JSValueRef {ex} = NULL; JSValueRef {ev} = tv_js_index({r}, {i}, &{ex});"));
                ev
            };
            match self.tget(l.ty) {
                Ty::Bool => self.line(format!("{} = tv_js_truthy({v});", l.access)),
                Ty::Int => {
                    let d = self.fresh("d");
                    self.line(format!("double {d} = 0; tv_js_as_num({v}, &{d}); {} = (tv_int){d};", l.access));
                }
                _ => {
                    let d = self.fresh("d");
                    self.line(format!("double {d} = 0; tv_js_as_num({v}, &{d}); {} = {d};", l.access));
                }
            }
        }
        self.close("}");
        self.pop_temps();
        self.error_check();
        if guards.is_empty() {
            return JsLoop::Done;
        }
        self.close("}");
        self.open("else {");
        JsLoop::Guarded
    }

    /// The `int` counters of the loop's that only step by constants (`ok++`, `n += 2`) and that
    /// range analysis can't bound, with how far from zero each may start for JavaScript's numbers
    /// to hold every value it takes exactly. A counted loop's rounds are bounded by its
    /// variable's range: at most K × rounds of steps, K what a round's steps add up to (none in
    /// a loop nested in it). `None` if a counter isn't one of those; empty if there are none.
    fn jl_guards(&mut self, s: StmtId, outs: &[u32]) -> Option<Vec<(u32, i128)>> {
        let m = self.cur_m();
        let ast = self.ast(m);
        let mut all = Vec::new();
        super::body::collect_exprs_stmt_pub(ast, s, &mut all);
        let occurs = |g: &mut Self, k: u32| all.iter().copied().find(|&e| matches!(ast.expr(e).kind, ExprKind::Ident(_)) && g.ident_fact(m, e) == Some(IdentFact::Local(k)));
        let mut want = Vec::new();
        for &k in outs {
            if self.b().locals.get(&k).is_none_or(|l| l.ty != INT) {
                continue;
            }
            let Some(e) = occurs(self, k) else { continue };
            if !matches!(self.int_range(e), super::range::R::In(lo, hi) if Self::jl_exact_int(lo, hi)) {
                want.push(k);
            }
        }
        if want.is_empty() {
            return Some(Vec::new());
        }
        // (the rounds: `for (let i = a; i < n; i++)`, nothing else changing i)
        let StmtKind::For { init: Some(init), cond: Some(cond), step: Some(step), body } = &ast.stmt(s).kind else { return None };
        let (init, cond, step, body) = (*init, *cond, *step, *body);
        let StmtKind::Let { name_span, .. } = &ast.stmt(init).kind else { return None };
        let ik = name_span.start;
        let is_i = |g: &mut Self, e: ExprId| matches!(ast.expr(e).kind, ExprKind::Ident(_)) && g.ident_fact(m, e) == Some(IdentFact::Local(ik));
        let iv = match &ast.expr(cond).kind {
            ExprKind::Binary(BinOp::Lt | BinOp::Le, l, _) if is_i(self, *l) => *l,
            _ => return None,
        };
        let steps_up = match &ast.expr(step).kind {
            ExprKind::Update { target, inc: true, .. } => is_i(self, *target),
            ExprKind::Assign(AssignOp::Op(BinOp::Add), t, v) => is_i(self, *t) && matches!(ast.expr(*v).kind, ExprKind::Int(1)),
            _ => false,
        };
        if !steps_up {
            return None;
        }
        let mut inner = Vec::new();
        super::body::collect_exprs_stmt_pub(ast, body, &mut inner);
        super::body::collect_exprs_expr(ast, cond, &mut inner);
        for &e in &inner {
            let t = match &ast.expr(e).kind {
                ExprKind::Assign(_, t, _) | ExprKind::Update { target: t, .. } => *t,
                _ => continue,
            };
            if is_i(self, t) {
                return None;
            }
        }
        let super::range::R::In(lo, hi) = self.int_range(iv) else { return None };
        let rounds = hi - lo + 1;
        // (what's in a loop nested in this one runs any number of times a round)
        let mut nested = Vec::new();
        jl_nested(ast, body, &mut nested);
        let nested: FxSet<ExprId> = nested.into_iter().collect();
        let mut guards = Vec::new();
        for k in want {
            let mut per_round: i128 = 0;
            for &e in &all {
                let (t, by) = match &ast.expr(e).kind {
                    ExprKind::Update { target, .. } => (*target, Some(1)),
                    ExprKind::Assign(AssignOp::Op(BinOp::Add | BinOp::Sub), t, v) => match ast.expr(*v).kind {
                        ExprKind::Int(c) if c < (1 << 32) => (*t, Some(c as i128)),
                        _ => (*t, None),
                    },
                    ExprKind::Assign(_, t, _) => (*t, None),
                    _ => continue,
                };
                if self.ident_fact(m, t) != Some(IdentFact::Local(k)) {
                    continue;
                }
                let by = by.filter(|_| !nested.contains(&e))?;
                per_round += by;
            }
            const LIMIT: i128 = (1 << 53) - 1;
            let reach = per_round.checked_mul(rounds)?;
            if reach > LIMIT / 2 {
                return None;
            }
            guards.push((k, LIMIT - reach));
        }
        Some(guards)
    }

    /// The locals the loop declares and assigns; false if it has a statement JavaScript can't run.
    fn jl_scan(&mut self, s: StmtId, inner: &mut FxSet<u32>, assigned: &mut FxSet<u32>) -> bool {
        let m = self.cur_m();
        let ast = self.ast(m);
        let mut exprs = Vec::new();
        match &ast.stmt(s).kind {
            StmtKind::Empty | StmtKind::Break(None) | StmtKind::Continue(None) => {}
            StmtKind::Let { name_span, init: Some(init), .. } => {
                inner.insert(name_span.start);
                exprs.push(*init);
            }
            StmtKind::Expr(e) => exprs.push(*e),
            StmtKind::If(c, t, e) => {
                exprs.push(*c);
                if !self.jl_scan(*t, inner, assigned) || e.is_some_and(|e| !self.jl_scan(e, inner, assigned)) {
                    return false;
                }
            }
            StmtKind::While(c, b) | StmtKind::DoWhile(b, c) => {
                exprs.push(*c);
                if !self.jl_scan(*b, inner, assigned) {
                    return false;
                }
            }
            StmtKind::For { init, cond, step, body } => {
                exprs.extend(cond.iter().chain(step.iter()).copied());
                if init.is_some_and(|i| !self.jl_scan(i, inner, assigned)) || !self.jl_scan(*body, inner, assigned) {
                    return false;
                }
            }
            StmtKind::Block(ss) => {
                for &x in ss {
                    if !self.jl_scan(x, inner, assigned) {
                        return false;
                    }
                }
            }
            _ => return false,
        }
        let mut all = Vec::new();
        for e in exprs {
            super::body::collect_exprs_expr(ast, e, &mut all);
        }
        for e in all {
            let target = match &ast.expr(e).kind {
                ExprKind::Assign(_, t, _) => *t,
                ExprKind::Update { target, .. } => *target,
                _ => continue,
            };
            match (&ast.expr(target).kind, self.ident_fact(m, target)) {
                (ExprKind::Ident(_), Some(IdentFact::Local(k))) => {
                    assigned.insert(k);
                }
                _ => return false,
            }
        }
        true
    }

    fn jl_kind(&mut self, t: TyId) -> Option<Kind> {
        match self.tget(t) {
            Ty::Int | Ty::F64 => Some(Kind::Num),
            Ty::Bool | Ty::BoolLit(_) => Some(Kind::Bool),
            Ty::Js => Some(Kind::Js),
            _ => None,
        }
    }

    fn jl_stmt(&mut self, s: StmtId, lw: &mut Lower) -> bool {
        let m = self.cur_m();
        let ast = self.ast(m);
        match &ast.stmt(s).kind {
            StmtKind::Empty => true,
            StmtKind::Break(None) => {
                lw.src.push_str("break; ");
                true
            }
            StmtKind::Continue(None) => {
                lw.src.push_str("continue; ");
                true
            }
            StmtKind::Block(ss) => {
                lw.src.push_str("{ ");
                for &x in ss {
                    if !self.jl_stmt(x, lw) {
                        return false;
                    }
                }
                lw.src.push_str("} ");
                true
            }
            StmtKind::Let { mutable, name_span, init: Some(init), .. } => {
                let (k, init, mutable) = (name_span.start, *init, *mutable);
                let Some(&bt) = self.facts(m).bindings.get(&s) else { return false };
                let bt = self.inst(bt);
                let it = self.ty(init);
                if self.b().boxed.contains(&k) || self.jl_kind(bt).is_none() || self.jl_kind(bt) != self.jl_kind(it) {
                    return false;
                }
                let _ = write!(lw.src, "{} v{k} = ", if mutable { "let" } else { "const" });
                if !self.jl_expr(init, lw) {
                    return false;
                }
                lw.src.push_str("; ");
                true
            }
            StmtKind::Expr(e) => {
                let e = *e;
                if !self.jl_effect(e, lw) {
                    return false;
                }
                lw.src.push_str("; ");
                true
            }
            StmtKind::If(c, t, e) => {
                let (c, t, e) = (*c, *t, *e);
                lw.src.push_str("if (");
                if !self.jl_cond(c, lw) {
                    return false;
                }
                lw.src.push_str(") ");
                if !self.jl_stmt(t, lw) {
                    return false;
                }
                if let Some(e) = e {
                    lw.src.push_str("else ");
                    if !self.jl_stmt(e, lw) {
                        return false;
                    }
                }
                true
            }
            StmtKind::While(c, b) => {
                let (c, b) = (*c, *b);
                lw.src.push_str("while (");
                if !self.jl_cond(c, lw) {
                    return false;
                }
                lw.src.push_str(") ");
                self.jl_stmt(b, lw)
            }
            StmtKind::DoWhile(b, c) => {
                let (c, b) = (*c, *b);
                lw.src.push_str("do ");
                if !self.jl_stmt(b, lw) {
                    return false;
                }
                lw.src.push_str("while (");
                if !self.jl_cond(c, lw) {
                    return false;
                }
                lw.src.push_str("); ");
                true
            }
            StmtKind::For { init, cond, step, body } => {
                let (init, cond, step, body) = (*init, *cond, *step, *body);
                lw.src.push_str("{ ");
                if let Some(i) = init
                    && !self.jl_stmt(i, lw)
                {
                    return false;
                }
                lw.src.push_str("for (; ");
                if let Some(c) = cond
                    && !self.jl_cond(c, lw)
                {
                    return false;
                }
                lw.src.push_str("; ");
                if let Some(st) = step
                    && !self.jl_effect(st, lw)
                {
                    return false;
                }
                lw.src.push_str(") ");
                if !self.jl_stmt(body, lw) {
                    return false;
                }
                lw.src.push_str("} ");
                true
            }
            _ => false,
        }
    }

    /// A condition: a boolean, number or JavaScript value, tested as JavaScript tests it.
    fn jl_cond(&mut self, e: ExprId, lw: &mut Lower) -> bool {
        let t = self.ty(e);
        self.jl_kind(t).is_some() && self.jl_expr(e, lw)
    }

    /// An expression statement (or a `for` step): an assignment to a local, or a JavaScript
    /// operation run for what it does.
    fn jl_effect(&mut self, e: ExprId, lw: &mut Lower) -> bool {
        let m = self.cur_m();
        let ast = self.ast(m);
        match &ast.expr(e).kind {
            ExprKind::Paren(x) => self.jl_effect(*x, lw),
            ExprKind::Seq(xs) => {
                for (i, &x) in xs.clone().iter().enumerate() {
                    if i > 0 {
                        lw.src.push_str(", ");
                    }
                    if !self.jl_effect(x, lw) {
                        return false;
                    }
                }
                true
            }
            ExprKind::Update { inc, target, .. } => {
                let (inc, target) = (*inc, *target);
                let tt = self.ty(target);
                if tt == INT && !self.jl_guarded(target, lw) {
                    let super::range::R::In(lo, hi) = self.int_range(target) else { return false };
                    let step = if inc { 1 } else { -1 };
                    if !Self::jl_exact_int(lo, hi) || !Self::jl_exact_int(lo + step, hi + step) {
                        return false;
                    }
                }
                if self.jl_kind(tt) != Some(Kind::Num) || !self.jl_local(target, lw) {
                    return false;
                }
                lw.src.push_str(if inc { "++" } else { "--" });
                true
            }
            ExprKind::Assign(op, target, value) => {
                let (op, target, value) = (*op, *target, *value);
                let (tt, vt) = (self.ty(target), self.ty(value));
                let (tk, vk) = (self.jl_kind(tt), self.jl_kind(vt));
                // (a guarded counter's steps were checked by `jl_guards`)
                let stepped = self.jl_guarded(target, lw) && matches!(op, AssignOp::Op(BinOp::Add | BinOp::Sub)) && matches!(ast.expr(value).kind, ExprKind::Int(_));
                if tt == INT && !stepped && !self.jl_int_assign(op, target, value) {
                    return false;
                }
                let ok = match op {
                    AssignOp::Assign => tk.is_some() && tk == vk,
                    AssignOp::Op(BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Rem) => tk == Some(Kind::Num) && vk == Some(Kind::Num),
                    AssignOp::Op(BinOp::Div) => self.tget(tt) == Ty::F64 && vk == Some(Kind::Num),
                    _ => false,
                };
                if !ok || !self.jl_local(target, lw) {
                    return false;
                }
                lw.src.push_str(match op {
                    AssignOp::Assign => " = ",
                    AssignOp::Op(BinOp::Add) => " += ",
                    AssignOp::Op(BinOp::Sub) => " -= ",
                    AssignOp::Op(BinOp::Mul) => " *= ",
                    AssignOp::Op(BinOp::Div) => " /= ",
                    _ => " %= ",
                });
                self.jl_expr(value, lw)
            }
            _ => {
                // (a JavaScript operation's result unused; nothing else is worth running)
                let mut x = e;
                while let ExprKind::Paren(y) | ExprKind::Try(y) = &ast.expr(x).kind {
                    x = *y;
                }
                self.jl_is_op(x) && self.jl_expr(e, lw)
            }
        }
    }

    /// An assignment's target: a local declared in the loop, or one copied in and out.
    fn jl_local(&mut self, e: ExprId, lw: &mut Lower) -> bool {
        let m = self.cur_m();
        match self.ident_fact(m, e) {
            Some(IdentFact::Local(k)) if lw.inner.contains(&k) || lw.outs.contains(&k) => {
                let _ = write!(lw.src, "v{k}");
                true
            }
            _ => false,
        }
    }

    /// `e` a JavaScript operation (on a JavaScript value, or making one from a literal).
    fn jl_is_op(&mut self, e: ExprId) -> bool {
        let m = self.cur_m();
        let ast = self.ast(m);
        match &ast.expr(e).kind {
            ExprKind::Member { obj, .. } | ExprKind::Index { obj, .. } => {
                let t = self.ty(*obj);
                t == JS
            }
            ExprKind::Call { .. } => matches!(self.facts(m).calls.get(&e).map(|c| &c.callee), Some(Callee::Js { .. })),
            ExprKind::New { .. } => matches!(self.facts(m).calls.get(&e).map(|c| &c.callee), Some(Callee::JsNew)),
            _ => false,
        }
    }

    fn jl_expr(&mut self, e: ExprId, lw: &mut Lower) -> bool {
        let int = self.ty(e) == INT;
        if int && !self.jl_guarded(e, lw) && !matches!(self.int_range(e), super::range::R::In(lo, hi) if Self::jl_exact_int(lo, hi)) {
            return false;
        }
        let native_int = int && !self.js_ints(self.cur_m());
        // Native integers have only one zero. JavaScript's negation, product and remainder can
        // produce -0, which must become +0 before an npm call or property access can observe it.
        if native_int {
            lw.src.push_str("(0 + (");
        }
        let ok = self.jl_expr_inner(e, lw);
        if native_int {
            lw.src.push_str("))");
        }
        ok
    }

    /// `e` names a counter exact because of the check on entry.
    fn jl_guarded(&mut self, e: ExprId, lw: &Lower) -> bool {
        let m = self.cur_m();
        matches!(self.ast(m).expr(e).kind, ExprKind::Ident(_)) && matches!(self.ident_fact(m, e), Some(IdentFact::Local(k)) if lw.guarded.contains(&k))
    }

    fn jl_exact_int(lo: i128, hi: i128) -> bool {
        const LIMIT: i128 = (1 << 53) - 1;
        lo >= -LIMIT && hi <= LIMIT
    }

    /// Integer assignments still have Tov's precision and trap behavior. If the existing range
    /// analysis can't prove the operands and result exact as JavaScript numbers, keep the loop
    /// in native code, including its overflow and zero-divisor checks.
    fn jl_int_assign(&mut self, op: AssignOp, target: ExprId, value: ExprId) -> bool {
        let (super::range::R::In(a, b), super::range::R::In(c, d)) = (self.int_range(target), self.int_range(value)) else { return false };
        if !Self::jl_exact_int(a, b) || !Self::jl_exact_int(c, d) {
            return false;
        }
        match op {
            AssignOp::Assign => true,
            AssignOp::Op(BinOp::Add) => Self::jl_exact_int(a + c, b + d),
            AssignOp::Op(BinOp::Sub) => Self::jl_exact_int(a - d, b - c),
            AssignOp::Op(BinOp::Mul) => [a * c, a * d, b * c, b * d].iter().all(|&n| Self::jl_exact_int(n, n)),
            AssignOp::Op(BinOp::Rem) => c > 0 || d < 0,
            _ => false,
        }
    }

    fn jl_expr_inner(&mut self, e: ExprId, lw: &mut Lower) -> bool {
        let m = self.cur_m();
        let ast = self.ast(m);
        let t = self.ty(e);
        if self.jl_is_op(e) {
            // (the tree's root: its error is one that passes on, as it would from Tov's code)
            if self.ty(e) != JS || (lw.in_js == 0 && !self.facts(m).throwing.contains(&e)) {
                return false;
            }
            lw.ops += 1;
            lw.in_js += 1;
            let ok = self.jl_op(e, lw);
            lw.in_js -= 1;
            return ok;
        }
        match &ast.expr(e).kind {
            ExprKind::Paren(x) => {
                let x = *x;
                lw.src.push('(');
                let ok = self.jl_expr(x, lw);
                lw.src.push(')');
                ok
            }
            ExprKind::Try(x) => self.jl_expr(*x, lw),
            ExprKind::Int(v) if *v < (1u64 << 53) => {
                let _ = write!(lw.src, "{v}");
                true
            }
            ExprKind::Float(x) if x.is_finite() => {
                let _ = write!(lw.src, "{x:?}");
                true
            }
            ExprKind::Bool(b) => {
                lw.src.push_str(if *b { "true" } else { "false" });
                true
            }
            ExprKind::Undefined if lw.in_js > 0 => {
                lw.src.push_str("undefined");
                true
            }
            ExprKind::Null if lw.in_js > 0 => {
                lw.src.push_str("null");
                true
            }
            ExprKind::Str(s) if lw.in_js > 0 && matches!(self.tget(t), Ty::Str | Ty::StrLit(_) | Ty::Js) => {
                let text = crate::npm::bundle::js_string(self.sym(*s));
                lw.src.push_str(&text);
                true
            }
            // (an object literal made into a JavaScript object, as a fused call makes it)
            ExprKind::Object(fields) if lw.in_js > 0 && matches!(self.tget(t), Ty::Record(_)) => {
                let fields: Vec<_> = fields.iter().map(|f| (f.name, f.value)).collect();
                lw.ops += 1;
                lw.src.push('{');
                for (i, (name, value)) in fields.into_iter().enumerate() {
                    if i > 0 {
                        lw.src.push_str(", ");
                    }
                    let key = crate::npm::bundle::js_string(self.sym(name));
                    let _ = write!(lw.src, "{key}: ");
                    if !self.jl_expr(value, lw) {
                        return false;
                    }
                }
                lw.src.push('}');
                true
            }
            ExprKind::Ident(_) => self.jl_ident(e, t, lw),
            ExprKind::Unary(op, x) => {
                let (op, x) = (*op, *x);
                let xt = self.ty(x);
                let xk = self.jl_kind(xt);
                match op {
                    UnOp::Not if xk.is_some() => lw.src.push_str("!("),
                    UnOp::Neg if xk == Some(Kind::Num) => lw.src.push_str("(-"),
                    _ => return false,
                }
                let ok = self.jl_expr(x, lw);
                lw.src.push(')');
                ok
            }
            ExprKind::Binary(op, a, b) => {
                let (op, a, b) = (*op, *a, *b);
                let (at, bt) = (self.ty(a), self.ty(b));
                let (ak, bk) = (self.jl_kind(at), self.jl_kind(bt));
                let num = ak == Some(Kind::Num) && bk == Some(Kind::Num);
                let text = match op {
                    BinOp::Add if num => "+",
                    BinOp::Sub if num => "-",
                    BinOp::Mul if num => "*",
                    BinOp::Rem if num => "%",
                    BinOp::Div if num && self.tget(t) == Ty::F64 => "/",
                    BinOp::Lt if num => "<",
                    BinOp::Gt if num => ">",
                    BinOp::Le if num => "<=",
                    BinOp::Ge if num => ">=",
                    BinOp::Eq if ak == bk && matches!(ak, Some(Kind::Num | Kind::Bool)) => "===",
                    BinOp::Ne if ak == bk && matches!(ak, Some(Kind::Num | Kind::Bool)) => "!==",
                    BinOp::And if ak == Some(Kind::Bool) && bk == Some(Kind::Bool) => "&&",
                    BinOp::Or if ak == Some(Kind::Bool) && bk == Some(Kind::Bool) => "||",
                    _ => return false,
                };
                lw.src.push('(');
                if !self.jl_expr(a, lw) {
                    return false;
                }
                let _ = write!(lw.src, " {text} ");
                if !self.jl_expr(b, lw) {
                    return false;
                }
                lw.src.push(')');
                true
            }
            ExprKind::Cond(c, a, b) => {
                let (c, a, b) = (*c, *a, *b);
                let (at, bt) = (self.ty(a), self.ty(b));
                let k = self.jl_kind(t);
                if !matches!(k, Some(Kind::Num | Kind::Bool)) || self.jl_kind(at) != k || self.jl_kind(bt) != k {
                    return false;
                }
                lw.src.push('(');
                if !self.jl_cond(c, lw) {
                    return false;
                }
                lw.src.push_str(" ? ");
                if !self.jl_expr(a, lw) {
                    return false;
                }
                lw.src.push_str(" : ");
                if !self.jl_expr(b, lw) {
                    return false;
                }
                lw.src.push(')');
                true
            }
            _ => false,
        }
    }

    /// A name: a local of the loop's, one copied in and out, or a value the loop only reads
    /// (passed in once: nothing in the loop can change it).
    fn jl_ident(&mut self, e: ExprId, t: TyId, lw: &mut Lower) -> bool {
        let m = self.cur_m();
        match self.ident_fact(m, e) {
            Some(IdentFact::Local(k)) if lw.inner.contains(&k) || lw.outs.contains(&k) => {
                let _ = write!(lw.src, "v{k}");
                true
            }
            Some(IdentFact::Local(k)) => {
                let Some(l) = self.b().locals.get(&k) else { return false };
                let lt = l.ty;
                if !matches!(self.tget(lt), Ty::Int | Ty::F64 | Ty::Bool | Ty::Js | Ty::Str) {
                    return false;
                }
                let p = match lw.param_of.get(&k) {
                    Some(&p) => p,
                    None => {
                        let p = lw.params.len();
                        lw.params.push(Param::Local(k));
                        lw.param_of.insert(k, p);
                        p
                    }
                };
                let _ = write!(lw.src, "a{p}");
                true
            }
            Some(IdentFact::Npm(..)) => {
                let _ = write!(lw.src, "a{}", lw.params.len());
                lw.params.push(Param::Expr(e));
                true
            }
            Some(IdentFact::Const(cm, item)) => {
                let fixed = matches!(&self.ast(cm).items[item as usize].kind, ItemKind::Const { mutable: false, .. });
                if !fixed || !matches!(self.tget(t), Ty::Int | Ty::F64 | Ty::Bool | Ty::Js | Ty::Str) {
                    return false;
                }
                let _ = write!(lw.src, "a{}", lw.params.len());
                lw.params.push(Param::Expr(e));
                true
            }
            _ => false,
        }
    }

    /// A JavaScript operation, its operands part of the same tree.
    fn jl_op(&mut self, e: ExprId, lw: &mut Lower) -> bool {
        let m = self.cur_m();
        let ast = self.ast(m);
        match &ast.expr(e).kind {
            ExprKind::Member { obj, name, optional, .. } => {
                let (obj, name, optional) = (*obj, *name, *optional);
                if !self.jl_operand(obj, lw) {
                    return false;
                }
                let text = self.sym(name).to_string();
                super::js::js_access(&mut lw.src, &text, optional);
                true
            }
            ExprKind::Index { obj, index, optional } => {
                let (obj, index, optional) = (*obj, *index, *optional);
                if !self.jl_operand(obj, lw) {
                    return false;
                }
                lw.src.push_str(if optional { "?.[" } else { "[" });
                if !self.jl_expr(index, lw) {
                    return false;
                }
                lw.src.push(']');
                true
            }
            ExprKind::Call { callee, args, optional, .. } => {
                let Some(Callee::Js { method }) = self.facts(m).calls.get(&e).map(|c| c.callee.clone()) else { return false };
                let (callee, optional) = (*callee, *optional);
                if args.iter().any(|a| a.by_ref.is_some() || matches!(ast.expr(a.expr).kind, ExprKind::Spread(_))) {
                    return false;
                }
                let args: Vec<ExprId> = args.iter().map(|a| a.expr).collect();
                match (&ast.expr(callee).kind, method) {
                    (ExprKind::Member { obj, name, optional: mopt, .. }, Some(_)) => {
                        let (obj, name, mopt) = (*obj, *name, *mopt);
                        if !self.jl_operand(obj, lw) {
                            return false;
                        }
                        let text = self.sym(name).to_string();
                        super::js::js_access(&mut lw.src, &text, mopt);
                    }
                    _ => {
                        if !self.jl_operand(callee, lw) {
                            return false;
                        }
                    }
                }
                lw.src.push_str(if optional { "?.(" } else { "(" });
                for (i, a) in args.into_iter().enumerate() {
                    if i > 0 {
                        lw.src.push_str(", ");
                    }
                    if !self.jl_expr(a, lw) {
                        return false;
                    }
                }
                lw.src.push(')');
                true
            }
            ExprKind::New { callee, args, .. } => {
                let callee = *callee;
                if args.iter().any(|a| a.by_ref.is_some() || matches!(ast.expr(a.expr).kind, ExprKind::Spread(_))) {
                    return false;
                }
                let args: Vec<ExprId> = args.iter().map(|a| a.expr).collect();
                lw.src.push_str("new (");
                if !self.jl_expr(callee, lw) {
                    return false;
                }
                lw.src.push_str(")(");
                for (i, a) in args.into_iter().enumerate() {
                    if i > 0 {
                        lw.src.push_str(", ");
                    }
                    if !self.jl_expr(a, lw) {
                        return false;
                    }
                }
                lw.src.push(')');
                true
            }
            _ => false,
        }
    }

    /// A JavaScript operation's object: a name as it is, anything else in parentheses.
    fn jl_operand(&mut self, e: ExprId, lw: &mut Lower) -> bool {
        let m = self.cur_m();
        if matches!(self.ast(m).expr(e).kind, ExprKind::Ident(_)) {
            return self.jl_expr(e, lw);
        }
        lw.src.push('(');
        let ok = self.jl_expr(e, lw);
        lw.src.push(')');
        ok
    }
}

/// The expressions of the loops nested in `s` (a loop's body).
fn jl_nested(ast: &crate::ast::Ast, s: StmtId, out: &mut Vec<ExprId>) {
    match &ast.stmt(s).kind {
        StmtKind::Block(ss) => {
            for &x in ss {
                jl_nested(ast, x, out);
            }
        }
        StmtKind::If(_, t, e) => {
            jl_nested(ast, *t, out);
            if let Some(e) = e {
                jl_nested(ast, *e, out);
            }
        }
        StmtKind::While(..) | StmtKind::DoWhile(..) | StmtKind::For { .. } | StmtKind::ForOf { .. } => super::body::collect_exprs_stmt_pub(ast, s, out),
        _ => {}
    }
}
