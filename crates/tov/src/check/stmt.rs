use super::*;
use crate::ast::{ArrowBody, AssignOp, BinOp, Case, ExprKind, StmtKind};

enum SwitchMode {
    /// `switch (x.kind)` over a discriminated union: variants are (literal, member type).
    Discriminated { local: LocalId, variants: Vec<(Sym, TyId)> },
    /// `switch (x)` where `x` is a union of string literals.
    Literal { local: Option<LocalId>, lits: Vec<Sym> },
    General,
}

impl<'a> Checker<'a> {
    /// Named local functions that call each other (`function isEven` / `function isOdd` in a
    /// body): each is declared, with the type its annotations give, before any is checked; all are
    /// checked again while what they throw changes.
    fn closure_group(&mut self, group: &[StmtId]) {
        let tscope = self.tscope();
        let mut pre = Vec::new();
        for &st in group {
            let StmtKind::Let { name, name_span, init: Some(i), .. } = self.ast().stmt(st).kind else { continue };
            let ExprKind::Arrow(f) = &self.ast().expr(i).kind else { continue };
            let t = match (f.ret, f.params.iter().all(|p| p.ty.is_some())) {
                (Some(rt), true) => {
                    let ps: Vec<(crate::ast::TypeId, bool, bool)> = f.params.iter().map(|p| (p.ty.unwrap(), p.inout, p.optional)).collect();
                    let throws = f.throws;
                    let params: Vec<FnParam> = ps
                        .into_iter()
                        .map(|(t, inout, optional)| {
                            let ty = self.resolve_type(t, &tscope);
                            let ty = if optional { self.types.optional(ty) } else { ty };
                            FnParam { ty, inout, optional }
                        })
                        .collect();
                    let ret = self.resolve_type(rt, &tscope);
                    let throws = match throws {
                        Some(t) => self.resolve_type(t, &tscope),
                        None => NEVER,
                    };
                    self.types.func_throws(params, ret, throws)
                }
                _ => {
                    let n = self.name(name).to_string();
                    self.report(Diagnostic::new("T0303", name_span, format!("`{n}` and the functions beside it call each other, so its parameters and result need types")));
                    ERROR
                }
            };
            let id = self.declare(name, t, LocalKind::Const, name_span, None, false);
            pre.push((id, name));
        }
        if let Some(f) = self.facts_mut() {
            f.closure_groups.insert(group[0], group.to_vec());
        }
        let marks = self.diags.len();
        let saved = self.rewrites.clone();
        for round in 0..3 {
            for &st in group {
                self.stmt(st);
            }
            let mut changed = false;
            for &(id, name) in &pre {
                let found = self.lookup(name).map(|(_, t)| t).unwrap_or(ERROR);
                if found != ERROR && self.fcx().locals[id].ty != ERROR && self.fcx().locals[id].ty != found {
                    self.fcx().locals[id].ty = found;
                    changed = true;
                }
            }
            if !changed || round == 2 {
                break;
            }
            self.diags.truncate(marks);
            self.rewrites = saved.clone();
        }
    }

    /// `[]`, or `new Array(n)` / `Array(n)` without its element type.
    fn untyped_array(&self, i: ExprId) -> bool {
        match &self.ast().expr(i).kind {
            // (`[[]]`, `[[], []]` too: arrays of empty arrays, `graph = [[]]` passed as a `number[][]`)
            ExprKind::Array(xs) => xs.iter().all(|&x| matches!(&self.ast().expr(x).kind, ExprKind::Array(inner) if inner.is_empty())),
            ExprKind::New { callee, type_args, args } => {
                type_args.is_empty() && args.len() == 1 && matches!(self.ast().expr(*callee).kind, ExprKind::Ident(n) if self.name(n) == "Array" && self.lookup(n).is_none())
            }
            _ => self.filled_with_nothing(i).is_some(),
        }
    }

    /// `name[k] = v` somewhere after `name` is declared (at `at`): it's a dictionary.
    fn stored_by_key(&self, name: Sym, at: Span) -> bool {
        let ast = self.ast();
        ast.exprs.iter().any(|e| {
            e.span.file == at.file
                && e.span.start > at.start
                && matches!(&e.kind, ExprKind::Assign(_, t, _) if matches!(&ast.expr(*t).kind, ExprKind::Index { obj, .. } if matches!(ast.expr(*obj).kind, ExprKind::Ident(n) if n == name)))
        })
    }

    /// `new Map()` (`Some(true)`) or `new Set()` (`Some(false)`), with neither type arguments nor
    /// entries.
    fn untyped_collection(&self, i: ExprId) -> Option<bool> {
        // (`const seen = {}` stored into by key: a map, as JavaScript's object of keys)
        if matches!(&self.ast().expr(i).kind, ExprKind::Object(fs) if fs.is_empty()) {
            return Some(true);
        }
        let ExprKind::New { callee, type_args, args } = &self.ast().expr(i).kind else { return None };
        if !type_args.is_empty() || !args.is_empty() {
            return None;
        }
        match self.ast().expr(*callee).kind {
            ExprKind::Ident(n) if self.lookup(n).is_none() && self.name(n) == "Map" => Some(true),
            ExprKind::Ident(n) if self.lookup(n).is_none() && self.name(n) == "Set" => Some(false),
            _ => None,
        }
    }

    /// `Array(n).fill(null)` (or `undefined`): what it's filled with, the first of what it holds
    /// (TypeScript's `any[]`: what's stored in it later counts too).
    pub(super) fn filled_with_nothing(&self, i: ExprId) -> Option<TyId> {
        let ast = self.ast();
        let ExprKind::Call { callee, args, .. } = &ast.expr(i).kind else { return None };
        let ExprKind::Member { obj, name, .. } = &ast.expr(*callee).kind else { return None };
        if self.name(*name) != "fill" || args.len() != 1 || !matches!(&ast.expr(*obj).kind, ExprKind::New { .. }) || !self.untyped_array(*obj) {
            return None;
        }
        match ast.expr(args[0].expr).kind {
            ExprKind::Null => Some(NULL),
            ExprKind::Undefined => Some(UNDEFINED),
            _ => None,
        }
    }

    /// Does a closure in `init` refer to `name`?
    fn closure_refers(&self, init: ExprId, name: Sym) -> bool {
        let ast = self.ast();
        let span = ast.expr(init).span;
        let arrows: Vec<Span> = ast.exprs.iter().filter(|e| matches!(e.kind, ExprKind::Arrow(_)) && e.span.file == span.file && e.span.start >= span.start && e.span.end <= span.end).map(|e| e.span).collect();
        !arrows.is_empty() && ast.exprs.iter().any(|e| matches!(e.kind, ExprKind::Ident(n) if n == name) && arrows.iter().any(|a| e.span.file == a.file && e.span.start >= a.start && e.span.end <= a.end))
    }

    /// `const fib = (n: number): number => ... fib(n - 1) ...` (a nested `function fib` too): the
    /// name is visible in its own body, with the type its annotations give. Checked again if what
    /// it throws turns out otherwise. `None` if the arrow doesn't refer to itself.
    fn recursive_arrow(&mut self, name: Sym, name_span: Span, init: ExprId, kind: LocalKind) -> Option<TyId> {
        let ast = self.ast();
        let ExprKind::Arrow(f) = &ast.expr(init).kind else { return None };
        let span = ast.expr(init).span;
        if !ast.exprs.iter().any(|e| matches!(e.kind, ExprKind::Ident(n) if n == name) && e.span.file == span.file && e.span.start >= span.start && e.span.end <= span.end) {
            return None;
        }
        let tscope = self.tscope();
        let pre = match (f.ret, f.params.iter().all(|p| p.ty.is_some())) {
            (Some(rt), true) => {
                let params: Vec<FnParam> = f
                    .params
                    .iter()
                    .map(|p| {
                        let ty = self.resolve_type(p.ty.unwrap(), &tscope);
                        let ty = if p.optional { self.types.optional(ty) } else { ty };
                        FnParam { ty, inout: p.inout, optional: p.optional }
                    })
                    .collect();
                let ret = self.resolve_type(rt, &tscope);
                let throws = match f.throws {
                    Some(t) => self.resolve_type(t, &tscope),
                    None => NEVER,
                };
                if f.is_async {
                    let rspan = self.ast().ty(rt).span;
                    let inner = self.async_inner(ret, rspan);
                    let p = self.types.promise(inner, throws);
                    self.types.func(params, p)
                } else {
                    self.types.func_throws(params, ret, throws)
                }
            }
            // No result type: its calls to itself first give `never` (see `infer_recursive`).
            (None, true) if !f.is_async => {
                let params: Vec<FnParam> = f
                    .params
                    .iter()
                    .map(|p| {
                        let ty = self.resolve_type(p.ty.unwrap(), &tscope);
                        let ty = if p.optional { self.types.optional(ty) } else { ty };
                        FnParam { ty, inout: p.inout, optional: p.optional }
                    })
                    .collect();
                self.types.func(params, NEVER)
            }
            _ => {
                let n = self.name(name).to_string();
                self.report(Diagnostic::new("T0303", name_span, format!("`{n}` calls itself, so its parameters and result need types")));
                ERROR
            }
        };
        let id = self.declare(name, pre, kind, name_span, None, false);
        let marks = self.diags.len();
        let saved = self.rewrites.clone();
        let probing = f.ret.is_none() && pre != ERROR;
        let t = if probing {
            let before = self.arrow_probe.replace(span);
            let t = self.expr(init, None);
            self.arrow_probe = before;
            let ret = match *self.types.get(t) {
                Ty::Func(_, r, _) => r,
                _ => ERROR,
            };
            let rv = match &f.body {
                crate::ast::ArrowBody::Block(b) => super::returns_value(self.ast(), *b),
                crate::ast::ArrowBody::Expr(_) => true,
            };
            if ret == ERROR || ret == NEVER || (ret == VOID && rv) {
                self.diags.truncate(marks);
                self.rewrites = saved.clone();
                let n = self.name(name).to_string();
                self.report(Diagnostic::new("T0303", name_span, format!("`{n}` calls itself, and its result can't be told from the returns that don't, so it needs an explicit return type")));
                self.fcx().locals[id].ty = ERROR;
                return Some(ERROR);
            }
            self.diags.truncate(marks);
            self.rewrites = saved.clone();
            self.fcx().locals[id].ty = t;
            self.expr(init, None)
        } else {
            self.expr(init, None)
        };
        let mut t = t;
        for _ in 0..3 {
            if t == ERROR || pre == ERROR || t == self.fcx().locals[id].ty {
                break;
            }
            self.diags.truncate(marks);
            self.rewrites = saved.clone();
            self.fcx().locals[id].ty = t;
            t = self.expr(init, None);
        }
        Some(t)
    }

    pub(super) fn stmt(&mut self, s: StmtId) {
        // (a labelled loop: its label, for `break outer` / `continue outer` inside)
        let label = self.ast().labels.iter().find(|(st, _)| *st == s).map(|x| x.1);
        let in_fn = self.fcx.last().is_some_and(|f| !f.frames.is_empty());
        if let Some(l) = label.filter(|_| in_fn) {
            self.cur_frame().labels.push(l);
        }
        self.stmt_inner(s);
        if label.is_some() && in_fn {
            self.cur_frame().labels.pop();
        }
    }

    fn stmt_inner(&mut self, s: StmtId) {
        let ast = self.ast();
        let node = ast.stmt(s);
        let span = node.span;
        match &node.kind {
            StmtKind::Empty | StmtKind::Error => {}
            StmtKind::InitGlobal(ii) if !matches!(self.modules[self.cur as usize].ast.items[*ii as usize].kind, ItemKind::Const { .. }) => {}
            StmtKind::InitGlobal(ii) => {
                let m = self.cur;
                let t = self.const_type(m, *ii);
                // A top-level `const`/`let` lives in module scope, which `lookup` can't see —
                // give the script a marked local too, or `if (x !== undefined)` can't narrow
                // it (and the effects machinery can't track it). `Local::global` keeps its
                // reads and writes resolving as the global, so codegen is unaffected.
                if let ItemKind::Const { name, mutable, name_span, .. } = &self.modules[m as usize].ast.items[*ii as usize].kind {
                    let (name, mutable, name_span) = (*name, *mutable, *name_span);
                    let kind = if mutable { LocalKind::Let } else { LocalKind::Const };
                    let id = self.declare(name, t, kind, name_span, None, false);
                    self.fcx().locals[id].global = Some((m, *ii));
                }
                if self.const_throws.contains(&(m, *ii)) {
                    let err = self.error_class();
                    self.cur_frame().thrown.push(err);
                }
            }
            StmtKind::Expr(e) => {
                self.expr(*e, None);
            }
            StmtKind::Let { mutable, name, name_span, ty, init } => {
                let kw = Span::new(span.file, span.start, span.start + if *mutable { 3 } else { 5 });
                let kind = if *mutable { LocalKind::Let } else { LocalKind::Const };
                let tscope = self.tscope();
                // (`const x: any = v`, a `test.each` row's `(xs: any[]) => ...`: `v`'s type, as if
                // unannotated)
                let ty = &ty.filter(|&te| !(init.is_some() && self.mentions_any(te)));
                let (t, promotable) = match (ty, init) {
                    (Some(te), init) => {
                        let declared = self.resolve_type(*te, &tscope);
                        // (`const lev: F = memoize((i, j) => ... lev(i - 1, j) ...)`: a closure in
                        // its initializer calls it later, so the name is visible there)
                        if let Some(i) = init
                            && self.closure_refers(*i, *name)
                        {
                            self.declare(*name, declared, kind, *name_span, None, false);
                            if !matches!(self.ast().expr(*i).kind, ExprKind::Arrow(_))
                                && let Some(f) = self.facts_mut()
                            {
                                f.self_init.insert(name_span.start);
                            }
                        }
                        if let Some(i) = init {
                            let sites = self.type_sites(self.cur, *te);
                            self.flow(sites, |c| {
                                let found = c.expr(*i, Some(declared));
                                let ispan = c.ast().expr(*i).span;
                                c.expect_assignable(found, declared, ispan, None);
                            });
                        } else if !*mutable {
                            self.report(Diagnostic::new("T0309", span, "a `const` needs a value"));
                        }
                        (declared, false)
                    }
                    // `const xs = []`: its elements are what it's given later (`xs.push(x)`), as
                    // TypeScript's evolving arrays (found by a first check of the body, see
                    // `settle_evolving`)
                    // (`const g = Array(n)` / `new Array(n)` too)
                    (None, Some(i)) if self.untyped_array(*i) && !self.fcx.is_empty() => {
                        let key = name_span.start;
                        let elem = match self.fcx.last().unwrap().evolved.get(&key) {
                            Some(&e) => e,
                            None => {
                                let span = self.ast().expr(*i).span;
                                let given = self.filled_with_nothing(*i).into_iter().collect();
                                self.fcx().evolving.push((key, span, given, EV_ARRAY));
                                ERROR
                            }
                        };
                        let t = self.types.array(elem);
                        self.expr(*i, Some(t));
                        (t, false)
                    }
                    // `const m = new Map()`: its keys and values are what it's given later
                    // (`m.set(k, v)`), as TypeScript's `Map<any, any>` holds them
                    (None, Some(i)) if self.untyped_collection(*i).is_some() && !self.fcx.is_empty() && (!matches!(self.ast().expr(*i).kind, ExprKind::Object(_)) || self.stored_by_key(*name, *name_span)) => {
                        let key = name_span.start;
                        let t = match self.fcx.last().unwrap().evolved.get(&key) {
                            Some(&t) => t,
                            None => {
                                let span = self.ast().expr(*i).span;
                                self.fcx().evolving.push((key, span, Vec::new(), EV_COLLECTION));
                                if self.untyped_collection(*i) == Some(true) { self.types.intern(Ty::Map(ERROR, ERROR)) } else { self.types.intern(Ty::Set(ERROR)) }
                            }
                        };
                        if !self.types.mentions_error(t) {
                            self.expr(*i, Some(t));
                        }
                        (t, false)
                    }
                    // `let prev = null`: as `let prev;`, what's assigned to it too (TypeScript's
                    // evolving `let`)
                    (None, Some(i)) if *mutable && !self.fcx.is_empty() && matches!(self.ast().expr(*i).kind, ExprKind::Null | ExprKind::Undefined) => {
                        let key = name_span.start;
                        let start = if matches!(self.ast().expr(*i).kind, ExprKind::Null) { NULL } else { UNDEFINED };
                        let t = match self.fcx.last().unwrap().evolved.get(&key) {
                            Some(&t) => self.types.union(&[t, start]),
                            None => {
                                self.fcx().evolving.push((key, *name_span, vec![start], EV_LET));
                                ERROR
                            }
                        };
                        if t != ERROR {
                            self.expr(*i, Some(t));
                        }
                        (t, false)
                    }
                    (None, Some(i)) => {
                        // (`new C(...)` of a generic class: type arguments the constructor doesn't
                        // give are what the calls on it give, see `EV_CLASS`)
                        let is_new = matches!(&self.ast().expr(*i).kind, ExprKind::New { type_args, .. } if type_args.is_empty()) && !self.fcx.is_empty();
                        let evolved_class = if is_new { self.fcx.last().unwrap().evolved.get(&name_span.start).copied() } else { None };
                        self.lenient_new = (is_new && evolved_class.is_none()).then(|| self.ast().expr(*i).span);
                        self.lenient_hit = false;
                        let mut found = match self.recursive_arrow(*name, *name_span, *i, kind) {
                            Some(t) => t,
                            None => self.expr(*i, evolved_class),
                        };
                        self.lenient_new = None;
                        if std::mem::take(&mut self.lenient_hit) {
                            let span = self.ast().expr(*i).span;
                            self.fcx().evolving.push((name_span.start, span, Vec::new(), super::EV_CLASS));
                        }
                        if let Some(t) = evolved_class {
                            found = t;
                        }
                        // (`const v = m.get(k)` of an untyped map: TypeScript's `any`)
                        if let ExprKind::Call { callee, .. } = &self.ast().expr(*i).kind
                            && let ExprKind::Member { obj, name: g, .. } = self.ast().expr(*callee).kind
                            && self.name(g) == "get"
                            && self.untyped_map(obj)
                        {
                            self.fcx().loose.insert(name_span.start);
                        }
                        // (an object literal given where a dictionary goes: a map)
                        if self.spec.float_globals.contains(&super::spec::dict_key(self.cur, 0x8000_0000 | name_span.start)) {
                            found = self.as_dict(*i, found);
                        }
                        let ispan = self.ast().expr(*i).span;
                        let t = self.check_inferred_binding(found, ispan);
                        let t = if *mutable { self.types.widen(t) } else { t };
                        // (in TypeScript, any `let` of an integer: `let min = INF` may take a float
                        // later, its one `number`)
                        let promotable = *mutable && t == INT && (self.is_int_literal(*i) || self.ts_module());
                        if promotable && self.fcx.last().unwrap().promoted.contains(&name_span.start) {
                            (F64, false)
                        } else {
                            (t, promotable)
                        }
                    }
                    // `let x`: what's assigned to it, as TypeScript's evolving `let` (see
                    // `settle_evolving`)
                    (None, None) if *mutable && !self.fcx.is_empty() => {
                        let key = name_span.start;
                        // (`undefined` until assigned: assignments narrow it, as TypeScript's do)
                        match self.fcx.last().unwrap().evolved.get(&key) {
                            Some(&t) => (self.types.union(&[t, UNDEFINED]), false),
                            None => {
                                self.fcx().evolving.push((key, *name_span, Vec::new(), EV_LET));
                                (ERROR, false)
                            }
                        }
                    }
                    (None, None) => {
                        let n = self.name(*name).to_string();
                        self.report(
                            Diagnostic::new("T0301", *name_span, format!("`{n}` needs a type annotation or an initial value"))
                                .fix(Applicability::Placeholder, format!("annotate `{n}: T`"), name_span.empty_at_end(), ": T"),
                        );
                        (ERROR, false)
                    }
                };
                self.rec_binding(s, t);
                let id = self.declare(*name, t, kind, *name_span, if *mutable { None } else { Some(kw) }, promotable);
                // (`const done = $t.done` of `const { done, value } = r`: see `field_aliases`)
                if !*mutable
                    && let Some(i) = init
                    && let ExprKind::Member { obj, name: field, optional: false, .. } = self.ast().expr(*i).kind
                    && let ExprKind::Ident(tmp) = self.ast().expr(obj).kind
                    && let Some((tid, _)) = self.lookup(tmp)
                    && self.fcx().locals[tid].kind == LocalKind::Const
                {
                    self.fcx().field_aliases.push((id, obj, field));
                }
                if self.spec.on {
                    let sites = match (ty, init) {
                        (Some(te), _) => self.type_sites(self.cur, *te),
                        (None, Some(i)) => self.sources(self.ast().expr(*i).span),
                        _ => Vec::new(),
                    };
                    self.fcx().locals[id].sites = sites;
                }
            }
            StmtKind::Block(stmts) => {
                let fx = if self.always_jumps(s) { Effects::default() } else { effects_of_stmts(ast, stmts) };
                let entry = self.fcx().locals.len();
                let ptab = self.fcx().path_table.len();
                self.push_scope();
                let mut reported_unreachable = false;
                let groups = closure_groups(ast, stmts);
                let mut i = 0;
                while i < stmts.len() {
                    if let Some(g) = groups.iter().find(|g| g[0] == stmts[i]) {
                        self.closure_group(g);
                        i += g.len();
                        continue;
                    }
                    let st = stmts[i];
                    i += 1;
                    let i = i - 1;
                    // (a nested function after a `return` is still declared: hoisting)
                    let local_fn = matches!(&ast.stmt(st).kind, StmtKind::Let { init: Some(x), .. } if matches!(ast.expr(*x).kind, ExprKind::Arrow(_)));
                    if i > 0 && !reported_unreachable && !local_fn && self.always_jumps(stmts[i - 1]) {
                        let sp = self.ast().stmt(st).span;
                        self.report(Diagnostic::new("F0102", sp, "unreachable code").note("why", "the previous statement always returns, breaks or continues"));
                        reported_unreachable = true;
                    }
                    // `const u = db[id]` tested later (`if (u)`, `u ?? d`): the read may find
                    // nothing, `undefined`, as in JavaScript (rather than trapping)
                    if let StmtKind::Let { ty: None, init: Some(x), name, .. } = &ast.stmt(st).kind
                        && matches!(ast.expr(*x).kind, ExprKind::Index { .. })
                        && tested(ast, *name, &stmts[i + 1..])
                    {
                        self.want_optional = true;
                    }
                    // `const peek = () => toks[pos]` whose results are tested (`const t = peek();
                    // if (t ...)`): the read may find nothing too
                    if let StmtKind::Let { ty: None, init: Some(x), name, .. } = &ast.stmt(st).kind
                        && let ExprKind::Arrow(f) = &ast.expr(*x).kind
                        && f.ret.is_none()
                        && let ArrowBody::Expr(body) = f.body
                        && matches!(ast.expr(body).kind, ExprKind::Index { .. })
                        && results_tested(ast, *name, &stmts[i + 1..])
                    {
                        let m = self.cur;
                        self.absent_reads.insert((m, body));
                    }
                    self.stmt(st);
                    // `let x: number;` read before anything is assigned to it
                    if let StmtKind::Let { mutable: true, ty: Some(_), init: None, name, .. } = &ast.stmt(st).kind
                        && let Some((_, t)) = self.lookup(*name)
                        && !matches!(t, ERROR | UNKNOWN | JS | VOID)
                        && !self.types.has_undefined(t)
                    {
                        let n = self.name(*name).to_string();
                        for sp in assigned::unassigned_reads(ast, *name, &stmts[i + 1..]) {
                            self.report(
                                Diagnostic::new("T0310", sp, format!("`{n}` is read before anything is assigned to it"))
                                    .note("why", "on some path to here nothing has been assigned to it yet"),
                            );
                        }
                    }
                }
                let ends = self.end_types(&fx.assigned, entry);
                let paths = self.end_paths(ptab, entry);
                self.pop_scope();
                self.apply_effects(&fx, entry, &ends);
                // (what the block's statements narrowed last, in order, still holds after it)
                for (l, p, t, heap) in paths {
                    self.apply(&[super::expr::Narrowing::Path(l, p, t, heap)]);
                }
            }
            StmtKind::If(c, then, els) => {
                self.cond(*c);
                let pos = self.narrowings(*c, true);
                let neg = self.narrowings(*c, false);
                let then_jumps = self.always_jumps(*then);
                let else_jumps = els.map(|e| self.always_jumps(e)).unwrap_or(false);
                // Effects of the branches that fall through reach the code after the `if`.
                let mut fx = Effects::default();
                if !then_jumps {
                    fx.extend(effects_of_stmt(ast, *then));
                }
                if let Some(e) = els
                    && !else_jumps
                {
                    fx.extend(effects_of_stmt(ast, *e));
                }
                let entry = self.fcx().locals.len();
                let ptab = self.fcx().path_table.len();
                let mut branch_ends: Vec<Vec<(LocalId, TyId)>> = Vec::new();
                let mut path_ends: Vec<Vec<(LocalId, Vec<Sym>, TyId, bool)>> = Vec::new();
                self.push_scope();
                self.apply(&pos);
                self.stmt(*then);
                if !then_jumps {
                    branch_ends.push(self.end_types(&fx.assigned, entry));
                    path_ends.push(self.end_paths(ptab, entry));
                }
                self.pop_scope();
                self.push_scope();
                self.apply(&neg);
                if let Some(e) = els {
                    self.stmt(*e);
                }
                if !else_jumps {
                    branch_ends.push(self.end_types(&fx.assigned, entry));
                    path_ends.push(self.end_paths(ptab, entry));
                }
                self.pop_scope();
                // Early exits narrow the rest of the enclosing block.
                if then_jumps && !else_jumps {
                    self.apply(&neg);
                } else if else_jumps && !then_jumps {
                    self.apply(&pos);
                }
                // Join: a local assigned in a branch has the union of its types at the branch ends.
                let mut joined: Vec<(LocalId, TyId)> = Vec::new();
                for ends in &branch_ends {
                    for &(id, t) in ends {
                        match joined.iter_mut().find(|(j, _)| *j == id) {
                            Some(entry) => entry.1 = self.types.union(&[entry.1, t]),
                            None => joined.push((id, t)),
                        }
                    }
                }
                self.apply_effects(&fx, entry, &joined);
                // (`if (this.items === null) { this.items = [] }`: then `this.items` is an array
                // after it, narrowed at the end of every branch that falls through)
                if let Some((first, rest)) = path_ends.split_first() {
                    for (l, p, t, heap) in first {
                        let (mut u, mut heap) = (*t, *heap);
                        let everywhere = rest.iter().all(|ends| match ends.iter().find(|(l2, p2, ..)| l2 == l && p2 == p) {
                            Some((_, _, t2, h2)) => {
                                u = self.types.union(&[u, *t2]);
                                heap |= *h2;
                                true
                            }
                            None => false,
                        });
                        if everywhere {
                            self.apply(&[super::expr::Narrowing::Path(*l, p.clone(), u, heap)]);
                        }
                    }
                }
            }
            StmtKind::While(c, body) => {
                let fx = effects_of_stmt(ast, *body).with(effects_of_expr(ast, *c));
                let entry = self.fcx().locals.len();
                self.push_scope();
                self.reset_assigned(*body);
                self.cond(*c);
                let pos = self.narrowings(*c, true);
                self.apply(&pos);
                self.loop_body(*body);
                self.pop_scope();
                self.apply_effects(&fx, entry, &[]);
            }
            StmtKind::DoWhile(body, c) => {
                let fx = effects_of_stmt(ast, *body).with(effects_of_expr(ast, *c));
                let entry = self.fcx().locals.len();
                self.push_scope();
                self.reset_assigned(*body);
                self.loop_body(*body);
                self.cond(*c);
                self.pop_scope();
                self.apply_effects(&fx, entry, &[]);
            }
            StmtKind::For { init, cond, step, body } => {
                let mut fx = effects_of_stmt(ast, *body);
                for e in [cond, step].into_iter().flatten() {
                    fx.extend(effects_of_expr(ast, *e));
                }
                let entry = self.fcx().locals.len();
                self.push_scope();
                if let Some(i) = init {
                    self.stmt(*i);
                }
                self.reset_assigned(*body);
                if let Some(st) = step {
                    self.reset_assigned_expr(*st);
                }
                if let Some(c) = cond {
                    self.cond(*c);
                    let pos = self.narrowings(*c, true);
                    self.apply(&pos);
                }
                self.loop_body(*body);
                if let Some(st) = step {
                    self.expr(*st, None);
                }
                self.pop_scope();
                self.apply_effects(&fx, entry, &[]);
            }
            StmtKind::ForOf { mutable, name, name_span, iter, body } => {
                let t = self.expr(*iter, None);
                let t = self.array_alias(t);
                let ispan = self.ast().expr(*iter).span;
                let tt = *self.types.get(t);
                let elem = match tt {
                    Ty::Array(e) | Ty::Set(e) | Ty::Gen(e) => e,
                    Ty::Error => ERROR,
                    // an instance of a class with `[Symbol.iterator]()`: what that gives (rewritten
                    // as `x.__iterator()`)
                    _ if let Some(e) = self.iterator_of(t) => {
                        self.rewrites.iterators.push((self.cur, *iter));
                        e
                    }
                    // a string's characters (code points), as in JavaScript
                    Ty::Str | Ty::StrLit(_) => STR,
                    // a map's entries, `[key, value]`, as in JavaScript
                    Ty::Map(k, v) => self.tuple_ty(&[k, v]),
                    // a JavaScript iterable (an array, a Set, a generator...)
                    Ty::Js => JS,
                    _ => {
                        let msg = format!("`for...of` needs an array or set, found `{}`", self.show(t));
                        self.report(Diagnostic::new("T0402", ispan, msg));
                        ERROR
                    }
                };
                let fx = effects_of_stmt(ast, *body);
                let entry = self.fcx().locals.len();
                self.push_scope();
                let kind = if *mutable { LocalKind::Let } else { LocalKind::LoopVar };
                self.rec_binding(s, elem);
                let id = self.declare(*name, elem, kind, *name_span, None, false);
                if self.spec.on {
                    self.fcx().locals[id].sites = self.sources(ispan);
                }
                self.reset_assigned(*body);
                self.loop_body(*body);
                self.pop_scope();
                self.apply_effects(&fx, entry, &[]);
            }
            StmtKind::Switch(disc, cases) => {
                let mut fx = Effects::default();
                for c in cases {
                    fx.extend(effects_of_stmts(ast, &c.body));
                }
                let entry = self.fcx().locals.len();
                self.switch(s, *disc, cases, span);
                self.apply_effects(&fx, entry, &[]);
            }
            StmtKind::Return(v) => {
                let frame_idx = self.fcx.last().unwrap().frames.len() - 1;
                let (ret, fname) = {
                    let f = &self.fcx.last().unwrap().frames[frame_idx];
                    (f.ret, f.name)
                };
                let who = fname.map(|n| format!("`{}`", self.name(n))).unwrap_or_else(|| "this function".to_string());
                let ret_sites = self.fcx.last().unwrap().frames[frame_idx].ret_sites.clone();
                let is_async = self.fcx.last().unwrap().frames[frame_idx].is_async;
                match (ret, v) {
                    (Some(r), Some(x)) => self.flow(ret_sites, |c| {
                        let t = c.expr(*x, Some(r));
                        let xspan = c.ast().expr(*x).span;
                        // an async function returning a promise: its value, as `return await p`
                        // (see `rewrite`)
                        if is_async
                            && matches!(c.types.get(t), Ty::Promise(..))
                            && !c.flat_members(r).iter().any(|&m| matches!(c.types.get(m), Ty::Promise(..)))
                        {
                            let m = c.cur;
                            c.rewrites.awaits.push((m, *x));
                            return;
                        }
                        if r == VOID {
                            if t != VOID && t != UNDEFINED && t != ERROR {
                                c.report(
                                    Diagnostic::new("T0001", xspan, format!("{who} returns `void`, so `return` takes no value"))
                                        .note("found", format!("`{}`", c.show(t))),
                                );
                            }
                        } else {
                            c.expect_assignable(t, r, xspan, Some(format!("return type of {who}")));
                        }
                    }),
                    (Some(r), None) => {
                        if r != VOID && r != ERROR && !self.types.has_undefined(r) {
                            self.report(Diagnostic::new("T0001", span, format!("{who} must return a value of type `{}`", self.show(r))));
                        }
                    }
                    (None, Some(x)) => {
                        let t = self.expr(*x, None);
                        self.fcx().frames[frame_idx].returns.push(t);
                    }
                    (None, None) => {
                        self.fcx().frames[frame_idx].returns.push(UNDEFINED);
                    }
                }
            }
            StmtKind::Throw(x) => {
                let t = self.expr(*x, None);
                let err = self.error_class();
                if t != ERROR && err != ERROR && !self.assignable(t, err) {
                    let shown = self.show(t);
                    let xs = self.ast().expr(*x).span;
                    let text = self.src(xs).to_string();
                    let repl = if self.types.is_string(t) { format!("new Error({text})") } else { format!("new Error(String({text}))") };
                    self.report(
                        Diagnostic::new("T0834", xs, format!("can only throw `Error` objects, found `{shown}`"))
                            .fix(Applicability::Maybe, format!("throw an Error: `{repl}`"), xs, repl.clone()),
                    );
                } else if t != ERROR {
                    let t = self.types.without_undefined(t);
                    self.on_throw(t, span, None, None);
                }
            }
            StmtKind::Try { body, catch, finally } => {
                let mut fx = effects_of_stmt(ast, *body);
                if let Some(c) = catch {
                    fx.extend(effects_of_stmt(ast, c.body));
                }
                if let Some(f) = finally {
                    fx.extend(effects_of_stmt(ast, *f));
                }
                let entry = self.fcx().locals.len();
                if catch.is_some() {
                    self.cur_frame().try_frames.push(Vec::new());
                }
                self.stmt(*body);
                if let Some(c) = catch {
                    let caught = self.cur_frame().try_frames.pop().unwrap_or_default();
                    let ety = self.catch_type(&caught);
                    self.push_scope();
                    if let Some((name, nspan, te)) = &c.param {
                        let ety = match te {
                            Some(te) => {
                                let tscope = self.tscope();
                                let declared = self.resolve_type(*te, &tscope);
                                if declared != UNKNOWN && !self.assignable(ety, declared) {
                                    let (a, b) = (self.show(ety), self.show(declared));
                                    self.report(Diagnostic::new("T0001", *nspan, format!("this `catch` receives `{a}`, which isn't `{b}`")));
                                }
                                declared
                            }
                            None => ety,
                        };
                        self.rec_binding(s, ety);
                        self.declare(*name, ety, LocalKind::Const, *nspan, None, false);
                    }
                    self.stmt(c.body);
                    self.pop_scope();
                }
                if let Some(f) = finally {
                    self.stmt(*f);
                }
                // Any part of the `try` may have run: forget what it changed.
                self.apply_effects(&fx, entry, &[]);
            }
            StmtKind::Break(label) | StmtKind::Continue(label) => {
                let brk = matches!(node.kind, StmtKind::Break(_));
                let f = self.fcx.last().unwrap().frames.last().unwrap();
                match label {
                    Some(l) if !f.labels.contains(l) => {
                        let n = self.name(*l).to_string();
                        self.report(Diagnostic::new("F0203", span, format!("no enclosing loop is labelled `{n}`")));
                    }
                    Some(_) => {}
                    None if brk && f.loops == 0 && f.switches == 0 => {
                        self.report(Diagnostic::new("F0201", span, "`break` outside of a loop or `switch`"));
                    }
                    None if !brk && f.loops == 0 => {
                        self.report(Diagnostic::new("F0202", span, "`continue` outside of a loop"));
                    }
                    None => {}
                }
            }
        }
    }

    /// Current types of the outer locals (declared before `entry`) named in `assigned`.
    pub(super) fn end_types(&self, assigned: &[Sym], entry: usize) -> Vec<(LocalId, TyId)> {
        let mut out = Vec::new();
        for &n in assigned {
            if let Some((id, t)) = self.lookup(n)
                && id < entry
                && !out.iter().any(|(x, _)| *x == id)
            {
                out.push((id, t));
            }
        }
        out
    }

    /// The field paths of outer locals narrowed since `ptab` (in this branch), as narrowed here.
    fn end_paths(&self, ptab: usize, entry: usize) -> Vec<(LocalId, Vec<Sym>, TyId, bool)> {
        let fcx = self.fcx.last().unwrap();
        let mut out: Vec<(LocalId, Vec<Sym>, TyId, bool)> = Vec::new();
        for (l, p, t) in &fcx.path_table[ptab.min(fcx.path_table.len())..] {
            if t.is_none() || p.is_empty() || *l >= entry || out.iter().any(|(l2, p2, ..)| l2 == l && p2 == p) {
                continue;
            }
            if let Some(t) = self.lookup_path(*l, p) {
                let heap = fcx.heap_paths.iter().any(|(l2, p2)| l2 == l && p2 == p);
                out.push((*l, p.clone(), t, heap));
            }
        }
        out
    }

    /// After a construct with its own scope: what it did to outer locals reaches the enclosing
    /// scope. Assigned locals get their type from `ends` (or their declared type), changed values
    /// forget their field-path narrowings, and a call forgets narrowings through class instances.
    pub(super) fn apply_effects(&mut self, fx: &Effects, entry: usize, ends: &[(LocalId, TyId)]) {
        for &n in &fx.assigned {
            if let Some((id, _)) = self.lookup(n)
                && id < entry
            {
                let t = ends.iter().find(|(x, _)| *x == id).map(|(_, t)| *t).unwrap_or(self.local(id).ty);
                self.narrow(id, t);
                self.narrow_path(id, Vec::new(), None);
            }
        }
        for &n in &fx.roots {
            if let Some((id, _)) = self.lookup(n)
                && id < entry
            {
                self.narrow_path(id, Vec::new(), None);
            }
        }
        // (`this.tail = x` in a branch: what's narrowed through a field `tail`, not `this.head`)
        for &(n, f) in &fx.fields {
            if let Some((id, _)) = self.lookup(n)
                && id < entry
            {
                let fcx = self.fcx.last().unwrap();
                let mut paths: Vec<Vec<Sym>> = fcx.path_table.iter().filter(|(l, p, t)| *l == id && t.is_some() && p.contains(&f)).map(|(_, p, _)| p.clone()).collect();
                paths.dedup();
                for p in paths {
                    self.narrow_path(id, p, None);
                }
            }
        }
        if fx.call {
            self.invalidate_heap();
        }
    }

    /// Forgets what a call may have changed: every narrowing of a path through a class
    /// instance, and every narrowing of a mutable global (any callee can reassign one).
    pub(super) fn invalidate_heap(&mut self) {
        let paths = std::mem::take(&mut self.fcx().heap_paths);
        // (TypeScript keeps them, `if (this.head) { log(); this.head.value }`: so does a
        // TypeScript module, each later read checked at run time, trapping if the call did
        // change it)
        let ts = self.ts_module();
        for (l, p) in paths {
            let keep = if ts { self.lookup_path(l, &p) } else { None };
            self.narrow_path(l, p.clone(), None);
            if let Some(t) = keep {
                self.narrow_path(l, p.clone(), Some(t));
                let fcx = self.fcx();
                let idx = fcx.path_table.len() - 1;
                fcx.asserted.insert(idx);
                fcx.heap_paths.push((l, p));
            }
        }
        let globals: Vec<LocalId> = self.fcx().locals.iter().enumerate().filter(|(_, l)| l.global.is_some() && l.kind == LocalKind::Let).map(|(i, _)| i).collect();
        for id in globals {
            let declared = self.local(id).ty;
            self.narrow(id, declared);
            self.narrow_path(id, Vec::new(), None);
        }
    }

    /// The type of a `catch` variable: the most specific class covering everything the `try`
    /// block can throw (`Error` if nothing, or unrelated classes).
    fn catch_type(&mut self, caught: &[TyId]) -> TyId {
        let err = self.error_class();
        let mut classes: Vec<TyId> = Vec::new();
        for &t in caught {
            for m in self.flat_members(t) {
                if !classes.contains(&m) {
                    classes.push(m);
                }
            }
        }
        let Some(&first) = classes.first() else { return err };
        let mut cur = Some(first);
        while let Some(c) = cur {
            if classes.iter().all(|&o| self.assignable(o, c)) {
                return c;
            }
            cur = self.class_of(c).and_then(|(d, args)| {
                let base = self.classes[d as usize].base?;
                let map: HashMap<u32, TyId> = self.classes[d as usize].params.iter().copied().zip(args).collect();
                Some(self.types.subst(base, &map))
            });
        }
        err
    }

    fn loop_body(&mut self, body: StmtId) {
        self.fcx().frames.last_mut().unwrap().loops += 1;
        self.push_scope();
        self.stmt(body);
        self.pop_scope();
        self.fcx().frames.last_mut().unwrap().loops -= 1;
    }

    /// Forgets narrowings of locals assigned inside a loop: the loop may run again after the assignment.
    fn reset_assigned(&mut self, body: StmtId) {
        let mut names = Vec::new();
        collect_assigned_stmt(self.ast(), body, &mut names);
        self.reset_names(&names);
    }

    fn reset_assigned_expr(&mut self, e: ExprId) {
        let mut names = Vec::new();
        collect_assigned_expr(self.ast(), e, &mut names);
        self.reset_names(&names);
    }

    fn reset_names(&mut self, names: &[Sym]) {
        for &n in names {
            if let Some((id, _)) = self.lookup(n) {
                let declared = self.local(id).ty;
                self.narrow(id, declared);
                self.narrow_path(id, Vec::new(), None);
            }
        }
    }

    fn switch_mode(&mut self, disc: ExprId, dt: TyId) -> SwitchMode {
        if let ExprKind::Member { obj, name, optional: false, .. } = &self.ast().expr(disc).kind
            && let ExprKind::Ident(s) = &self.ast().expr(*obj).kind
                && let Some((id, ty)) = self.lookup(*s) {
                    let members = self.flat_members(ty);
                    if members.len() > 1 {
                        let mut variants = Vec::new();
                        for &m in &members {
                            let lit = match *self.types.get(m) {
                                Ty::Record(fs) => self.types.fields(fs).iter().find(|f| f.name == *name).and_then(|f| match *self.types.get(f.ty) {
                                    Ty::StrLit(l) => Some(l),
                                    _ => None,
                                }),
                                _ => None,
                            };
                            match lit {
                                Some(l) => variants.push((l, m)),
                                None => {
                                    variants.clear();
                                    break;
                                }
                            }
                        }
                        if !variants.is_empty() {
                            return SwitchMode::Discriminated { local: id, variants };
                        }
                    }
                }
        let members = self.flat_members(dt);
        if !members.is_empty() && members.iter().all(|&m| matches!(self.types.get(m), Ty::StrLit(_))) {
            let lits = members.iter().map(|&m| if let Ty::StrLit(l) = self.types.get(m) { *l } else { unreachable!() }).collect();
            let local = match &self.ast().expr(disc).kind {
                ExprKind::Ident(s) => self.lookup(*s).map(|(id, _)| id),
                _ => None,
            };
            return SwitchMode::Literal { local, lits };
        }
        SwitchMode::General
    }

    /// `E.name` of a module constant that's an object literal (an enum), whose field is a
    /// string literal: the string.
    pub(crate) fn enum_str_member(&self, e: ExprId) -> Option<Sym> {
        let ExprKind::Member { obj, name, optional: false, .. } = self.ast().expr(e).kind else { return None };
        let cur = self.cur as usize;
        let fact = self.facts.as_ref()?.get(cur)?.idents.get(&obj).copied()?;
        let (mast, init) = enum_object(self.modules, cur as u32, fact)?;
        let ExprKind::Object(fs) = &mast.expr(init).kind else { return None };
        let v = fs.iter().find(|f| f.name == name)?.value;
        match mast.expr(v).kind {
            ExprKind::Str(s) => Some(s),
            _ => None,
        }
    }

    fn switch(&mut self, sid: StmtId, disc: ExprId, cases: &[Case], span: Span) {
        let dt = self.expr(disc, None);
        let mode = self.switch_mode(disc, dt);
        let options: Vec<Sym> = match &mode {
            SwitchMode::Discriminated { variants, .. } => variants.iter().map(|v| v.0).collect(),
            SwitchMode::Literal { lits, .. } => lits.clone(),
            SwitchMode::General => Vec::new(),
        };
        let literal_mode = !matches!(mode, SwitchMode::General);
        // Labels first, so `default` can be narrowed to what's left.
        let mut covered: Vec<Sym> = Vec::new();
        let mut labels: Vec<Option<Sym>> = Vec::new();
        let mut has_default = false;
        for case in cases {
            let Some(test) = case.test else {
                has_default = true;
                labels.push(None);
                continue;
            };
            let tspan = self.ast().expr(test).span;
            if literal_mode {
                let lit = match self.ast().expr(test).kind {
                    ExprKind::Str(lit) => Some(lit),
                    // (a string enum's member: its value)
                    _ => {
                        self.expr(test, None);
                        self.enum_str_member(test)
                    }
                };
                let Some(lit) = lit else {
                    self.report(Diagnostic::new("T0104", tspan, "case labels here must be string literals"));
                    labels.push(None);
                    continue;
                };
                let v = self.name(lit).to_string();
                let v = &v;
                if !options.contains(&lit) {
                    let opts: Vec<String> = options.iter().map(|&o| self.name(o).to_string()).collect();
                    let quoted: Vec<String> = opts.iter().map(|o| format!("\"{o}\"")).collect();
                    let shown = self.show(if let SwitchMode::Discriminated { local, .. } = &mode { self.local(*local).ty } else { dt });
                    let mut d = Diagnostic::new("T0103", tspan, format!("\"{v}\" is not a possible value of `{shown}`")).note("valid values", quoted.join(", "));
                    let sug = similar(v, opts.iter().map(|s| s.as_str()));
                    if let Some(first) = sug.first() {
                        d = d.fix(Applicability::Maybe, format!("use \"{first}\""), tspan, format!("\"{first}\""));
                    }
                    self.report(d);
                    labels.push(None);
                    continue;
                }
                if covered.contains(&lit) {
                    self.report(Diagnostic::new("F0303", tspan, format!("duplicate case \"{v}\"")));
                }
                covered.push(lit);
                labels.push(Some(lit));
            } else {
                let t = self.expr(test, Some(dt));
                if !self.comparable(t, dt) {
                    let msg = format!("case of type `{}` can never match a `{}`", self.show(t), self.show(dt));
                    self.report(Diagnostic::new("T0502", tspan, msg));
                }
                labels.push(None);
            }
        }
        // Bodies, grouped: empty cases fall through to the next one.
        self.fcx().frames.last_mut().unwrap().switches += 1;
        let mut group: Vec<Option<Sym>> = Vec::new();
        let mut group_default = false;
        for (i, case) in cases.iter().enumerate() {
            if case.test.is_none() {
                group_default = true;
            }
            group.push(labels[i]);
            if case.body.is_empty() && i + 1 < cases.len() {
                continue;
            }
            self.push_scope();
            let narrowed_lits: Vec<Sym> = if group_default {
                options.iter().copied().filter(|o| !covered.contains(o) || group.contains(&Some(*o))).collect()
            } else {
                group.iter().flatten().copied().collect()
            };
            match &mode {
                // (a `default` after a case for every variant: `never`, as in TypeScript's
                // `const _exhaustive: never = s`)
                SwitchMode::Discriminated { local, variants } => {
                    let tys: Vec<TyId> = variants.iter().filter(|(l, _)| narrowed_lits.contains(l)).map(|(_, t)| *t).collect();
                    if !tys.is_empty() || group_default {
                        let t = self.types.union(&tys);
                        self.narrow(*local, t);
                    }
                }
                SwitchMode::Literal { local: Some(local), .. } => {
                    let tys: Vec<TyId> = narrowed_lits.iter().map(|&l| self.types.str_lit(l)).collect();
                    if !tys.is_empty() || group_default {
                        let t = self.types.union(&tys);
                        self.narrow(*local, t);
                    }
                }
                _ => {}
            }
            for &st in &case.body {
                self.stmt(st);
            }
            self.pop_scope();
            if i + 1 < cases.len() && !case.body.is_empty() && !self.always_jumps_in_switch(&case.body) {
                let last = *case.body.last().unwrap();
                let lspan = self.ast().stmt(last).span;
                let (line, _) = self.sm.get(lspan.file).line_col(lspan.start);
                let indent: String = self.sm.get(lspan.file).line_text(line).chars().take_while(|c| c.is_whitespace()).collect();
                self.report(
                    Diagnostic::new("F0302", case.span, "this case falls through into the next one")
                        .note("why", "implicit fallthrough is almost always a bug")
                        .fix(Applicability::Safe, "end the case with `break`", lspan.empty_at_end(), format!("\n{indent}break")),
                );
            }
            group.clear();
            group_default = false;
        }
        self.fcx().frames.last_mut().unwrap().switches -= 1;
        if literal_mode && !has_default {
            let missing: Vec<Sym> = options.iter().copied().filter(|o| !covered.contains(o)).collect();
            if missing.is_empty() {
                self.exhaustive.insert((self.cur, sid));
            } else {
                let names: Vec<String> = missing.iter().map(|&m| format!("\"{}\"", self.name(m))).collect();
                let close = Span::new(span.file, span.end.saturating_sub(1), span.end.saturating_sub(1));
                let insert: String = missing.iter().map(|&m| format!("  case \"{}\": <handle>\n", self.name(m))).collect();
                let dspan = self.ast().expr(disc).span;
                self.fcx().reported_nonexhaustive = true;
                self.report(
                    Diagnostic::new("F0301", dspan, format!("`switch` is not exhaustive: missing {}", names.join(", ")))
                        .fix(Applicability::Placeholder, format!("add case{} {}", if missing.len() == 1 { "" } else { "s" }, names.join(", ")), close, insert),
                );
            }
        } else if has_default || self.covers_enum(cases) {
            self.exhaustive.insert((self.cur, sid));
        }
    }

    /// Every case is a member of one numeric enum (`case Direction.Up:`), and every member has
    /// a case: the switch handles each value the enum's type was meant to hold.
    fn covers_enum(&self, cases: &[Case]) -> bool {
        let ast = self.ast();
        let mut enum_name = None;
        let mut covered = Vec::new();
        for c in cases {
            let Some(test) = c.test else { return false };
            let ExprKind::Member { obj, name, .. } = ast.expr(test).kind else { return false };
            let ExprKind::Ident(e) = ast.expr(obj).kind else { return false };
            if enum_name.is_some_and(|n| n != e) {
                return false;
            }
            enum_name = Some(e);
            covered.push(name);
        }
        let Some(e) = enum_name else { return false };
        self.enum_members(e).is_some_and(|(_, ms)| ms.iter().all(|(n, _)| covered.contains(n)))
    }

    /// `e` names an enum (not shadowed): its module and its members with their values.
    pub(super) fn enum_members(&self, e: Sym) -> Option<(u32, Vec<(Sym, ExprId)>)> {
        // (a script's top-level `const` is a local standing in for the item)
        let (m, i) = match self.lookup(e) {
            Some((id, _)) => self.local(id).global?,
            None => match self.scopes[self.cur as usize].values.get(&e) {
                Some(&(Decl::Const(m, i), _)) => (m, i),
                _ => return None,
            },
        };
        let mast = &self.modules[m as usize].ast;
        let ItemKind::Const { ty: Some(t), init, .. } = &mast.items[i as usize].kind else { return None };
        let (TypeExprKind::Record(fields), ExprKind::Object(values)) = (&mast.ty(*t).kind, &mast.expr(*init).kind) else { return None };
        // (the record `{ Up: Direction, ... }` the parser gives an enum's members)
        let is_enum = !fields.is_empty() && fields.iter().all(|f| matches!(&mast.ty(f.ty).kind, TypeExprKind::Named { name, args, .. } if *name == e && args.is_empty()));
        is_enum.then(|| (m, values.iter().map(|f| (f.name, f.value)).collect()))
    }

    /// `Direction[d]` on an enum whose values are 0, 1, ...: the members' names, in order.
    pub(super) fn enum_names(&self, e: Sym) -> Option<Vec<Sym>> {
        let (m, ms) = self.enum_members(e)?;
        let ast = &self.modules[m as usize].ast;
        ms.iter().enumerate().all(|(k, &(_, v))| matches!(ast.expr(v).kind, ExprKind::Int(x) if x == k as u64)).then(|| ms.iter().map(|m| m.0).collect())
    }

    // ---------------------------------------------------------------- control flow

    /// Control never reaches the statement after `s`.
    pub(super) fn always_jumps(&self, s: StmtId) -> bool {
        match &self.ast().stmt(s).kind {
            StmtKind::Return(_) | StmtKind::Break(_) | StmtKind::Continue(_) | StmtKind::Throw(_) => true,
            StmtKind::Block(ss) => ss.iter().any(|&x| self.always_jumps(x)),
            StmtKind::If(_, t, Some(e)) => self.always_jumps(*t) && self.always_jumps(*e),
            _ => self.always_returns(s),
        }
    }

    fn always_jumps_in_switch(&self, body: &[StmtId]) -> bool {
        body.iter().any(|&s| self.always_jumps(s))
    }

    /// Every path through `s` ends in `return`.
    pub(super) fn always_returns(&self, s: StmtId) -> bool {
        let ast = self.ast();
        match &ast.stmt(s).kind {
            StmtKind::Return(_) => true,
            StmtKind::Block(ss) => {
                for &x in ss {
                    if self.always_returns(x) {
                        return true;
                    }
                    if self.always_jumps(x) {
                        return false;
                    }
                }
                false
            }
            StmtKind::If(_, t, Some(e)) => self.always_returns(*t) && self.always_returns(*e),
            StmtKind::Switch(_, cases) => {
                if !self.exhaustive.contains(&(self.cur, s)) {
                    return false;
                }
                // Each non-empty group must return (a `break` leaves the switch).
                cases.iter().enumerate().all(|(i, c)| {
                    if c.body.is_empty() && i + 1 < cases.len() {
                        return true;
                    }
                    for &x in &c.body {
                        if self.always_returns(x) {
                            return true;
                        }
                        if self.always_jumps(x) {
                            return false;
                        }
                    }
                    false
                })
            }
            StmtKind::While(c, body) => self.is_true(*c) && !has_break(ast, *body),
            StmtKind::For { cond: None, body, .. } => !has_break(ast, *body),
            StmtKind::For { cond: Some(c), body, .. } => self.is_true(*c) && !has_break(ast, *body),
            StmtKind::DoWhile(body, _) => self.always_returns(*body),
            // `throw` never falls through (for "missing return" it ends the function).
            StmtKind::Throw(_) => true,
            StmtKind::Try { body, catch, finally } => {
                finally.map(|f| self.always_returns(f)).unwrap_or(false)
                    || (self.always_returns(*body) && catch.as_ref().map(|c| self.always_returns(c.body)).unwrap_or(true))
            }
            _ => false,
        }
    }

    fn is_true(&self, e: ExprId) -> bool {
        matches!(self.ast().expr(e).kind, ExprKind::Bool(true))
    }
}

/// A `break` that exits this loop (not a nested loop or switch), or a labelled `break` /
/// `continue` anywhere in it (which leaves it too, unless it names a loop inside: counted anyway).
fn has_break(ast: &Ast, s: StmtId) -> bool {
    match &ast.stmt(s).kind {
        StmtKind::Break(_) => true,
        StmtKind::Block(ss) => ss.iter().any(|&x| has_break(ast, x)),
        StmtKind::If(_, t, e) => has_break(ast, *t) || e.map(|e| has_break(ast, e)).unwrap_or(false),
        _ => has_labelled_jump(ast, s),
    }
}

/// A `break outer` / `continue outer` anywhere in `s`.
pub(crate) fn has_labelled_jump(ast: &Ast, s: StmtId) -> bool {
    if ast.labels.is_empty() {
        return false;
    }
    let sp = ast.stmt(s).span;
    ast.stmts.iter().any(|x| matches!(x.kind, StmtKind::Break(Some(_)) | StmtKind::Continue(Some(_))) && x.span.file == sp.file && x.span.start >= sp.start && x.span.end <= sp.end)
}

pub(super) fn collect_assigned_stmt(ast: &Ast, s: StmtId, out: &mut Vec<Sym>) {
    match &ast.stmt(s).kind {
        StmtKind::Expr(e) => collect_assigned_expr(ast, *e, out),
        StmtKind::Let { init: Some(e), .. } => collect_assigned_expr(ast, *e, out),
        StmtKind::If(c, t, e) => {
            collect_assigned_expr(ast, *c, out);
            collect_assigned_stmt(ast, *t, out);
            if let Some(e) = e {
                collect_assigned_stmt(ast, *e, out);
            }
        }
        StmtKind::While(c, b) | StmtKind::DoWhile(b, c) => {
            collect_assigned_expr(ast, *c, out);
            collect_assigned_stmt(ast, *b, out);
        }
        StmtKind::For { init, cond, step, body } => {
            if let Some(i) = init {
                collect_assigned_stmt(ast, *i, out);
            }
            for e in [cond, step].into_iter().flatten() {
                collect_assigned_expr(ast, *e, out);
            }
            collect_assigned_stmt(ast, *body, out);
        }
        StmtKind::ForOf { iter, body, .. } => {
            collect_assigned_expr(ast, *iter, out);
            collect_assigned_stmt(ast, *body, out);
        }
        StmtKind::Switch(d, cases) => {
            collect_assigned_expr(ast, *d, out);
            for c in cases {
                for &x in &c.body {
                    collect_assigned_stmt(ast, x, out);
                }
            }
        }
        StmtKind::Return(Some(e)) | StmtKind::Throw(e) => collect_assigned_expr(ast, *e, out),
        StmtKind::Block(ss) => ss.iter().for_each(|&x| collect_assigned_stmt(ast, x, out)),
        StmtKind::Try { body, catch, finally } => {
            collect_assigned_stmt(ast, *body, out);
            if let Some(c) = catch {
                collect_assigned_stmt(ast, c.body, out);
            }
            if let Some(f) = finally {
                collect_assigned_stmt(ast, *f, out);
            }
        }
        _ => {}
    }
}

fn collect_assigned_expr(ast: &Ast, e: ExprId, out: &mut Vec<Sym>) {
    match &ast.expr(e).kind {
        ExprKind::Assign(op, t, v) => {
            if let ExprKind::Ident(s) = ast.expr(*t).kind {
                out.push(s);
            }
            let _: &AssignOp = op;
            collect_assigned_expr(ast, *t, out);
            collect_assigned_expr(ast, *v, out);
        }
        ExprKind::Update { target, .. } => {
            if let ExprKind::Ident(s) = ast.expr(*target).kind {
                out.push(s);
            }
        }
        ExprKind::Unary(_, x) | ExprKind::Paren(x) | ExprKind::NonNull(x) | ExprKind::Typeof(x) | ExprKind::Spread(x) | ExprKind::As(x, _) | ExprKind::Try(x) | ExprKind::Await(x) | ExprKind::Yield(Some(x), _) => collect_assigned_expr(ast, *x, out),
        ExprKind::Binary(_, l, r) => {
            collect_assigned_expr(ast, *l, out);
            collect_assigned_expr(ast, *r, out);
        }
        ExprKind::Call { callee, args, .. } => {
            collect_assigned_expr(ast, *callee, out);
            for a in args {
                collect_assigned_expr(ast, a.expr, out);
            }
        }
        ExprKind::Member { obj, .. } => collect_assigned_expr(ast, *obj, out),
        ExprKind::Index { obj, index, .. } => {
            collect_assigned_expr(ast, *obj, out);
            collect_assigned_expr(ast, *index, out);
        }
        ExprKind::Cond(a, b, c) => {
            for x in [a, b, c] {
                collect_assigned_expr(ast, *x, out);
            }
        }
        ExprKind::Array(xs) | ExprKind::Seq(xs) => xs.iter().for_each(|&x| collect_assigned_expr(ast, x, out)),
        ExprKind::Object(fs) => fs.iter().for_each(|f| collect_assigned_expr(ast, f.value, out)),
        ExprKind::Template(_, xs) => xs.iter().for_each(|&x| collect_assigned_expr(ast, x, out)),
        ExprKind::Arrow(f) => match &f.body {
            ArrowBody::Expr(x) => collect_assigned_expr(ast, *x, out),
            ArrowBody::Block(b) => collect_assigned_stmt(ast, *b, out),
        },
        _ => {}
    }
}

/// What a construct may do to the locals around it.
#[derive(Default)]
pub(super) struct Effects {
    /// Locals assigned (`x = ...`, `x++`).
    pub assigned: Vec<Sym>,
    /// Locals changed in place (`x[i] = ...`, `&x`, method calls on `x...`).
    pub roots: Vec<Sym>,
    /// Fields assigned through a local (`x.f = ...`, `x.a.f++`): (the local, the field). Only
    /// paths through a field of that name can change.
    pub fields: Vec<(Sym, Sym)>,
    /// Contains a call (which may change class instances).
    pub call: bool,
}

impl Effects {
    pub(super) fn extend(&mut self, o: Effects) {
        self.assigned.extend(o.assigned);
        self.roots.extend(o.roots);
        self.fields.extend(o.fields);
        self.call |= o.call;
    }

    pub(super) fn with(mut self, o: Effects) -> Effects {
        self.extend(o);
        self
    }
}

pub(super) fn effects_of_stmts(ast: &Ast, ss: &[StmtId]) -> Effects {
    let mut fx = Effects::default();
    for &s in ss {
        fx.extend(effects_of_stmt(ast, s));
    }
    fx
}

pub(super) fn effects_of_stmt(ast: &Ast, s: StmtId) -> Effects {
    let mut fx = Effects::default();
    let mut exprs = Vec::new();
    stmt_exprs(ast, s, &mut exprs);
    for e in exprs {
        effects_into(ast, e, &mut fx);
    }
    fx
}

pub(super) fn effects_of_expr(ast: &Ast, e: ExprId) -> Effects {
    let mut fx = Effects::default();
    effects_into(ast, e, &mut fx);
    fx
}

fn root_sym(ast: &Ast, mut e: ExprId) -> Option<Sym> {
    loop {
        match &ast.expr(e).kind {
            ExprKind::Paren(x) | ExprKind::NonNull(x) | ExprKind::Member { obj: x, .. } | ExprKind::Index { obj: x, .. } => e = *x,
            ExprKind::Ident(s) => return Some(*s),
            ExprKind::This => return Some(super::THIS_SYM),
            _ => return None,
        }
    }
}

/// Effects of evaluating `e` (arrow bodies don't run when the arrow is created).
fn effects_into(ast: &Ast, e: ExprId, fx: &mut Effects) {
    match &ast.expr(e).kind {
        ExprKind::Assign(_, t, v) => {
            match &ast.expr(*t).kind {
                ExprKind::Ident(s) => fx.assigned.push(*s),
                ExprKind::Member { name, .. } if let Some(r) = root_sym(ast, *t) => fx.fields.push((r, *name)),
                _ => fx.roots.extend(root_sym(ast, *t)),
            }
            effects_into(ast, *t, fx);
            effects_into(ast, *v, fx);
        }
        ExprKind::Update { target, .. } => {
            match &ast.expr(*target).kind {
                ExprKind::Ident(s) => fx.assigned.push(*s),
                ExprKind::Member { name, .. } if let Some(r) = root_sym(ast, *target) => fx.fields.push((r, *name)),
                _ => fx.roots.extend(root_sym(ast, *target)),
            }
            effects_into(ast, *target, fx);
        }
        ExprKind::Call { callee, args, .. } => {
            fx.call = true;
            if let ExprKind::Member { obj, .. } = &ast.expr(*callee).kind {
                fx.roots.extend(root_sym(ast, *obj));
            }
            effects_into(ast, *callee, fx);
            for a in args {
                if a.by_ref.is_some() {
                    match &ast.expr(a.expr).kind {
                        ExprKind::Ident(s) => fx.assigned.push(*s),
                        _ => fx.roots.extend(root_sym(ast, a.expr)),
                    }
                }
                effects_into(ast, a.expr, fx);
            }
        }
        ExprKind::New { args, .. } => {
            fx.call = true;
            for a in args {
                effects_into(ast, a.expr, fx);
            }
        }
        // A getter is a call; any member access through a class may run one.
        ExprKind::Member { obj, .. } => {
            fx.call = true;
            effects_into(ast, *obj, fx);
        }
        ExprKind::Unary(_, x) | ExprKind::Paren(x) | ExprKind::NonNull(x) | ExprKind::Typeof(x) | ExprKind::Spread(x) | ExprKind::As(x, _) | ExprKind::Try(x) | ExprKind::Await(x) | ExprKind::Yield(Some(x), _) => effects_into(ast, *x, fx),
        ExprKind::Binary(_, l, r) => {
            effects_into(ast, *l, fx);
            effects_into(ast, *r, fx);
        }
        ExprKind::Index { obj, index, .. } => {
            effects_into(ast, *obj, fx);
            effects_into(ast, *index, fx);
        }
        ExprKind::Cond(a, b, c) => {
            for x in [a, b, c] {
                effects_into(ast, *x, fx);
            }
        }
        ExprKind::Array(xs) | ExprKind::Template(_, xs) | ExprKind::Seq(xs) => xs.iter().for_each(|&x| effects_into(ast, x, fx)),
        ExprKind::Object(fs) => fs.iter().for_each(|f| effects_into(ast, f.value, fx)),
        _ => {}
    }
}

/// The expressions a statement evaluates directly or in nested statements (not inside arrows).
/// The names a block declares (`let`, `const`, `for (const x of ...)`, `catch (e)`), at any depth
/// of its statements (not inside nested closures, whose names are their own).
fn declared_in(ast: &Ast, s: StmtId, out: &mut HashSet<Sym>) {
    match &ast.stmt(s).kind {
        StmtKind::Let { name, .. } => {
            out.insert(*name);
        }
        StmtKind::ForOf { name, body, .. } => {
            out.insert(*name);
            declared_in(ast, *body, out);
        }
        StmtKind::Block(b) => b.iter().for_each(|&x| declared_in(ast, x, out)),
        StmtKind::If(_, t, e) => {
            declared_in(ast, *t, out);
            if let Some(e) = e {
                declared_in(ast, *e, out);
            }
        }
        StmtKind::While(_, b) | StmtKind::DoWhile(b, _) => declared_in(ast, *b, out),
        StmtKind::For { init, body, .. } => {
            if let Some(i) = init {
                declared_in(ast, *i, out);
            }
            declared_in(ast, *body, out);
        }
        StmtKind::Switch(_, cases) => cases.iter().for_each(|c| c.body.iter().for_each(|&x| declared_in(ast, x, out))),
        StmtKind::Try { body, catch, finally } => {
            declared_in(ast, *body, out);
            if let Some(c) = catch {
                if let Some((n, _, _)) = c.param {
                    out.insert(n);
                }
                declared_in(ast, c.body, out);
            }
            if let Some(f) = finally {
                declared_in(ast, *f, out);
            }
        }
        _ => {}
    }
}

fn stmt_exprs(ast: &Ast, s: StmtId, out: &mut Vec<ExprId>) {
    match &ast.stmt(s).kind {
        StmtKind::Expr(e) | StmtKind::Return(Some(e)) | StmtKind::Throw(e) => out.push(*e),
        StmtKind::Let { init: Some(e), .. } => out.push(*e),
        StmtKind::Try { body, catch, finally } => {
            stmt_exprs(ast, *body, out);
            if let Some(c) = catch {
                stmt_exprs(ast, c.body, out);
            }
            if let Some(f) = finally {
                stmt_exprs(ast, *f, out);
            }
        }
        StmtKind::If(c, t, e) => {
            out.push(*c);
            stmt_exprs(ast, *t, out);
            if let Some(e) = e {
                stmt_exprs(ast, *e, out);
            }
        }
        StmtKind::While(c, b) | StmtKind::DoWhile(b, c) => {
            out.push(*c);
            stmt_exprs(ast, *b, out);
        }
        StmtKind::For { init, cond, step, body } => {
            if let Some(i) = init {
                stmt_exprs(ast, *i, out);
            }
            out.extend([cond, step].into_iter().flatten().copied());
            stmt_exprs(ast, *body, out);
        }
        StmtKind::ForOf { iter, body, .. } => {
            out.push(*iter);
            stmt_exprs(ast, *body, out);
        }
        StmtKind::Switch(d, cases) => {
            out.push(*d);
            for c in cases {
                out.extend(c.test);
                for &x in &c.body {
                    stmt_exprs(ast, x, out);
                }
            }
        }
        StmtKind::Block(ss) => ss.iter().for_each(|&x| stmt_exprs(ast, x, out)),
        _ => {}
    }
}

/// Locals changed inside closures anywhere in a function body: (assigned or changed through —
/// a field, an element, a method call on them —, only assigned).
pub(super) fn closure_mutated_stmt(ast: &Ast, s: StmtId) -> (HashSet<Sym>, HashSet<Sym>) {
    let mut exprs = Vec::new();
    stmt_exprs(ast, s, &mut exprs);
    let mut out = (HashSet::default(), HashSet::default());
    for e in exprs {
        closure_mutated_into(ast, e, false, &mut out);
    }
    out
}

pub(super) fn closure_mutated_expr(ast: &Ast, e: ExprId) -> (HashSet<Sym>, HashSet<Sym>) {
    let mut out = (HashSet::default(), HashSet::default());
    closure_mutated_into(ast, e, false, &mut out);
    out
}

fn closure_mutated_into(ast: &Ast, e: ExprId, inside: bool, out: &mut (HashSet<Sym>, HashSet<Sym>)) {
    let mut kids: Vec<ExprId> = Vec::new();
    match &ast.expr(e).kind {
        ExprKind::Arrow(f) => {
            let mut exprs = Vec::new();
            match &f.body {
                ArrowBody::Expr(x) => exprs.push(*x),
                ArrowBody::Block(b) => stmt_exprs(ast, *b, &mut exprs),
            }
            let mut inner = (HashSet::default(), HashSet::default());
            let mut nested = (HashSet::default(), HashSet::default());
            for &x in &exprs {
                closure_mutated_into(ast, x, true, &mut inner);
                closure_mutated_into(ast, x, false, &mut nested);
            }
            // The closure's own parameters and variables aren't the enclosing function's: what
            // the closure itself does to them doesn't count, what a closure inside it does does.
            let mut own: HashSet<Sym> = f.params.iter().map(|p| p.name).collect();
            if let ArrowBody::Block(b) = &f.body {
                declared_in(ast, *b, &mut own);
            }
            out.0.extend(inner.0.into_iter().filter(|n| !own.contains(n)));
            out.1.extend(inner.1.into_iter().filter(|n| !own.contains(n)));
            out.0.extend(nested.0);
            out.1.extend(nested.1);
            return;
        }
        _ if inside => {
            let fx = effects_of_expr(ast, e);
            out.1.extend(fx.assigned.iter().copied());
            out.0.extend(fx.assigned);
            out.0.extend(fx.roots);
            out.0.extend(fx.fields.iter().map(|&(r, _)| r));
        }
        _ => {}
    }
    collect_children(ast, e, &mut kids);
    for k in kids {
        closure_mutated_into(ast, k, inside, out);
    }
}

fn collect_children(ast: &Ast, e: ExprId, out: &mut Vec<ExprId>) {
    match &ast.expr(e).kind {
        ExprKind::Unary(_, x) | ExprKind::Paren(x) | ExprKind::NonNull(x) | ExprKind::Typeof(x) | ExprKind::Spread(x) | ExprKind::As(x, _) | ExprKind::Try(x) | ExprKind::Await(x) | ExprKind::Yield(Some(x), _) => out.push(*x),
        ExprKind::Binary(_, l, r) | ExprKind::Assign(_, l, r) => out.extend([*l, *r]),
        ExprKind::Update { target, .. } => out.push(*target),
        ExprKind::Call { callee, args, .. } | ExprKind::New { callee, args, .. } => {
            out.push(*callee);
            out.extend(args.iter().map(|a| a.expr));
        }
        ExprKind::Member { obj, .. } => out.push(*obj),
        ExprKind::Index { obj, index, .. } => out.extend([*obj, *index]),
        ExprKind::Cond(a, b, c) => out.extend([*a, *b, *c]),
        ExprKind::Array(xs) | ExprKind::Template(_, xs) | ExprKind::Seq(xs) => out.extend(xs.iter().copied()),
        ExprKind::Object(fs) => out.extend(fs.iter().map(|f| f.value)),
        _ => {}
    }
}

/// A variable, or a field or element path from one (`grid[i]!.cells`).
pub(super) fn place_path(ast: &Ast, e: ExprId) -> bool {
    use crate::ast::ExprKind;
    match &ast.expr(e).kind {
        ExprKind::Ident(_) | ExprKind::This => true,
        ExprKind::Paren(x) | ExprKind::NonNull(x) => place_path(ast, *x),
        ExprKind::Member { obj, .. } | ExprKind::Index { obj, .. } => place_path(ast, *obj),
        _ => false,
    }
}

/// Runs of named local functions (`const f = (...) => ...`, nested function declarations) side
/// by side in `stmts` where one calls a later one: they're declared together.
pub(crate) fn closure_groups(ast: &Ast, stmts: &[StmtId]) -> Vec<Vec<StmtId>> {
    let named = |s: StmtId| match &ast.stmt(s).kind {
        StmtKind::Let { mutable: false, ty: None, init: Some(i), name, .. } => match &ast.expr(*i).kind {
            ExprKind::Arrow(f) if !f.is_async => Some((*name, ast.expr(*i).span)),
            _ => None,
        },
        _ => None,
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < stmts.len() {
        let mut j = i;
        while j < stmts.len() && named(stmts[j]).is_some() {
            j += 1;
        }
        if j - i >= 2 {
            let run: Vec<(Sym, Span)> = stmts[i..j].iter().map(|&s| named(s).unwrap()).collect();
            let refers = |a: usize, b: usize| {
                let (sp, n) = (run[a].1, run[b].0);
                ast.exprs.iter().any(|e| matches!(e.kind, ExprKind::Ident(x) if x == n) && e.span.file == sp.file && e.span.start >= sp.start && e.span.end <= sp.end)
            };
            // (from the first that calls one declared after it to the last such one: those
            // before and after are checked in order, as usual)
            let first = (0..run.len()).find(|&a| (a + 1..run.len()).any(|b| refers(a, b)));
            let last = (0..run.len()).rev().find(|&b| (0..b).any(|a| refers(a, b)));
            if let (Some(a), Some(b)) = (first, last)
                && b > a
            {
                out.push(stmts[i + a..=i + b].to_vec());
            }
        }
        i = j.max(i + 1);
    }
    out
}

/// Whether `name` is tested for being there in `stmts` (`if (u)`, `!u`, `u === undefined`,
/// `u ?? d`, `u?.x`).
pub(super) fn tested(ast: &crate::ast::Ast, name: Sym, stmts: &[StmtId]) -> bool {
    use crate::ast::BinOp;
    let is = |e: ExprId| matches!(ast.expr(e).kind, ExprKind::Ident(n) if n == name);
    let absent = |e: ExprId| matches!(ast.expr(e).kind, ExprKind::Undefined | ExprKind::Null);
    let Some(last) = stmts.last() else { return false };
    let span = ast.stmt(stmts[0]).span.to(ast.stmt(*last).span);
    let inside = |sp: Span| sp.file == span.file && sp.start >= span.start && sp.end <= span.end;
    for st in ast.stmts.iter().filter(|st| inside(st.span)) {
        if let StmtKind::If(c, ..) | StmtKind::While(c, _) | StmtKind::DoWhile(_, c) = &st.kind
            && is(*c)
        {
            return true;
        }
    }
    let mut exprs = Vec::new();
    for &st in stmts {
        crate::codegen::body::collect_exprs_stmt_pub(ast, st, &mut exprs);
    }
    exprs.iter().any(|&e| match &ast.expr(e).kind {
        ExprKind::Unary(crate::ast::UnOp::Not, x) => is(*x),
        ExprKind::Cond(c, ..) => is(*c),
        ExprKind::Binary(BinOp::And | BinOp::Or | BinOp::Nullish, a, _) => is(*a),
        ExprKind::Binary(BinOp::Eq | BinOp::Ne | BinOp::LooseEq | BinOp::LooseNe, a, b) => (is(*a) && absent(*b)) || (is(*b) && absent(*a)),
        ExprKind::Member { obj, optional: true, .. } => is(*obj),
        _ => false,
    })
}

/// Whether a call of `name` in `stmts` gives a variable tested for being there (`const t =
/// peek(); if (t ...)`).
fn results_tested(ast: &crate::ast::Ast, name: Sym, stmts: &[StmtId]) -> bool {
    let Some(last) = stmts.last() else { return false };
    let span = ast.stmt(stmts[0]).span.to(ast.stmt(*last).span);
    let inside = |sp: Span| sp.file == span.file && sp.start >= span.start && sp.end <= span.end;
    let calls = |e: ExprId| matches!(&ast.expr(e).kind, ExprKind::Call { callee, .. } if matches!(ast.expr(*callee).kind, ExprKind::Ident(n) if n == name));
    ast.stmts.iter().filter(|st| inside(st.span)).any(|st| match &st.kind {
        StmtKind::Let { name: t, init: Some(x), .. } if calls(*x) => {
            // (tested anywhere in what follows it, inside the same statements)
            let after: Vec<StmtId> = stmts.to_vec();
            tested(ast, *t, &after)
        }
        _ => false,
    })
}

/// Is `ne` (a `new Array(n)`) the start of an array the next statement fills, one element a
/// round, in order: `const xs: T[] = new Array(n); for (let i = 0; i < n; i++) { ...; xs[i] = v; }`
/// with `xs` used nowhere else before (the loop is the first statement after to mention it)? It's then made empty (with room for `n`), and each
/// store appends: no element is ever missing (a read before its store is out of bounds).
pub(super) fn filled_by_next_loop(ast: &Ast, ne: ExprId) -> bool {
    let local = |e: ExprId, s: Sym| matches!(ast.expr(e).kind, ExprKind::Ident(n) if n == s);
    let mentions = |s: StmtId, x: Sym| {
        let mut es = Vec::new();
        crate::codegen::body::collect_exprs_stmt_pub(ast, s, &mut es);
        es.iter().filter(|&&e| local(e, x)).count()
    };
    fn jumps(ast: &Ast, s: StmtId) -> bool {
        match &ast.stmt(s).kind {
            StmtKind::Break(_) | StmtKind::Continue(_) => true,
            StmtKind::Block(ss) => ss.iter().any(|&x| jumps(ast, x)),
            StmtKind::If(_, t, e) => jumps(ast, *t) || e.is_some_and(|e| jumps(ast, e)),
            StmtKind::While(_, b) | StmtKind::DoWhile(b, _) | StmtKind::For { body: b, .. } | StmtKind::ForOf { body: b, .. } => jumps(ast, *b),
            StmtKind::Switch(_, cases) => cases.iter().flat_map(|c| c.body.iter()).any(|&x| jumps(ast, x)),
            StmtKind::Try { body, catch, finally } => jumps(ast, *body) || catch.as_ref().is_some_and(|c| jumps(ast, c.body)) || finally.is_some_and(|f| jumps(ast, f)),
            _ => false,
        }
    }
    for st in &ast.stmts {
        let StmtKind::Block(ss) = &st.kind else { continue };
        // (a script's top-level `const`: initialized where it's declared)
        let declared = |s: StmtId| match ast.stmt(s).kind {
            StmtKind::Let { name, init: Some(i), .. } if i == ne => Some(name),
            StmtKind::InitGlobal(item) => match ast.items[item as usize].kind {
                crate::ast::ItemKind::Const { name, init, .. } if init == ne => Some(name),
                _ => None,
            },
            _ => None,
        };
        let Some((k, x)) = ss.iter().enumerate().find_map(|(k, &s)| declared(s).map(|x| (k, x))) else { continue };
        // (statements between that don't mention `xs` can't see it)
        let Some(&next) = ss[k + 1..].iter().find(|&&s| mentions(s, x) > 0) else { return false };
        let StmtKind::For { init: Some(init), cond: Some(cond), step: Some(step), body } = &ast.stmt(next).kind else { return false };
        let StmtKind::Let { name: i, init: Some(zero), .. } = ast.stmt(*init).kind else { return false };
        if !matches!(ast.expr(zero).kind, ExprKind::Int(0)) {
            return false;
        }
        let ExprKind::Binary(BinOp::Lt, l, bound) = ast.expr(*cond).kind else { return false };
        let one_up = match &ast.expr(*step).kind {
            ExprKind::Update { inc: true, target, .. } => local(*target, i),
            ExprKind::Assign(AssignOp::Op(BinOp::Add), t, one) => local(*t, i) && matches!(ast.expr(*one).kind, ExprKind::Int(1)),
            _ => false,
        };
        let mut bound_exprs = Vec::new();
        crate::codegen::body::collect_exprs_expr(ast, bound, &mut bound_exprs);
        if !local(l, i) || !one_up || bound_exprs.iter().any(|&e| local(e, x)) {
            return false;
        }
        // (`i` changes only in the step)
        let mut es = Vec::new();
        crate::codegen::body::collect_exprs_stmt_pub(ast, *body, &mut es);
        if es.iter().any(|&e| matches!(&ast.expr(e).kind, ExprKind::Assign(_, t, _) | ExprKind::Update { target: t, .. } if local(*t, i))) {
            return false;
        }
        let body_ss: Vec<StmtId> = match &ast.stmt(*body).kind {
            StmtKind::Block(bs) => bs.clone(),
            _ => vec![*body],
        };
        // the one mention of `xs` in the loop: a store at `i`, every round
        let Some(j) = body_ss.iter().position(|&s| mentions(s, x) > 0) else { return false };
        let StmtKind::Expr(store) = ast.stmt(body_ss[j]).kind else { return false };
        let ExprKind::Assign(AssignOp::Assign, place, v) = ast.expr(store).kind else { return false };
        let ExprKind::Index { obj, index, optional: false } = ast.expr(place).kind else { return false };
        let mut ves = Vec::new();
        crate::codegen::body::collect_exprs_expr(ast, v, &mut ves);
        return local(obj, x)
            && local(index, i)
            && !ves.iter().any(|&e| local(e, x))
            && body_ss[j + 1..].iter().all(|&s| mentions(s, x) == 0)
            && !body_ss[..j].iter().any(|&s| jumps(ast, s));
    }
    false
}

/// The object an enum's name refers to (`fact`, an identifier in module `m`): the module's AST
/// and the object literal. An enum is a constant object with a type alias of the same name.
pub(crate) fn enum_object(modules: &[Module], m: u32, fact: IdentFact) -> Option<(&Ast, ExprId)> {
    match fact {
        IdentFact::Const(cm, ci) => {
            let mast = &modules[cm as usize].ast;
            let ItemKind::Const { name, init, mutable: false, .. } = &mast.items[ci as usize].kind else { return None };
            mast.items.iter().any(|it| matches!(&it.kind, ItemKind::TypeAlias { name: n, .. } if n == name)).then_some((mast, *init))
        }
        // (a script's: a local of the function running its statements)
        IdentFact::Local(k) => {
            let mast = &modules[m as usize].ast;
            let (name, init) = mast.stmts.iter().find_map(|st| match &st.kind {
                StmtKind::Let { name, name_span, init: Some(i), mutable: false, .. } if name_span.start == k => Some((*name, *i)),
                _ => None,
            })?;
            let is_obj = matches!(mast.expr(init).kind, ExprKind::Object(_));
            (is_obj && mast.items.iter().any(|it| matches!(&it.kind, ItemKind::TypeAlias { name: n, .. } if *n == name))).then_some((mast, init))
        }
        _ => None,
    }
}
