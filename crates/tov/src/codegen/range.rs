//! What values an `int` expression can take, to leave out overflow checks that can't fire:
//! `(seed * IA + IC) % IM` when `seed` only ever holds a remainder by `IM`, `(h * 31 + b) &
//! 0xffffff` when `h` is always masked. Each `let` of type `int` (and each module variable)
//! takes the union of what it's ever given (its initializer, assignments, `+=`, `++`), worked
//! out for the whole program at once and widened to "anything" when it keeps growing (a loop
//! counter). Parameters, loop variables and whatever else isn't tracked can be anything.

use super::*;
use crate::ast::{AssignOp, BinOp, StmtId, UnOp};

/// A set of values: none yet (while working it out), an interval, or anything.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum R {
    Bot,
    In(i128, i128),
    Top,
}

const MIN: i128 = i64::MIN as i128;
const MAX: i128 = i64::MAX as i128;
/// Arrays and strings are shorter than 2^48 (see `tv_arr`).
const MAX_LEN: i128 = 1 << 48;

impl R {
    fn join(self, o: R) -> R {
        match (self, o) {
            (R::Bot, x) | (x, R::Bot) => x,
            (R::In(a, b), R::In(c, d)) => R::In(a.min(c), b.max(d)),
            _ => R::Top,
        }
    }

    /// The interval, if it fits an `int`.
    fn fit(lo: i128, hi: i128) -> R {
        if lo < MIN || hi > MAX { R::Top } else { R::In(lo, hi) }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Var {
    /// (module, the declaring name's offset)
    Local(u32, u32),
    /// (module, item)
    Global(u32, u32),
}

enum Def {
    /// `x = e` / its initializer (module, expression).
    Expr(u32, ExprId),
    /// `x op= e`.
    Op(u32, BinOp, ExprId),
    /// `x++` / `x--`.
    Step(i128),
    Top,
}

pub(crate) struct Ranges {
    vars: FxMap<Var, R>,
}

impl<'c, 'a> Gen<'c, 'a> {
    /// What `int` expression `e` (of the current module) can be.
    pub(crate) fn int_range(&mut self, e: ExprId) -> R {
        if self.ranges.is_none() {
            let r = self.compute_ranges();
            self.ranges = Some(r);
        }
        let m = self.cur_m();
        let ranges = self.ranges.take().unwrap();
        let r = self.eval_range(&ranges.vars, m, e);
        self.ranges = Some(ranges);
        r
    }

    /// `l op r` (`+`, `-`, `*` of `int`s) can't overflow.
    pub(crate) fn cannot_overflow(&mut self, op: BinOp, l: ExprId, r: ExprId) -> bool {
        let (a, b) = (self.int_range(l), self.int_range(r));
        matches!(interval_op(op, a, b), R::In(..))
    }

    fn var_of(&self, m: u32, e: ExprId) -> Option<Var> {
        match (&self.ast(m).expr(e).kind, self.facts(m).idents.get(&e)) {
            (ExprKind::Ident(_), Some(IdentFact::Local(k))) => Some(Var::Local(m, *k)),
            (ExprKind::Ident(_), Some(IdentFact::Const(cm, ci))) => Some(Var::Global(*cm, *ci)),
            (ExprKind::Paren(x), _) => self.var_of(m, *x),
            _ => None,
        }
    }

    fn compute_ranges(&self) -> Ranges {
        let mut defs: FxMap<Var, Vec<Def>> = FxMap::default();
        // names assigned where it isn't known what they name: anything
        let mut unknown: FxSet<(u32, Sym)> = FxSet::default();
        let n = self.c.modules.len() as u32;
        for m in 0..n {
            let ast = self.ast(m);
            let facts = self.facts(m);
            for (si, s) in ast.stmts.iter().enumerate() {
                if let StmtKind::Let { name_span, init, .. } = &s.kind
                    && facts.bindings.get(&(si as StmtId)).copied() == Some(INT)
                {
                    let d = match init {
                        Some(i) => Def::Expr(m, *i),
                        None => Def::Top,
                    };
                    defs.entry(Var::Local(m, name_span.start)).or_default().push(d);
                }
            }
            for (ii, item) in ast.items.iter().enumerate() {
                if let ItemKind::Const { init, .. } = &item.kind
                    && facts.expr_ty.get(*init as usize).copied() == Some(INT)
                {
                    defs.entry(Var::Global(m, ii as u32)).or_default().push(Def::Expr(m, *init));
                }
            }
        }
        for m in 0..n {
            let ast = self.ast(m);
            for e in ast.exprs.iter() {
                let (target, def) = match &e.kind {
                    ExprKind::Assign(AssignOp::Assign, t, v) => (*t, Def::Expr(m, *v)),
                    ExprKind::Assign(op, t, v) => match assign_bin(*op) {
                        Some(b) => (*t, Def::Op(m, b, *v)),
                        None => (*t, Def::Top),
                    },
                    ExprKind::Update { target, inc, .. } => (*target, Def::Step(if *inc { 1 } else { -1 })),
                    ExprKind::Call { args, .. } | ExprKind::New { args, .. } => {
                        for a in args.iter().filter(|a| a.by_ref.is_some()) {
                            match self.var_of(m, a.expr) {
                                Some(v) => defs.entry(v).or_default().push(Def::Top),
                                None => {
                                    if let ExprKind::Ident(s) = ast.expr(a.expr).kind {
                                        unknown.insert((m, s));
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    _ => continue,
                };
                match self.var_of(m, target) {
                    Some(v) => {
                        if let Some(d) = defs.get_mut(&v) {
                            d.push(def);
                        }
                    }
                    None => {
                        if let ExprKind::Ident(s) = ast.expr(target).kind {
                            unknown.insert((m, s));
                        }
                    }
                }
            }
        }
        let mut vars: FxMap<Var, R> = defs.keys().map(|&v| (v, R::Bot)).collect();
        for (&v, r) in vars.iter_mut() {
            let name = match v {
                Var::Local(m, k) => self.ast(m).stmts.iter().find_map(|s| match s.kind {
                    StmtKind::Let { name, name_span, .. } if name_span.start == k => Some((m, name)),
                    _ => None,
                }),
                // (a module variable can be assigned from other modules)
                Var::Global(m, i) => match self.ast(m).items[i as usize].kind {
                    ItemKind::Const { name, .. } => Some((m, name)),
                    _ => None,
                },
            };
            let global = matches!(v, Var::Global(..));
            if name.is_some_and(|(m, s)| unknown.contains(&(m, s)) || (global && unknown.iter().any(|&(_, u)| u == s))) || defs[&v].iter().any(|d| matches!(d, Def::Top)) {
                *r = R::Top;
            }
        }
        let keys: Vec<Var> = defs.keys().copied().collect();
        let eval_defs = |g: &Self, vars: &FxMap<Var, R>, v: Var| {
            let old = vars[&v];
            let mut new = R::Bot;
            for d in &defs[&v] {
                let r = match *d {
                    Def::Expr(m, e) => g.eval_range(vars, m, e),
                    Def::Op(m, op, e) => interval_op(op, old, g.eval_range(vars, m, e)),
                    Def::Step(s) => interval_op(BinOp::Add, old, R::In(s, s)),
                    Def::Top => R::Top,
                };
                new = new.join(r);
            }
            new
        };
        // Up to a fixpoint, widening a bound that keeps moving to the end of the range (a
        // counter's)...
        let mut stable = false;
        for round in 0..64 {
            let mut changed = false;
            for &v in &keys {
                let old = vars[&v];
                if old == R::Top {
                    continue;
                }
                let mut new = old.join(eval_defs(self, &vars, v));
                if round >= 3
                    && let (R::In(a, b), R::In(c, d)) = (old, new)
                {
                    new = R::In(if c < a { MIN } else { a }, if d > b { MAX } else { b });
                }
                if new != old {
                    vars.insert(v, new);
                    changed = true;
                }
            }
            if !changed {
                stable = true;
                break;
            }
        }
        if !stable {
            return Ranges { vars: FxMap::default() };
        }
        // ... then down again where that went too far: `h = (h * 31 + b) & 0xffffff` is at most
        // 0xffffff whatever `h` was
        for _ in 0..3 {
            for &v in &keys {
                if vars[&v] != R::Top {
                    let new = eval_defs(self, &vars, v);
                    vars.insert(v, new);
                }
            }
        }
        Ranges { vars }
    }

    fn eval_range(&self, vars: &FxMap<Var, R>, m: u32, e: ExprId) -> R {
        let ast = self.ast(m);
        let facts = self.facts(m);
        let ty = facts.expr_ty.get(e as usize).copied().unwrap_or(ERROR);
        let sized = match self.tget(ty) {
            Ty::U8 => Some((0, u8::MAX as i128)),
            Ty::I8 => Some((i8::MIN as i128, i8::MAX as i128)),
            Ty::U16 => Some((0, u16::MAX as i128)),
            Ty::I16 => Some((i16::MIN as i128, i16::MAX as i128)),
            Ty::U32 => Some((0, u32::MAX as i128)),
            Ty::I32 => Some((i32::MIN as i128, i32::MAX as i128)),
            Ty::Int => None,
            _ => return R::Top,
        };
        if let Some((lo, hi)) = sized {
            return R::In(lo, hi);
        }
        match &ast.expr(e).kind {
            ExprKind::Int(v) => R::fit(*v as i128, *v as i128),
            ExprKind::Paren(x) | ExprKind::NonNull(x) => self.eval_range(vars, m, *x),
            ExprKind::Unary(UnOp::Neg, x) => match self.eval_range(vars, m, *x) {
                R::In(lo, hi) => R::fit(-hi, -lo),
                r => r,
            },
            ExprKind::Binary(op @ (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Rem), a, b) => {
                let (ra, rb) = (self.eval_range(vars, m, *a), self.eval_range(vars, m, *b));
                interval_op(*op, ra, rb)
            }
            ExprKind::Binary(BinOp::BitAnd, a, b) => {
                // `x & c`, `c >= 0`: from 0 to `c`
                let (ra, rb) = (self.eval_range(vars, m, *a), self.eval_range(vars, m, *b));
                match (ra, rb) {
                    (R::Bot, _) | (_, R::Bot) => R::Bot,
                    (R::In(lo, hi), R::In(lo2, hi2)) if lo >= 0 && lo2 >= 0 => R::In(0, hi.min(hi2)),
                    (R::In(lo, hi), _) | (_, R::In(lo, hi)) if lo >= 0 => R::In(0, hi),
                    _ => R::Top,
                }
            }
            ExprKind::Cond(_, a, b) => self.eval_range(vars, m, *a).join(self.eval_range(vars, m, *b)),
            ExprKind::Assign(AssignOp::Assign, _, v) => self.eval_range(vars, m, *v),
            ExprKind::Ident(_) => match self.var_of(m, e) {
                Some(v) => vars.get(&v).copied().unwrap_or(R::Top),
                None => R::Top,
            },
            ExprKind::Member { name, .. } if self.sym(*name) == "length" => R::In(0, MAX_LEN),
            ExprKind::Call { callee, args, .. } => {
                let callee_name = |c: ExprId| match &ast.expr(c).kind {
                    ExprKind::Ident(s) => Some((None, self.sym(*s))),
                    ExprKind::Member { obj, name, .. } => match ast.expr(*obj).kind {
                        ExprKind::Ident(o) => Some((Some(self.sym(o)), self.sym(*name))),
                        _ => None,
                    },
                    _ => None,
                };
                let builtin = matches!(facts.calls.get(&e).map(|c| &c.callee), Some(Callee::Builtin { .. }));
                match (builtin, callee_name(*callee), args.as_slice()) {
                    (true, Some((None, "int")), [a]) => self.eval_range(vars, m, a.expr),
                    (true, Some((Some("Math"), f @ ("min" | "max"))), [a, b]) => {
                        let (ra, rb) = (self.eval_range(vars, m, a.expr), self.eval_range(vars, m, b.expr));
                        match (ra, rb) {
                            (R::Bot, _) | (_, R::Bot) => R::Bot,
                            (R::In(a0, a1), R::In(b0, b1)) if f == "min" => R::In(a0.min(b0), a1.min(b1)),
                            (R::In(a0, a1), R::In(b0, b1)) => R::In(a0.max(b0), a1.max(b1)),
                            // (`Math.min(x, c)` is at most c; `Math.max(x, c)` at least c)
                            (R::In(_, hi), R::Top) | (R::Top, R::In(_, hi)) if f == "min" => R::fit(MIN, hi),
                            (R::In(lo, _), R::Top) | (R::Top, R::In(lo, _)) => R::fit(lo, MAX),
                            _ => R::Top,
                        }
                    }
                    _ => R::Top,
                }
            }
            _ => R::Top,
        }
    }
}

fn assign_bin(op: AssignOp) -> Option<BinOp> {
    match op {
        AssignOp::Op(b @ (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Rem)) => Some(b),
        _ => None,
    }
}

fn interval_op(op: BinOp, a: R, b: R) -> R {
    let (R::In(a0, a1), R::In(b0, b1)) = (a, b) else {
        return if a == R::Bot || b == R::Bot { R::Bot } else { R::Top };
    };
    match op {
        BinOp::Add => R::fit(a0 + b0, a1 + b1),
        BinOp::Sub => R::fit(a0 - b1, a1 - b0),
        BinOp::Mul => {
            let ps = [a0 * b0, a0 * b1, a1 * b0, a1 * b1];
            R::fit(*ps.iter().min().unwrap(), *ps.iter().max().unwrap())
        }
        // `x % d`, `d > 0`: smaller than d, and the sign of x
        BinOp::Rem if b0 > 0 => {
            let d = b1 - 1;
            if a0 >= 0 { R::In(0, a1.min(d)) } else { R::In((-d).max(a0.min(0)), d.min(a1.max(0))) }
        }
        _ => R::Top,
    }
}
