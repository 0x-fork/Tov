//! Records that can stay values. A record type changed in place is an object (shared, as
//! JavaScript's are) unless no program point can tell a copy of one of its values from the
//! value itself: then it stays a plain struct, stored inline in arrays and fields (n-body's
//! bodies, changed only through `bodies[i].x = ...`).
//!
//! A copy can be told apart once two places hold it and one is changed. So every value of such a
//! type that comes from a place (not made fresh: a literal, or a user function's result, whose
//! `return`s are checked here too) may only be read in place (`p.x`), compared, printed, or bound
//! to a local that is itself only read, isn't captured by a closure, and lives while nothing
//! changes a record (no store, no call that might make one). Anything else (stored, returned,
//! pushed, spread, captured, changed through) makes the type an object. Unrecognized uses count as
//! copies, so mistakes cost speed, not behaviour.

use super::facts::{Callee, IdentFact, ModuleFacts};
use super::*;
use crate::ast::{ArrowBody, ExprKind, MemberKind, StmtKind};

#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    /// Read where it is (a field read, a comparison, printed, discarded).
    Free,
    /// Copied into another place.
    Copy,
}

/// A local that may share its value with a place: a parameter, a `for...of` variable, or a local
/// initialized from a place. Fine if nothing can tell: `region` is what runs while it lives.
struct Alias {
    key: u32,
    region: Vec<ExprId>,
}

struct Walk<'w, 'a> {
    c: &'w Checker<'a>,
    m: u32,
    ast: &'a Ast,
    facts: &'w ModuleFacts,
    cands: &'w HashSet<TyId>,
    /// Candidates found to be shared: objects after all.
    out: HashSet<TyId>,
    /// A value of a type parameter copied (in a generic function or class): its instances'
    /// candidates are objects (see `finish`).
    generic_fns: HashSet<(u32, u32)>,
    generic_classes: HashSet<u32>,
    item: Option<u32>,
    class: Option<u32>,
    func: u32,
    funcs: u32,
    owner: HashMap<u32, u32>,
    owned: HashSet<u32>,
    aliases: Vec<Alias>,
    /// Locals changed in place (a store through record fields rooted at them), and locals used
    /// in a closure other than the function declaring them.
    rooted: HashSet<u32>,
    captured: HashSet<u32>,
    local_ty: HashMap<u32, TyId>,
    may_mutate: &'w mut HashMap<(u32, u32), bool>,
}

/// Array (and map, set) methods whose result holds the receiver's elements: copies of them.
const COPYING: &[&str] = &["slice", "concat", "filter", "flat", "flatMap", "toSorted", "toReversed", "toSpliced", "with", "entries", "values", "copyWithin", "fill"];

impl Checker<'_> {
    /// The mutated record types (see `mutated`) that must be objects: shared somewhere.
    /// The generic module function (module, item) whose body has call `e` in it.
    fn enclosing_generic_fn(&self, m: u32, e: ExprId) -> Option<(u32, u32)> {
        let ast = &self.modules[m as usize].ast;
        let sp = ast.expr(e).span;
        ast.items.iter().enumerate().find_map(|(i, it)| match &it.kind {
            ast::ItemKind::Function(f) if !f.tparams.is_empty() && it.span.file == sp.file && it.span.start <= sp.start && sp.end <= it.span.end => Some((m, i as u32)),
            _ => None,
        })
    }

    pub(super) fn shared_records(&self, cands: &HashSet<TyId>) -> HashSet<TyId> {
        let mut out = HashSet::default();
        let Some(facts) = &self.facts else { return out };
        if cands.is_empty() {
            return out;
        }
        let mut may_mutate = HashMap::default();
        let (mut generic_fns, mut generic_classes) = (HashSet::default(), HashSet::default());
        for m in 0..self.modules.len() as u32 {
            let ast = &self.modules[m as usize].ast;
            let mut w = Walk {
                c: self,
                m,
                ast,
                facts: &facts[m as usize],
                cands,
                out: HashSet::default(),
                generic_fns: HashSet::default(),
                generic_classes: HashSet::default(),
                item: None,
                class: None,
                func: 0,
                funcs: 0,
                owner: HashMap::default(),
                owned: HashSet::default(),
                aliases: Vec::new(),
                rooted: HashSet::default(),
                captured: HashSet::default(),
                local_ty: HashMap::default(),
                may_mutate: &mut may_mutate,
            };
            w.module();
            w.finish();
            out.extend(w.out);
            generic_fns.extend(w.generic_fns);
            generic_classes.extend(w.generic_classes);
        }
        // Generic code that copies a `T`: its instances' types carry their copies.
        let all = |c: &Self, t: TyId, out: &mut HashSet<TyId>| {
            let (mut ts, mut generic) = (Vec::new(), false);
            carried(c, cands, t, &mut ts, &mut generic, true, 0);
            if generic {
                out.extend(cands.iter().copied());
            }
            out.extend(ts);
        };
        if !generic_fns.is_empty() {
            // (a generic function passing its own `T` to one that copies a `T` copies it too:
            // its callers' types, not every candidate, carry the copies)
            let generic_targ = |c: &Self, call: &crate::check::facts::CallFact| {
                call.targs.iter().any(|&(_, t)| {
                    let (mut ts, mut g) = (Vec::new(), false);
                    carried(c, cands, t, &mut ts, &mut g, true, 0);
                    g
                })
            };
            loop {
                let mut more = Vec::new();
                for (m, f) in facts.iter().enumerate() {
                    for (&e, call) in &f.calls {
                        if let Callee::Fn(fm, fi) = call.callee
                            && generic_fns.contains(&(fm, fi))
                            && generic_targ(self, call)
                            && let Some(g) = self.enclosing_generic_fn(m as u32, e)
                            && !generic_fns.contains(&g)
                        {
                            more.push(g);
                        }
                    }
                }
                if more.is_empty() {
                    break;
                }
                generic_fns.extend(more);
            }
            for (m, f) in facts.iter().enumerate() {
                for (&e, call) in &f.calls {
                    if let Callee::Fn(fm, fi) = call.callee
                        && generic_fns.contains(&(fm, fi))
                    {
                        if generic_targ(self, call) && self.enclosing_generic_fn(m as u32, e).is_some() {
                            continue;
                        }
                        for &(_, t) in &call.targs {
                            all(self, t, &mut out);
                        }
                    }
                }
            }
        }
        if !generic_classes.is_empty() {
            for i in 0..self.types.count() {
                if let Ty::Class(c, args) = *self.types.get(TyId(i))
                    && generic_classes.contains(&c)
                {
                    for &t in self.types.tys(args) {
                        all(self, t, &mut out);
                    }
                }
            }
        }
        out
    }
}

/// The candidate records a value of `t` holds by value (its own type, fields of records, union
/// members; with `deep`, also inside arrays, maps, sets and promises). `generic`: a type parameter.
fn carried(c: &Checker, cands: &HashSet<TyId>, t: TyId, out: &mut Vec<TyId>, generic: &mut bool, deep: bool, depth: u32) {
    if depth > 16 {
        return;
    }
    match *c.types.get(t) {
        Ty::Record(fs) => {
            if cands.contains(&t) {
                out.push(t);
            }
            for f in c.types.fields(fs) {
                carried(c, cands, f.ty, out, generic, deep, depth + 1);
            }
        }
        Ty::Union(ms) => {
            for &m in c.types.tys(ms) {
                carried(c, cands, m, out, generic, deep, depth + 1);
            }
        }
        Ty::Param(_) => *generic = true,
        Ty::Array(e) | Ty::Set(e) | Ty::Promise(e, _) if deep => carried(c, cands, e, out, generic, deep, depth + 1),
        Ty::Map(k, v) if deep => {
            carried(c, cands, k, out, generic, deep, depth + 1);
            carried(c, cands, v, out, generic, deep, depth + 1);
        }
        _ => {}
    }
}

impl Walk<'_, '_> {
    fn ty(&self, e: ExprId) -> TyId {
        self.facts.expr_ty.get(e as usize).copied().unwrap_or(ERROR)
    }

    fn carried(&self, t: TyId, deep: bool) -> (Vec<TyId>, bool) {
        let (mut ts, mut generic) = (Vec::new(), false);
        carried(self.c, self.cands, t, &mut ts, &mut generic, deep, 0);
        (ts, generic)
    }

    /// A value of type `t` (from a place) is copied: the records it holds are shared.
    fn copied(&mut self, t: TyId, deep: bool) {
        let (ts, generic) = self.carried(t, deep);
        self.out.extend(ts);
        if generic {
            match (self.class, self.item) {
                (Some(c), _) => {
                    self.generic_classes.insert(c);
                }
                (None, Some(i)) => {
                    self.generic_fns.insert((self.m, i));
                }
                // (a generic copy we can't follow: every candidate)
                (None, None) => self.out.extend(self.cands.iter().copied()),
            }
        }
    }

    fn module(&mut self) {
        let ast = self.ast;
        for (ii, item) in ast.items.iter().enumerate() {
            self.item = Some(ii as u32);
            self.class = None;
            match &item.kind {
                ItemKind::Function(f) => self.function(&f.params, f.body),
                ItemKind::Class(cd) => {
                    if let Some(Decl::Class(c)) = self.c.scopes[self.m as usize].types.get(&cd.name).map(|d| d.0) {
                        self.class = Some(c);
                    } else {
                        // (no class to follow generic copies through: every candidate)
                        self.class = None;
                        self.item = None;
                    }
                    for mem in &cd.members {
                        match &mem.kind {
                            MemberKind::Field { init: Some(e), .. } => {
                                self.enter_func();
                                self.expr(*e, Ctx::Copy);
                            }
                            MemberKind::Method(f) | MemberKind::Getter(f) | MemberKind::Constructor(f) => self.function(&f.params, f.body),
                            _ => {}
                        }
                    }
                }
                ItemKind::Const { init, .. } => {
                    self.enter_func();
                    self.expr(*init, Ctx::Copy);
                }
                ItemKind::Test { body, .. } => {
                    self.enter_func();
                    self.expr(*body, Ctx::Free);
                }
                _ => {}
            }
        }
    }

    fn enter_func(&mut self) -> u32 {
        self.funcs += 1;
        let prev = self.func;
        self.func = self.funcs;
        prev
    }

    /// A function body: its parameters are aliases of the arguments, living through the body.
    fn function(&mut self, params: &[crate::ast::Param], body: StmtId) {
        let prev = self.enter_func();
        let mut region = Vec::new();
        crate::codegen::body::collect_exprs_stmt_pub(self.ast, body, &mut region);
        self.params(params, region);
        self.stmt(body);
        self.func = prev;
    }

    fn params(&mut self, params: &[crate::ast::Param], region: Vec<ExprId>) {
        for p in params {
            let key = p.span.start;
            self.owner.insert(key, self.func);
            if p.inout {
                // (the caller's own place, not a copy)
                self.owned.insert(key);
            } else {
                self.aliases.push(Alias { key, region: region.clone() });
            }
        }
    }

    fn stmts(&mut self, ss: &[StmtId]) {
        for (i, &s) in ss.iter().enumerate() {
            if let StmtKind::Let { name_span, init: Some(e), .. } = &self.ast.stmt(s).kind {
                let key = name_span.start;
                self.owner.insert(key, self.func);
                let bt = self.facts.bindings.get(&s).copied().unwrap_or(ERROR);
                let (bc, _) = self.carried(bt, false);
                let (ec, eg) = self.carried(self.ty(*e), false);
                if ec.iter().all(|t| bc.contains(t)) && !eg {
                    if self.fresh(*e) {
                        self.owned.insert(key);
                    } else {
                        let mut region = Vec::new();
                        for &r in &ss[i + 1..] {
                            crate::codegen::body::collect_exprs_stmt_pub(self.ast, r, &mut region);
                        }
                        self.aliases.push(Alias { key, region });
                    }
                    self.expr(*e, Ctx::Free);
                } else {
                    // (converted on the way: a copy)
                    self.expr(*e, Ctx::Copy);
                }
                continue;
            }
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: StmtId) {
        let ast = self.ast;
        match &ast.stmt(s).kind {
            StmtKind::Block(ss) => self.stmts(ss),
            StmtKind::Let { name_span, init, .. } => {
                // (not directly in a block, e.g. a `for` initializer: an alias living anywhere)
                let key = name_span.start;
                self.owner.insert(key, self.func);
                if let Some(e) = init {
                    if self.fresh(*e) {
                        self.owned.insert(key);
                        self.expr(*e, Ctx::Free);
                    } else {
                        self.expr(*e, Ctx::Copy);
                    }
                } else {
                    self.owned.insert(key);
                }
            }
            StmtKind::Expr(e) => self.expr(*e, Ctx::Free),
            StmtKind::If(c, t, e) => {
                self.expr(*c, Ctx::Free);
                self.stmt(*t);
                if let Some(e) = e {
                    self.stmt(*e);
                }
            }
            StmtKind::While(c, b) | StmtKind::DoWhile(b, c) => {
                self.expr(*c, Ctx::Free);
                self.stmt(*b);
            }
            StmtKind::For { init, cond, step, body } => {
                if let Some(i) = init {
                    self.stmt(*i);
                }
                for e in [cond, step].into_iter().flatten() {
                    self.expr(*e, Ctx::Free);
                }
                self.stmt(*body);
            }
            StmtKind::ForOf { name_span, iter, body, .. } => {
                let key = name_span.start;
                self.owner.insert(key, self.func);
                let mut region = Vec::new();
                crate::codegen::body::collect_exprs_stmt_pub(ast, *body, &mut region);
                self.aliases.push(Alias { key, region });
                // (`for (const v of m.values())`: the variable is checked, not the iterator)
                if let ExprKind::Call { callee, args, .. } = &ast.expr(*iter).kind
                    && args.is_empty()
                    && let ExprKind::Member { obj, name, .. } = &ast.expr(*callee).kind
                    && matches!(self.c.interner.get(*name), "values" | "keys" | "entries")
                    && matches!(self.facts.calls.get(iter).map(|c| &c.callee), Some(Callee::Method(_)))
                {
                    self.expr(*obj, Ctx::Free);
                } else {
                    self.expr(*iter, Ctx::Free);
                }
                self.stmt(*body);
            }
            StmtKind::Switch(d, cases) => {
                self.expr(*d, Ctx::Free);
                for c in cases {
                    if let Some(t) = c.test {
                        self.expr(t, Ctx::Free);
                    }
                    for &x in &c.body {
                        self.stmt(x);
                    }
                }
            }
            StmtKind::Return(Some(e)) | StmtKind::Throw(e) => self.expr(*e, Ctx::Copy),
            StmtKind::Try { body, catch, finally } => {
                self.stmt(*body);
                if let Some(c) = catch {
                    self.stmt(c.body);
                }
                if let Some(f) = finally {
                    self.stmt(*f);
                }
            }
            StmtKind::InitGlobal(i) => {
                if let ItemKind::Const { init, .. } = &ast.items[*i as usize].kind {
                    self.expr(*init, Ctx::Copy);
                }
            }
            _ => {}
        }
    }

    /// Is `e`'s value made fresh (no other place holds it)?
    fn fresh(&self, e: ExprId) -> bool {
        match &self.ast.expr(e).kind {
            ExprKind::Object(_) | ExprKind::Array(_) => true,
            ExprKind::Paren(x) | ExprKind::NonNull(x) | ExprKind::As(x, _) | ExprKind::Try(x) | ExprKind::Await(x) | ExprKind::Yield(Some(x), _) => self.fresh(*x),
            ExprKind::Cond(_, a, b) | ExprKind::Binary(crate::ast::BinOp::And | crate::ast::BinOp::Or | crate::ast::BinOp::Nullish, a, b) => self.fresh(*a) && self.fresh(*b),
            ExprKind::Undefined | ExprKind::Null => true,
            ExprKind::Call { .. } | ExprKind::New { .. } => match self.facts.calls.get(&e).map(|c| &c.callee) {
                // (user code: its `return`s are checked)
                Some(Callee::Fn(..) | Callee::Value | Callee::ClassMethod { .. } | Callee::StaticMethod(..) | Callee::New(_) | Callee::SuperCtor(_)) => true,
                Some(Callee::Builtin { ns: Some(ns), name }) => self.c.interner.get(*ns) == "JSON" && self.c.interner.get(*name) == "parse",
                _ => false,
            },
            _ => false,
        }
    }

    fn expr(&mut self, e: ExprId, ctx: Ctx) {
        if ctx == Ctx::Copy && !self.fresh(e) {
            let t = self.ty(e);
            self.copied(t, false);
        }
        let ast = self.ast;
        match &ast.expr(e).kind {
            ExprKind::Ident(_) => {
                if let Some(IdentFact::Local(k)) = self.facts.idents.get(&e) {
                    let t = self.ty(e);
                    self.local_ty.entry(*k).or_insert(t);
                    if self.owner.get(k).is_some_and(|&f| f != self.func) {
                        self.captured.insert(*k);
                    }
                }
            }
            ExprKind::Member { obj, .. } => self.expr(*obj, Ctx::Free),
            ExprKind::Index { obj, index, .. } => {
                self.expr(*obj, Ctx::Free);
                self.expr(*index, Ctx::Free);
            }
            ExprKind::Paren(x) | ExprKind::NonNull(x) | ExprKind::As(x, _) | ExprKind::Try(x) => self.expr(*x, ctx),
            ExprKind::Unary(_, x) | ExprKind::Typeof(x) | ExprKind::Await(x) | ExprKind::Yield(Some(x), _) => self.expr(*x, Ctx::Free),
            ExprKind::Spread(x) => {
                let t = self.ty(*x);
                self.copied(t, true);
                self.expr(*x, Ctx::Free);
            }
            ExprKind::Binary(op, l, r) => {
                let inner = if matches!(op, crate::ast::BinOp::And | crate::ast::BinOp::Or | crate::ast::BinOp::Nullish) { ctx } else { Ctx::Free };
                self.expr(*l, inner);
                self.expr(*r, inner);
            }
            ExprKind::Cond(a, b, c) => {
                self.expr(*a, Ctx::Free);
                self.expr(*b, ctx);
                self.expr(*c, ctx);
            }
            ExprKind::Assign(_, l, r) => {
                self.target(*l);
                self.expr(*r, Ctx::Copy);
            }
            ExprKind::Update { target, .. } => self.target(*target),
            ExprKind::Template(_, xs) | ExprKind::Seq(xs) => xs.iter().for_each(|&x| self.expr(x, Ctx::Free)),
            ExprKind::Array(xs) => xs.iter().for_each(|&x| self.expr(x, Ctx::Copy)),
            ExprKind::Object(fs) => fs.iter().for_each(|f| self.expr(f.value, Ctx::Copy)),
            ExprKind::Call { callee, args, .. } | ExprKind::New { callee, args, .. } => self.call(e, *callee, args),
            ExprKind::Arrow(f) => {
                let prev = self.enter_func();
                let mut region = Vec::new();
                match &f.body {
                    ArrowBody::Expr(x) => crate::codegen::body::collect_exprs_expr(ast, *x, &mut region),
                    ArrowBody::Block(b) => crate::codegen::body::collect_exprs_stmt_pub(ast, *b, &mut region),
                }
                self.params(&f.params, region);
                match &f.body {
                    ArrowBody::Expr(x) => self.expr(*x, Ctx::Copy),
                    ArrowBody::Block(b) => self.stmt(*b),
                }
                self.func = prev;
            }
            _ => {}
        }
    }

    fn call(&mut self, e: ExprId, callee: ExprId, args: &[crate::ast::Arg]) {
        let ast = self.ast;
        let fact = self.facts.calls.get(&e);
        let callee_kind = fact.map(|f| f.callee.clone());
        // The receiver of a method: read in place, unless the method copies its elements out.
        if let ExprKind::Member { obj, name, .. } = &ast.expr(callee).kind {
            let n = self.c.interner.get(*name);
            if matches!(callee_kind, Some(Callee::Method(_))) && COPYING.contains(&n) {
                let t = self.ty(*obj);
                self.copied(t, true);
            }
            self.expr(*obj, Ctx::Free);
        } else {
            self.expr(callee, Ctx::Free);
        }
        for (i, a) in args.iter().enumerate() {
            let ctx = match &callee_kind {
                _ if a.by_ref.is_some() => Ctx::Free,
                // (printing and serializing keep nothing)
                Some(Callee::Builtin { ns: Some(ns), name }) if matches!((self.c.interner.get(*ns), self.c.interner.get(*name)), ("console", _) | ("JSON", "stringify")) => Ctx::Free,
                Some(Callee::Builtin { ns: None, name }) if self.c.interner.get(*name) == "String" => Ctx::Free,
                Some(Callee::Matcher(_)) => Ctx::Free,
                Some(Callee::Builtin { .. } | Callee::Method(_) | Callee::Js { .. } | Callee::JsNew | Callee::NewPromise) | None => {
                    // (a built-in may keep what it's given, or copy it into what it makes)
                    let t = self.ty(a.expr);
                    if !self.fresh(a.expr) {
                        self.copied(t, true);
                    }
                    Ctx::Free
                }
                // A user function's parameter is checked as an alias of what it's given, as long
                // as it holds the same records (not converted, e.g. into an interface).
                Some(_) => {
                    let pt = fact.and_then(|f| f.params.get(i).map(|p| p.ty).or(f.rest));
                    let (ac, ag) = self.carried(self.ty(a.expr), false);
                    match pt {
                        Some(pt) if !ag && ac.iter().all(|t| self.carried(pt, false).0.contains(t)) => Ctx::Free,
                        _ => Ctx::Copy,
                    }
                }
            };
            self.expr(a.expr, ctx);
        }
    }

    /// The target of an assignment: a store into the place's root value, unless the path passes
    /// through something shared (an array, map, class instance, recursive alias's box...).
    fn target(&mut self, l: ExprId) {
        let ast = self.ast;
        let mut cur = l;
        let mut shared = false;
        loop {
            match &ast.expr(cur).kind {
                ExprKind::Paren(x) | ExprKind::NonNull(x) => cur = *x,
                ExprKind::Member { obj, .. } => {
                    shared |= !self.value_step(self.ty(*obj));
                    cur = *obj;
                }
                ExprKind::Index { obj, index, .. } => {
                    self.expr(*index, Ctx::Free);
                    shared |= !self.value_step(self.ty(*obj));
                    cur = *obj;
                }
                ExprKind::Ident(_) => {
                    if let Some(IdentFact::Local(k)) = self.facts.idents.get(&cur) {
                        let t = self.ty(cur);
                        self.local_ty.entry(*k).or_insert(t);
                        if self.owner.get(k).is_some_and(|&f| f != self.func) {
                            self.captured.insert(*k);
                        }
                        // (changing the local itself, `x = v`, isn't changing its value in place)
                        if !shared && cur != l {
                            self.rooted.insert(*k);
                        }
                    }
                    return;
                }
                _ => {
                    self.expr(cur, Ctx::Free);
                    return;
                }
            }
        }
    }

    /// A store through a value of type `t` changes that value itself (a record that's a value),
    /// not something shared (an array, map, class instance, object...).
    fn value_step(&self, t: TyId) -> bool {
        let ms: Vec<TyId> = self.c.types.members(t).into_iter().filter(|&m| !matches!(self.c.types.get(m), Ty::Undefined | Ty::Null)).collect();
        !ms.is_empty() && ms.iter().all(|&m| matches!(self.c.types.get(m), Ty::Record(_)) && !self.c.objects.contains(&m))
    }

    /// Could running `exprs` change a record in place (a store, or a call that might)?
    fn region_mutates(&mut self, exprs: &[ExprId]) -> bool {
        let ast = self.ast;
        for &x in exprs {
            match &ast.expr(x).kind {
                ExprKind::Assign(_, l, _) | ExprKind::Update { target: l, .. } => {
                    if let ExprKind::Member { obj, .. } | ExprKind::Index { obj, .. } = &ast.expr(*l).kind {
                        let t = self.ty(*obj);
                        let (ts, generic) = self.carried(t, false);
                        if !ts.is_empty() || generic {
                            return true;
                        }
                    }
                }
                ExprKind::Await(_) => return true,
                ExprKind::Call { args, .. } | ExprKind::New { args, .. } => {
                    let callbacks = args.iter().any(|a| matches!(self.c.types.get(self.ty(a.expr)), Ty::Func(..)));
                    match self.facts.calls.get(&x).map(|f| f.callee.clone()) {
                        Some(Callee::Builtin { .. } | Callee::Method(_)) if !callbacks => {}
                        Some(Callee::Matcher(_)) => {}
                        Some(Callee::Fn(fm, fi)) if !callbacks => {
                            if self.fn_mutates(fm, fi) {
                                return true;
                            }
                        }
                        _ => return true,
                    }
                }
                _ => {}
            }
        }
        false
    }

    fn fn_mutates(&mut self, m: u32, i: u32) -> bool {
        if let Some(&r) = self.may_mutate.get(&(m, i)) {
            return r;
        }
        self.may_mutate.insert((m, i), false);
        let ast = &self.c.modules[m as usize].ast;
        let ItemKind::Function(f) = &ast.items[i as usize].kind else { return true };
        let mut exprs = Vec::new();
        crate::codegen::body::collect_exprs_stmt_pub(ast, f.body, &mut exprs);
        let r = if m == self.m {
            self.region_mutates(&exprs)
        } else {
            // (another module's function: checked with its own facts)
            let facts = &self.c.facts.as_ref().unwrap()[m as usize];
            let mut w = Walk {
                c: self.c,
                m,
                ast,
                facts,
                cands: self.cands,
                out: HashSet::default(),
                generic_fns: HashSet::default(),
                generic_classes: HashSet::default(),
                item: None,
                class: None,
                func: 0,
                funcs: 0,
                owner: HashMap::default(),
                owned: HashSet::default(),
                aliases: Vec::new(),
                rooted: HashSet::default(),
                captured: HashSet::default(),
                local_ty: HashMap::default(),
                may_mutate: self.may_mutate,
            };
            w.region_mutates(&exprs)
        };
        self.may_mutate.insert((m, i), r);
        r
    }

    /// Checks the module's aliases, and locals changed in place or captured.
    fn finish(&mut self) {
        let aliases = std::mem::take(&mut self.aliases);
        let mut alias_keys = HashSet::default();
        for a in &aliases {
            alias_keys.insert(a.key);
            let t = self.local_ty.get(&a.key).copied();
            let Some(t) = t else { continue };
            let (ts, generic) = self.carried(t, false);
            if ts.is_empty() && !generic {
                continue;
            }
            if self.rooted.contains(&a.key) || self.captured.contains(&a.key) || self.region_mutates(&a.region) {
                self.copied(t, false);
            }
        }
        // Owned values changed while a closure holds them, and locals of unknown origin.
        let keys: Vec<u32> = self.rooted.iter().chain(self.captured.iter()).copied().collect();
        for k in keys {
            if alias_keys.contains(&k) {
                continue;
            }
            let owned_ok = self.owned.contains(&k) && !(self.rooted.contains(&k) && self.captured.contains(&k));
            if !owned_ok && let Some(&t) = self.local_ty.get(&k) {
                self.copied(t, false);
            }
        }
    }
}
