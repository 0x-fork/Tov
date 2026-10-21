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
/// Small values (lengths, indexes) and how far steps of one can take a counter (see
/// `compute_ranges`).
const SMALL: i128 = 1 << 48;
const STEPS: i128 = 1 << 62;

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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Var {
    /// (module, the declaring name's offset)
    Local(u32, u32),
    /// (module, item)
    Global(u32, u32),
    /// The elements of a local `int[]` (module, the declaring name's offset), and of every local
    /// it's assigned to or from (one array, shared).
    Elems(u32, u32),
}

#[derive(Clone, Copy)]
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
        self.param_defs(&mut defs);
        self.array_defs(&mut defs);
        let mut vars: FxMap<Var, R> = defs.keys().map(|&v| (v, R::Bot)).collect();
        // (anything from the start: changed where it isn't known how)
        let mut forced: FxSet<Var> = FxSet::default();
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
                Var::Elems(..) => None,
            };
            let global = matches!(v, Var::Global(..));
            if name.is_some_and(|(m, s)| unknown.contains(&(m, s)) || (global && unknown.iter().any(|&(_, u)| u == s))) || defs[&v].iter().any(|d| matches!(d, Def::Top)) {
                *r = R::Top;
                forced.insert(v);
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
        // Counters and the like: variables (and array elements) only ever given small values
        // (within 2^48 of zero, as lengths and indexes are) or one of their own values moved by at
        // most one (`flips++`, `d = Math.min(prev[i]! + 1, curr[i - 1]! + cost)`). After N such
        // steps none is more than N past 2^48; 2^62 steps take decades, so that's as far as they go.
        let mut cand: FxSet<Var> = keys.iter().copied().filter(|v| !forced.contains(v)).collect();
        loop {
            let before = cand.len();
            let keep: Vec<Var> = cand
                .iter()
                .copied()
                .filter(|v| {
                    defs[v].iter().all(|d| match *d {
                        Def::Step(1 | -1) => true,
                        Def::Op(m, BinOp::Add | BinOp::Sub, e) => self.unit(&vars, m, e),
                        Def::Expr(m, e) => self.step_of(&vars, &cand, m, e).is_some_and(|k| k <= 1),
                        _ => false,
                    })
                })
                .collect();
            cand = keep.into_iter().collect();
            if cand.len() == before {
                break;
            }
        }
        // (and where none of their steps goes down, none goes below where they started: `i++`
        // from 0 is never negative)
        const DOWN: i128 = i128::MIN;
        let mut low: FxMap<Var, i128> = cand.iter().map(|&v| (v, i128::MAX)).collect();
        let mut settled = false;
        for _ in 0..12 {
            let mut changed = false;
            for &v in &keys {
                if !cand.contains(&v) {
                    continue;
                }
                let mut lo = i128::MAX;
                for d in &defs[&v] {
                    let l = match *d {
                        Def::Step(1) => low[&v],
                        Def::Op(m, BinOp::Add, e) if matches!(self.eval_range(&vars, m, e), R::In(a, _) if a >= 0) => low[&v],
                        Def::Op(m, BinOp::Sub, e) if matches!(self.eval_range(&vars, m, e), R::In(_, b) if b <= 0) => low[&v],
                        Def::Expr(m, e) => self.low_of(&vars, &low, m, e),
                        _ => DOWN,
                    };
                    lo = lo.min(l);
                }
                if lo < low[&v] {
                    low.insert(v, lo);
                    changed = true;
                }
            }
            if !changed {
                settled = true;
                break;
            }
        }
        for &v in &keys {
            if cand.contains(&v) && !defs[&v].iter().all(|d| matches!(*d, Def::Expr(m, e) if self.small(&vars, m, e))) {
                let lo = if settled && low[&v] >= -SMALL { low[&v] } else { -SMALL - STEPS };
                vars.insert(v, R::In(lo, SMALL + STEPS));
            }
        }
        Ranges { vars }
    }

    /// `e` is within 2^48 of zero.
    fn small(&self, vars: &FxMap<Var, R>, m: u32, e: ExprId) -> bool {
        matches!(self.eval_range(vars, m, e), R::In(lo, hi) if lo >= -SMALL && hi <= SMALL)
    }

    /// `e` is -1, 0 or 1.
    fn unit(&self, vars: &FxMap<Var, R>, m: u32, e: ExprId) -> bool {
        matches!(self.eval_range(vars, m, e), R::In(lo, hi) if lo >= -1 && hi <= 1)
    }

    /// How far `e` can be from a value of the variables `of` (or small): at most the number of
    /// ones it adds along a path, `None` if it isn't such an expression.
    fn step_of(&self, vars: &FxMap<Var, R>, of: &FxSet<Var>, m: u32, e: ExprId) -> Option<u32> {
        if self.small(vars, m, e) {
            return Some(0);
        }
        let ast = self.ast(m);
        match &ast.expr(e).kind {
            ExprKind::Paren(x) | ExprKind::NonNull(x) => self.step_of(vars, of, m, *x),
            ExprKind::Ident(_) => self.var_of(m, e).filter(|v| of.contains(v)).map(|_| 0),
            ExprKind::Index { obj, .. } => match self.var_of(m, *obj) {
                Some(Var::Local(am, k)) if of.contains(&Var::Elems(am, k)) => Some(0),
                _ => None,
            },
            ExprKind::Binary(BinOp::Add, a, b) if self.unit(vars, m, *b) => self.step_of(vars, of, m, *a).map(|k| k + 1),
            ExprKind::Binary(BinOp::Add, a, b) if self.unit(vars, m, *a) => self.step_of(vars, of, m, *b).map(|k| k + 1),
            ExprKind::Binary(BinOp::Sub, a, b) if self.unit(vars, m, *b) => self.step_of(vars, of, m, *a).map(|k| k + 1),
            ExprKind::Cond(_, a, b) => Some(self.step_of(vars, of, m, *a)?.max(self.step_of(vars, of, m, *b)?)),
            ExprKind::Call { callee, args, .. } if matches!(self.facts(m).calls.get(&e).map(|c| &c.callee), Some(Callee::Builtin { .. })) => {
                let ExprKind::Member { obj, name, .. } = &ast.expr(*callee).kind else { return None };
                if !matches!(ast.expr(*obj).kind, ExprKind::Ident(o) if self.sym(o) == "Math") || !matches!(self.sym(*name), "min" | "max") || args.is_empty() {
                    return None;
                }
                let mut k = 0;
                for a in args {
                    if matches!(ast.expr(a.expr).kind, ExprKind::Spread(_)) {
                        return None;
                    }
                    k = k.max(self.step_of(vars, of, m, a.expr)?);
                }
                Some(k)
            }
            _ => None,
        }
    }

    /// Parameters of module functions only ever called directly: what their calls pass (a
    /// function used as a value, or called with a spread or a different count, can get anything).
    fn param_defs(&self, defs: &mut FxMap<Var, Vec<Def>>) {
        let n = self.c.modules.len() as u32;
        let mut args: FxMap<(u32, u32), Vec<Option<Vec<(u32, ExprId)>>>> = FxMap::default();
        let mut valued: FxSet<(u32, u32)> = FxSet::default();
        for m in 0..n {
            let ast = self.ast(m);
            let facts = self.facts(m);
            let mut callees: FxSet<ExprId> = FxSet::default();
            for (ei, e) in ast.exprs.iter().enumerate() {
                if let ExprKind::Call { callee, args: xs, .. } = &e.kind
                    && let Some(Callee::Fn(fm, fi)) = facts.calls.get(&(ei as ExprId)).map(|c| &c.callee)
                {
                    let mut c = *callee;
                    while let ExprKind::Paren(x) = ast.expr(c).kind {
                        c = x;
                    }
                    callees.insert(c);
                    let given = (!xs.iter().any(|a| a.by_ref.is_some() || matches!(ast.expr(a.expr).kind, ExprKind::Spread(_)))).then(|| xs.iter().map(|a| (m, a.expr)).collect());
                    args.entry((*fm, *fi)).or_default().push(given);
                }
            }
            for (&e, f) in &facts.idents {
                if let IdentFact::Fn(fm, fi) = f
                    && !callees.contains(&e)
                {
                    valued.insert((*fm, *fi));
                }
            }
            for (&e, f) in &facts.members {
                if let crate::check::MemberFact::NsFn(fm, fi) = f
                    && !callees.contains(&e)
                {
                    valued.insert((*fm, *fi));
                }
            }
        }
        for m in 0..n {
            for (ii, item) in self.ast(m).items.iter().enumerate() {
                let ItemKind::Function(f) = &item.kind else { continue };
                let key = (m, ii as u32);
                let calls = args.get(&key);
                for (pi, p) in f.params.iter().enumerate() {
                    if p.rest || p.optional || p.inout {
                        continue;
                    }
                    let ds: Vec<Def> = match calls {
                        Some(cs) if !valued.contains(&key) => cs
                            .iter()
                            .map(|c| match c {
                                Some(xs) if xs.len() == f.params.len() => Def::Expr(xs[pi].0, xs[pi].1),
                                _ => Def::Top,
                            })
                            .collect(),
                        _ => vec![Def::Top],
                    };
                    defs.entry(Var::Local(m, p.span.start)).or_default().extend(ds);
                }
            }
        }
    }

    /// The least `e` can be, given these least values of the variables in `low` (`step_of`'s
    /// expressions; `i128::MIN` if it can go down).
    fn low_of(&self, vars: &FxMap<Var, R>, low: &FxMap<Var, i128>, m: u32, e: ExprId) -> i128 {
        if let R::In(lo, hi) = self.eval_range(vars, m, e)
            && lo >= -SMALL
            && hi <= SMALL
        {
            return lo;
        }
        let ast = self.ast(m);
        let nonneg = |x: ExprId| matches!(self.eval_range(vars, m, x), R::In(a, _) if a >= 0);
        match &ast.expr(e).kind {
            ExprKind::Paren(x) | ExprKind::NonNull(x) => self.low_of(vars, low, m, *x),
            ExprKind::Ident(_) => self.var_of(m, e).and_then(|v| low.get(&v).copied()).unwrap_or(i128::MIN),
            ExprKind::Index { obj, .. } => match self.var_of(m, *obj) {
                Some(Var::Local(am, k)) => low.get(&Var::Elems(am, k)).copied().unwrap_or(i128::MIN),
                _ => i128::MIN,
            },
            ExprKind::Binary(BinOp::Add, a, b) if nonneg(*b) => self.low_of(vars, low, m, *a),
            ExprKind::Binary(BinOp::Add, a, b) if nonneg(*a) => self.low_of(vars, low, m, *b),
            ExprKind::Binary(BinOp::Sub, a, b) if matches!(self.eval_range(vars, m, *b), R::In(_, h) if h <= 0) => self.low_of(vars, low, m, *a),
            ExprKind::Cond(_, a, b) => self.low_of(vars, low, m, *a).min(self.low_of(vars, low, m, *b)),
            ExprKind::Call { args, .. } if self.step_of(vars, &low.keys().copied().collect(), m, e).is_some() => {
                args.iter().map(|a| self.low_of(vars, low, m, a.expr)).min().unwrap_or(i128::MIN)
            }
            _ => i128::MIN,
        }
    }

    /// The elements of local `int[]`s that stay in their function: what's pushed and stored, and
    /// `Top` if the array is used any other way (passed, stored, returned, captured...).
    fn array_defs(&self, defs: &mut FxMap<Var, Vec<Def>>) {
        // (the arrays, joined where one is assigned to another)
        let mut parent: FxMap<(u32, u32), (u32, u32)> = FxMap::default();
        fn find(p: &mut FxMap<(u32, u32), (u32, u32)>, x: (u32, u32)) -> (u32, u32) {
            let mut r = x;
            while p[&r] != r {
                r = p[&r];
            }
            p.insert(x, r);
            r
        }
        let n = self.c.modules.len() as u32;
        let mut found: Vec<((u32, u32), Def)> = Vec::new();
        let mut joins: Vec<((u32, u32), (u32, u32))> = Vec::new();
        let mut escaped: FxSet<(u32, u32)> = FxSet::default();
        for m in 0..n {
            let ast = self.ast(m);
            let facts = self.facts(m);
            for (si, s) in ast.stmts.iter().enumerate() {
                if let StmtKind::Let { name_span, .. } = &s.kind
                    && let Some(&t) = facts.bindings.get(&(si as StmtId))
                    && matches!(self.tget(t), Ty::Array(INT))
                {
                    parent.insert((m, name_span.start), (m, name_span.start));
                }
            }
        }
        if parent.is_empty() {
            return;
        }
        let tracked = |m: u32, e: ExprId| match (&self.ast(m).expr(e).kind, self.facts(m).idents.get(&e)) {
            (ExprKind::Ident(_), Some(IdentFact::Local(k))) if parent.contains_key(&(m, *k)) => Some((m, *k)),
            _ => None,
        };
        for m in 0..n {
            let ast = self.ast(m);
            let mut allowed: FxSet<ExprId> = FxSet::default();
            // `xs = [..]`, `xs = ys`: elements, or one array
            let given = |a: (u32, u32), v: ExprId, allowed: &mut FxSet<ExprId>, found: &mut Vec<((u32, u32), Def)>, joins: &mut Vec<_>, escaped: &mut FxSet<(u32, u32)>| match &ast.expr(v).kind {
                ExprKind::Array(xs) => {
                    for &x in xs {
                        if matches!(ast.expr(x).kind, ExprKind::Spread(_)) {
                            escaped.insert(a);
                        } else {
                            found.push((a, Def::Expr(m, x)));
                        }
                    }
                }
                _ => match tracked(m, v) {
                    Some(b) => {
                        allowed.insert(v);
                        joins.push((a, b));
                    }
                    None => {
                        escaped.insert(a);
                    }
                },
            };
            for s in ast.stmts.iter() {
                match &s.kind {
                    StmtKind::Let { name_span, init, .. } if parent.contains_key(&(m, name_span.start)) => {
                        if let Some(i) = init {
                            given((m, name_span.start), *i, &mut allowed, &mut found, &mut joins, &mut escaped);
                        }
                    }
                    StmtKind::ForOf { iter, .. } => {
                        allowed.insert(*iter);
                    }
                    _ => {}
                }
            }
            for e in ast.exprs.iter() {
                match &e.kind {
                    ExprKind::Index { obj, .. } => {
                        allowed.insert(*obj);
                    }
                    ExprKind::Member { obj, name, .. } if matches!(self.sym(*name), "length" | "pop" | "shift" | "sort" | "reverse" | "at" | "includes" | "indexOf" | "lastIndexOf" | "join" | "slice" | "map" | "filter" | "forEach" | "reduce" | "some" | "every" | "find" | "findIndex" | "findLast" | "findLastIndex" | "push" | "unshift") => {
                        allowed.insert(*obj);
                    }
                    _ => {}
                }
                let target = |t: ExprId| match &ast.expr(t).kind {
                    ExprKind::Index { obj, .. } => tracked(m, *obj),
                    ExprKind::Paren(x) | ExprKind::NonNull(x) => match &ast.expr(*x).kind {
                        ExprKind::Index { obj, .. } => tracked(m, *obj),
                        _ => None,
                    },
                    _ => None,
                };
                match &e.kind {
                    ExprKind::Assign(op, t, v) => {
                        if let Some(a) = target(*t) {
                            found.push((a, match (op, assign_bin(*op)) {
                                (AssignOp::Assign, _) => Def::Expr(m, *v),
                                (_, Some(b)) => Def::Op(m, b, *v),
                                _ => Def::Top,
                            }));
                        } else if let (AssignOp::Assign, Some(a)) = (op, tracked(m, *t)) {
                            allowed.insert(*t);
                            given(a, *v, &mut allowed, &mut found, &mut joins, &mut escaped);
                        }
                    }
                    ExprKind::Update { target: t, inc, .. } => {
                        if let Some(a) = target(*t) {
                            found.push((a, Def::Step(if *inc { 1 } else { -1 })));
                        }
                    }
                    ExprKind::Call { callee, args, .. } => {
                        if let ExprKind::Member { obj, name, .. } = &ast.expr(*callee).kind
                            && matches!(self.sym(*name), "push" | "unshift")
                            && let Some(a) = tracked(m, *obj)
                        {
                            for x in args {
                                if matches!(ast.expr(x.expr).kind, ExprKind::Spread(_)) {
                                    escaped.insert(a);
                                } else {
                                    found.push((a, Def::Expr(m, x.expr)));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            // every other use: anything can happen to the elements
            for (ei, _) in ast.exprs.iter().enumerate() {
                if let Some(a) = tracked(m, ei as ExprId)
                    && !allowed.contains(&(ei as ExprId))
                {
                    escaped.insert(a);
                }
            }
        }
        for (a, b) in joins {
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            if ra != rb {
                parent.insert(ra, rb);
            }
        }
        let mut class_defs: FxMap<(u32, u32), Vec<Def>> = FxMap::default();
        for (a, d) in found {
            let r = find(&mut parent, a);
            class_defs.entry(r).or_default().push(d);
        }
        for a in escaped {
            let r = find(&mut parent, a);
            class_defs.entry(r).or_default().push(Def::Top);
        }
        let members: Vec<(u32, u32)> = parent.keys().copied().collect();
        for a in members {
            let r = find(&mut parent, a);
            let ds = class_defs.get(&r).map(|ds| ds.to_vec()).unwrap_or_default();
            defs.insert(Var::Elems(a.0, a.1), ds);
        }
    }

    fn eval_range(&self, vars: &FxMap<Var, R>, m: u32, e: ExprId) -> R {
        let ast = self.ast(m);
        let facts = self.facts(m);
        // `xs[i]!`: an element (`xs[i]` alone may be `undefined`)
        if let ExprKind::NonNull(x) = ast.expr(e).kind
            && let ExprKind::Index { obj, .. } = ast.expr(x).kind
            && let Some(Var::Local(am, k)) = self.var_of(m, obj)
            && let Some(&r) = vars.get(&Var::Elems(am, k))
        {
            return r;
        }
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
            ExprKind::Index { obj, .. } => match self.var_of(m, *obj) {
                Some(Var::Local(am, k)) => vars.get(&Var::Elems(am, k)).copied().unwrap_or(R::Top),
                _ => R::Top,
            },
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
