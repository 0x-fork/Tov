//! Definite assignment, as TypeScript's "used before being assigned": where a `let x: T;`
//! (no value, and `T` without `undefined`) may be read before anything is assigned to it.

use crate::ast::{AssignOp, Ast, BinOp, ExprId, ExprKind, StmtId, StmtKind};
use crate::intern::Sym;
use crate::source::Span;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Unassigned,
    Assigned,
    /// Unreachable (after a `return`, `throw`, `break` or `continue`).
    Dead,
}

fn join(a: State, b: State) -> State {
    match (a, b) {
        (State::Dead, x) | (x, State::Dead) => x,
        (State::Assigned, State::Assigned) => State::Assigned,
        _ => State::Unassigned,
    }
}

/// Where `x` is jumped out of: what's assigned at each `break` and `continue`.
struct Target {
    breaks: State,
    continues: State,
    is_loop: bool,
}

struct Da<'a> {
    ast: &'a Ast,
    name: Sym,
    reads: Vec<Span>,
    targets: Vec<Target>,
    /// Inside a declaration that hides `x` (a block's own `let x`, a loop's or a `catch`'s).
    hidden: u32,
    /// A labelled `break`/`continue` was seen: where it goes isn't tracked, so nothing is reported.
    gave_up: bool,
}

/// The reads of `name` in `stmts` (which follow its declaration) that may come before it's
/// assigned. Reads inside functions aren't counted (they may run later), nor are assignments.
pub(super) fn unassigned_reads(ast: &Ast, name: Sym, stmts: &[StmtId]) -> Vec<Span> {
    let mut da = Da { ast, name, reads: Vec::new(), targets: Vec::new(), hidden: 0, gave_up: false };
    da.stmts(stmts, State::Unassigned);
    if da.gave_up { Vec::new() } else { da.reads }
}

impl Da<'_> {
    fn stmts(&mut self, ss: &[StmtId], mut st: State) -> State {
        let hidden = self.hidden;
        for &s in ss {
            st = self.stmt(s, st);
            if matches!(&self.ast.stmt(s).kind, StmtKind::Let { name, .. } if *name == self.name) {
                self.hidden += 1;
            }
        }
        self.hidden = hidden;
        st
    }

    fn stmt(&mut self, s: StmtId, st: State) -> State {
        let ast = self.ast;
        match &ast.stmt(s).kind {
            StmtKind::Let { init, .. } => match init {
                Some(e) => self.expr(*e, st),
                None => st,
            },
            StmtKind::Expr(e) => self.expr(*e, st),
            StmtKind::If(c, a, b) => {
                let st = self.expr(*c, st);
                let sa = self.stmt(*a, st);
                let sb = match b {
                    Some(b) => self.stmt(*b, st),
                    None => st,
                };
                join(sa, sb)
            }
            StmtKind::While(c, body) => {
                let st = self.expr(*c, st);
                let t = self.in_loop(true, |d| { d.stmt(*body, st); });
                let exit = if is_true(ast, *c) { State::Dead } else { st };
                join(exit, t.breaks)
            }
            StmtKind::DoWhile(body, c) => {
                let mut end = State::Dead;
                let t = self.in_loop(true, |d| end = d.stmt(*body, st));
                let st = self.expr(*c, join(end, t.continues));
                let exit = if is_true(ast, *c) { State::Dead } else { st };
                join(exit, t.breaks)
            }
            StmtKind::For { init, cond, step, body } => {
                let st = match init {
                    Some(i) => self.stmt(*i, st),
                    None => st,
                };
                let st = match cond {
                    Some(c) => self.expr(*c, st),
                    None => st,
                };
                let mut end = State::Dead;
                let t = self.in_loop(true, |d| end = d.stmt(*body, st));
                if let Some(x) = step {
                    self.expr(*x, join(end, t.continues));
                }
                let exit = if cond.is_none_or(|c| is_true(ast, c)) { State::Dead } else { st };
                join(exit, t.breaks)
            }
            StmtKind::ForOf { name, iter, body, .. } => {
                let st = self.expr(*iter, st);
                let hide = *name == self.name;
                self.hidden += hide as u32;
                let t = self.in_loop(true, |d| { d.stmt(*body, st); });
                self.hidden -= hide as u32;
                join(st, t.breaks)
            }
            StmtKind::Switch(d, cases) => {
                let st = self.expr(*d, st);
                let mut end = State::Dead;
                let t = self.in_loop(false, |da| {
                    for c in cases {
                        let mut cs = join(st, end);
                        if let Some(x) = c.test {
                            cs = da.expr(x, cs);
                        }
                        end = da.stmts(&c.body, cs);
                    }
                });
                let exit = if cases.iter().any(|c| c.test.is_none()) { end } else { join(st, end) };
                join(exit, t.breaks)
            }
            StmtKind::Return(e) => {
                if let Some(e) = e {
                    self.expr(*e, st);
                }
                State::Dead
            }
            StmtKind::Throw(e) => {
                self.expr(*e, st);
                State::Dead
            }
            StmtKind::Break(label) | StmtKind::Continue(label) => {
                let is_break = matches!(&ast.stmt(s).kind, StmtKind::Break(_));
                if label.is_some() {
                    self.gave_up = true;
                    return State::Dead;
                }
                let t = if is_break { self.targets.last_mut() } else { self.targets.iter_mut().rev().find(|t| t.is_loop) };
                match t {
                    Some(t) if is_break => t.breaks = join(t.breaks, st),
                    Some(t) => t.continues = join(t.continues, st),
                    None => self.gave_up = true,
                }
                State::Dead
            }
            StmtKind::Block(ss) => self.stmts(ss, st),
            StmtKind::Try { body, catch, finally } => {
                let sb = self.stmt(*body, st);
                let after = match catch {
                    Some(c) => {
                        let hide = c.param.as_ref().is_some_and(|p| p.0 == self.name);
                        self.hidden += hide as u32;
                        let sc = self.stmt(c.body, st);
                        self.hidden -= hide as u32;
                        join(sb, sc)
                    }
                    None => sb,
                };
                match finally {
                    Some(f) => match self.stmt(*f, st) {
                        State::Unassigned => after,
                        s => s,
                    },
                    None => after,
                }
            }
            StmtKind::InitGlobal(_) | StmtKind::Empty | StmtKind::Error => st,
        }
    }

    /// `f` inside a loop (or a `switch`, for `break`): what its `break`s and `continue`s saw.
    fn in_loop(&mut self, is_loop: bool, f: impl FnOnce(&mut Self)) -> Target {
        self.targets.push(Target { breaks: State::Dead, continues: State::Dead, is_loop });
        f(self);
        self.targets.pop().unwrap()
    }

    fn is_x(&self, e: ExprId) -> bool {
        self.hidden == 0 && matches!(self.ast.expr(e).kind, ExprKind::Ident(n) if n == self.name)
    }

    fn read(&mut self, e: ExprId, st: State) {
        if st == State::Unassigned {
            self.reads.push(self.ast.expr(e).span);
        }
    }

    fn expr(&mut self, e: ExprId, st: State) -> State {
        let ast = self.ast;
        match &ast.expr(e).kind {
            ExprKind::Ident(_) => {
                if self.is_x(e) {
                    self.read(e, st);
                }
                st
            }
            // (functions may run later: neither their reads nor their assignments count)
            ExprKind::Arrow(_) => st,
            ExprKind::Typeof(x) if self.is_x(*x) => st,
            ExprKind::Assign(op, l, r) => {
                if self.is_x(*l) {
                    if matches!(op, AssignOp::Op(_)) {
                        self.read(*l, st);
                    }
                    let st = self.expr(*r, st);
                    return if st == State::Dead { st } else { State::Assigned };
                }
                if matches!(ast.expr(*l).kind, ExprKind::Array(_) | ExprKind::Object(_)) {
                    let st = self.expr(*r, st);
                    return if self.binds(*l) && st != State::Dead { State::Assigned } else { st };
                }
                let st = self.expr(*l, st);
                self.expr(*r, st)
            }
            ExprKind::Binary(BinOp::And | BinOp::Or | BinOp::Nullish, l, r) => {
                let st = self.expr(*l, st);
                join(st, self.expr(*r, st))
            }
            ExprKind::Cond(c, a, b) => {
                let st = self.expr(*c, st);
                join(self.expr(*a, st), self.expr(*b, st))
            }
            ExprKind::Call { optional: true, callee, args, .. } => {
                let st = self.expr(*callee, st);
                let mut s = st;
                for a in args {
                    s = self.expr(a.expr, s);
                }
                join(st, s)
            }
            _ => {
                let mut st = st;
                for x in super::class::children(ast, e) {
                    st = self.expr(x, st);
                }
                st
            }
        }
    }

    /// A destructuring pattern (`[a, x] = ...`, `({ x } = ...)`) that assigns `x`.
    fn binds(&self, p: ExprId) -> bool {
        match &self.ast.expr(p).kind {
            ExprKind::Array(xs) => xs.iter().any(|&x| self.binds(x)),
            ExprKind::Object(fs) => fs.iter().any(|f| self.binds(f.value)),
            ExprKind::Spread(x) | ExprKind::Paren(x) => self.binds(*x),
            ExprKind::Assign(_, l, _) => self.binds(*l),
            _ => self.is_x(p),
        }
    }
}

fn is_true(ast: &Ast, e: ExprId) -> bool {
    match &ast.expr(e).kind {
        ExprKind::Bool(b) => *b,
        ExprKind::Paren(x) => is_true(ast, *x),
        _ => false,
    }
}
