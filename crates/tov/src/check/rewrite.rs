//! TypeScript habits checking finds and the driver rewrites before checking again: arguments for
//! `inout` parameters passed without `&` (V0110) and `const`s passed to them (V0001, V0102), as
//! if the program said `&` and `let`; `return p` from async functions, RegExp string methods,
//! match indexes, `Record` keys, spreads and keyed field reads.

use super::*;
use crate::ast::{Arg, ExprKind, StmtKind};

/// What a check found to rewrite.
#[derive(Default, Clone, Debug)]
pub struct Rewrites {
    /// Arguments to `inout` parameters given without `&`: (module, argument).
    pub(crate) args: Vec<(u32, ExprId)>,
    /// `const` locals (module, the name's span start) passed to `inout` parameters.
    pub(crate) consts: Vec<(u32, u32)>,
    /// Module constants (module, item) passed to `inout` parameters.
    pub(crate) items: Vec<(u32, u32)>,
    /// Promises returned by async functions (module, the returned expression): `return await p`.
    pub(crate) awaits: Vec<(u32, ExprId)>,
    /// `s.replace(re, x)` and the other string methods taking a RegExp: (module, the call, the
    /// prelude function), as `__reReplace(s, re, x)`.
    pub(crate) calls: Vec<(u32, ExprId, crate::intern::Sym)>,
    /// `{ type: "x", extra: 1 }` of a record with an index signature: (module, the literal,
    /// `$rest`, the other keys), as `{ type: "x", $rest: { extra: 1 } }`.
    /// (and `...`, a spread field's name)
    pub(crate) rest_lits: Vec<(u32, ExprId, crate::intern::Sym, Vec<crate::intern::Sym>, crate::intern::Sym)>,
    /// `g[k]` of a record with an index signature: (module, the index, `$rest`), as `g.$rest?.[k]`.
    pub(crate) rest_indexes: Vec<(u32, ExprId, crate::intern::Sym)>,
    /// `xs.at(k)` written through (`xs.at(-1)!.src = s`): (module, the call, `xs`, `k`, `length`),
    /// as `xs[k]` / `xs[xs.length - k]`.
    pub(crate) at_places: Vec<(u32, ExprId, ExprId, i64, crate::intern::Sym)>,
    /// `f.call(t, ...)` (0), `f.apply(t, xs)` (1), `f.bind(t)` (2) of a function: (module, the call,
    /// which), as `f(...)`, `f(...xs)`, `f`.
    /// (and the `.call` member, so it's rewritten once)
    pub(crate) fn_methods: Vec<(u32, ExprId, u8, ExprId)>,
    /// `{ type: c ? "a" : "b", ... }` of variants: (module, the literal, the field), as
    /// `c ? { type: "a", ... } : { type: "b", ... }`.
    pub(crate) split_lits: Vec<(u32, ExprId, usize)>,
    /// `x.m.call(t, ...)` (0) / `x.m.apply(t, xs)` (1) of a class's method: (module, the call,
    /// which, optional), as `x.m(...)` (`x?.m(...)`).
    /// (and the `.call` member, so it's rewritten once)
    pub(crate) method_values: Vec<(u32, ExprId, u8, bool, ExprId)>,
    /// `Object.create(null)`: (module, the call), as `{}`.
    pub(crate) empty_objects: Vec<(u32, ExprId)>,
    /// `r["field"]` in TypeScript: (module, the index, the field), as `r.field`.
    pub(crate) str_members: Vec<(u32, ExprId, crate::intern::Sym)>,
    /// `m[i]` on a match: (module, the index expression, `items`, assert present), as `m.items[i]` (`!`).
    /// (mode: 0 as it is, 1 asserted present `!`, 2 `?? ""`)
    pub(crate) elems: Vec<(u32, ExprId, crate::intern::Sym, u8)>,
    /// `r.key` on a `Record<string, V>` (module, the member expression), as `r["key"]`.
    pub(crate) keys: Vec<(u32, ExprId)>,
    /// `{ ...a, y: 3 }`: (module, the literal, each spread's fields), as `{ x: a.x, y: 3 }`.
    /// (each spread's field names, and whether the field is optional: `b.f ?? a.f` over an earlier one)
    pub(crate) spreads: Vec<(u32, ExprId, Vec<Vec<(crate::intern::Sym, bool)>>)>,
    /// `p[k]`, `k` one of these field names: (module, the index), as `k === "a" ? p.a : p.b`.
    pub(crate) keyed: Vec<(u32, ExprId, Vec<crate::intern::Sym>)>,
    /// A class instance where a number is wanted (`Number(x)`, `+x`): (module, the value, the
    /// method), as `x.valueOf()` (JavaScript's conversion calls it).
    pub(crate) prims: Vec<(u32, ExprId, crate::intern::Sym)>,
    /// `t[1] = v`, `t[1] += v` of a tuple: the element's field, `t.1` (module, index expression, field).
    pub(crate) tuple_places: Vec<(u32, ExprId, crate::intern::Sym)>,
    /// `g.next().value` where a value must be: `g.next().value!` (module, expression).
    pub(crate) nonnulls: Vec<(u32, ExprId)>,
    /// A callback's value where a `boolean` is wanted (`xs.filter(o => o.note)`): `!!(x)`, its
    /// truthiness, as JavaScript tests it (module, the arrow's body).
    pub(crate) truthy: Vec<(u32, ExprId)>,
    /// `Object.hasOwn(o, k)` / `o.hasOwnProperty(k)` of a record or a dictionary: (module, the
    /// call, o, k), as `k in o`.
    pub(crate) has_own: Vec<(u32, ExprId, ExprId, ExprId)>,
    /// `Object.fromEntries(x)`: (module, the call, the prelude function), as `__fromEntries(x)`.
    pub(crate) ns_calls: Vec<(u32, ExprId, crate::intern::Sym)>,
    /// `xs.flat(d)`: (module, the call, levels, `flat`, `slice`), as `xs.flat().flat()` (d of
    /// them; none: `xs.slice()`).
    pub(crate) flat_chains: Vec<(u32, ExprId, u32, crate::intern::Sym, crate::intern::Sym)>,
    /// `m.map(f)` on a match (an array in JavaScript): (module, the member, `items`), as `m.items.map(f)`.
    pub(crate) elem_methods: Vec<(u32, ExprId, crate::intern::Sym)>,
    /// `m as { a: T, b?: U }` of a `Record<string, V>`: (module, the cast, `m`, each field and
    /// whether it may be absent).
    pub(crate) record_casts: Vec<(u32, ExprId, ExprId, Vec<(crate::intern::Sym, bool)>)>,
    /// `{ [Color.Red]: v }` where a record type is expected (every key a string literal): (module,
    /// the `new Map(...)` it was written as, the field names), as `{ red: v }`.
    pub(crate) records: Vec<(u32, ExprId, Vec<crate::intern::Sym>)>,
    /// A built-in function as a value (`xs.map(Number)`): (module, the name, a parameter name),
    /// as `(v) => Number(v)`.
    pub(crate) fn_values: Vec<(u32, ExprId, crate::intern::Sym)>,
    /// `Direction[d]` on a numeric enum: (module, the enum, its member names), as
    /// `["Up", "Down"][d]`.
    pub(crate) enum_names: Vec<(u32, ExprId, Vec<crate::intern::Sym>)>,
    /// `const { a, ...others } = r`: (module, the `__rest(r, "a")` call, the other fields), as
    /// `{ b: r.b, c: r.c }`.
    pub(crate) rests: Vec<(u32, ExprId, Vec<crate::intern::Sym>)>,
    /// `x.name = v` (or `op=`) through a setter: (module, the assignment, the setter method), as
    /// `x.__set_name(v)` (`x.__set_name(x.name op v)`).
    pub(crate) setters: Vec<(u32, ExprId, crate::intern::Sym)>,
    /// `Promise.allSettled(ps)`: (module, the call, the prelude function), as `__allSettled(ps)`.
    pub(crate) static_calls: Vec<(u32, ExprId, crate::intern::Sym)>,
    /// `a ||= b`, `a &&= b` on a variable or field path: (module, the assignment), as
    /// `a = a || b`.
    pub(crate) logical: Vec<(u32, ExprId)>,
    /// `x as T`, T a record with x's fields and optional ones more: (module, the cast, the
    /// spread field's name), as `{ ...x } as T`.
    pub(crate) widen_casts: Vec<(u32, ExprId, crate::intern::Sym)>,
    /// `Object.assign(t, p, ...)`: (module, the call, each source with its fields (optional or
    /// not)), as `(t.a = p.a, t.b = p.b ?? t.b, t)`.
    pub(crate) assigns: Vec<(u32, ExprId, Vec<(ExprId, Vec<(crate::intern::Sym, bool)>)>)>,
    /// `Object.freeze(x)`: (module, the call), as `(x)`.
    pub(crate) identities: Vec<(u32, ExprId)>,
    /// `counts[k]++` / `counts[k] op= v`, `k` one of the record's field names: (module, the
    /// assignment, the names), as `k === "a" ? counts.a++ : counts.b++`.
    pub(crate) keyed_places: Vec<(u32, ExprId, Vec<crate::intern::Sym>)>,
    /// `for (const x of bag)`, `[...bag]`, bag's class having `[Symbol.iterator]()`: (module, the
    /// expression), as `bag.__iterator()`.
    pub(crate) iterators: Vec<(u32, ExprId)>,
}

impl Rewrites {
    pub fn is_empty(&self) -> bool {
        self.args.is_empty() && self.truthy.is_empty() && self.has_own.is_empty() && self.ns_calls.is_empty() && self.flat_chains.is_empty() && self.elem_methods.is_empty() && self.prims.is_empty() && self.tuple_places.is_empty() && self.nonnulls.is_empty() && self.record_casts.is_empty() && self.records.is_empty() && self.fn_values.is_empty() && self.enum_names.is_empty() && self.rests.is_empty() && self.setters.is_empty() && self.static_calls.is_empty() && self.logical.is_empty() && self.widen_casts.is_empty() && self.assigns.is_empty() && self.identities.is_empty() && self.keyed_places.is_empty() && self.iterators.is_empty() && self.consts.is_empty() && self.items.is_empty() && self.awaits.is_empty() && self.calls.is_empty() && self.elems.is_empty() && self.rest_lits.is_empty() && self.rest_indexes.is_empty() && self.str_members.is_empty() && self.at_places.is_empty() && self.fn_methods.is_empty() && self.empty_objects.is_empty() && self.split_lits.is_empty() && self.method_values.is_empty() && self.keys.is_empty() && self.spreads.is_empty() && self.keyed.is_empty()
    }

    pub(crate) fn extend(&mut self, o: Rewrites) {
        self.args.extend(o.args);
        self.consts.extend(o.consts);
        self.items.extend(o.items);
        self.awaits.extend(o.awaits);
        self.calls.extend(o.calls);
        self.elems.extend(o.elems);
        self.rest_lits.extend(o.rest_lits);
        self.rest_indexes.extend(o.rest_indexes);
        self.str_members.extend(o.str_members);
        self.at_places.extend(o.at_places);
        self.fn_methods.extend(o.fn_methods);
        self.empty_objects.extend(o.empty_objects);
        self.split_lits.extend(o.split_lits);
        self.method_values.extend(o.method_values);
        self.keys.extend(o.keys);
        self.spreads.extend(o.spreads);
        self.keyed.extend(o.keyed);
        self.prims.extend(o.prims);
        self.tuple_places.extend(o.tuple_places);
        self.nonnulls.extend(o.nonnulls);
        self.truthy.extend(o.truthy);
        self.has_own.extend(o.has_own);
        self.ns_calls.extend(o.ns_calls);
        self.flat_chains.extend(o.flat_chains);
        self.elem_methods.extend(o.elem_methods);
        self.record_casts.extend(o.record_casts);
        self.records.extend(o.records);
        self.fn_values.extend(o.fn_values);
        self.enum_names.extend(o.enum_names);
        self.rests.extend(o.rests);
        self.setters.extend(o.setters);
        self.static_calls.extend(o.static_calls);
        self.logical.extend(o.logical);
        self.widen_casts.extend(o.widen_casts);
        self.assigns.extend(o.assigns);
        self.identities.extend(o.identities);
        self.keyed_places.extend(o.keyed_places);
        self.iterators.extend(o.iterators);
    }
}

/// Applies `rw` to the program; true if anything changed.
pub fn apply(modules: &mut [Module], rw: &Rewrites) -> bool {
    let mut changed = false;
    for &(m, arg) in &rw.args {
        let ast = &mut modules[m as usize].ast;
        let span = ast.exprs[arg as usize].span;
        for e in ast.exprs.iter_mut() {
            if let ExprKind::Call { args, .. } | ExprKind::New { args, .. } = &mut e.kind
                && let Some(a) = args.iter_mut().find(|a| a.expr == arg && a.by_ref.is_none())
            {
                a.by_ref = Some(Span::new(span.file, span.start, span.start));
                changed = true;
            }
        }
    }
    for &(m, start) in &rw.consts {
        for s in modules[m as usize].ast.stmts.iter_mut() {
            if let StmtKind::Let { mutable, name_span, .. } = &mut s.kind
                && name_span.start == start
                && !*mutable
            {
                *mutable = true;
                changed = true;
            }
        }
    }
    for &(m, call, f) in &rw.calls {
        // `s.replace(re, x)` → `__reReplace(s, re, x)`
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Call { callee, args, .. } = &ast.exprs[call as usize].kind else { continue };
        let ExprKind::Member { obj, name_span, .. } = ast.exprs[*callee as usize].kind else { continue };
        let mut all = vec![Arg { expr: obj, by_ref: None }];
        all.extend(args.iter().map(|a| Arg { expr: a.expr, by_ref: a.by_ref }));
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Ident(f), span: name_span });
        let fe = ast.exprs.len() as ExprId - 1;
        if let ExprKind::Call { callee, args, .. } = &mut ast.exprs[call as usize].kind {
            *callee = fe;
            *args = all;
        }
        changed = true;
    }
    for &(m, e) in &rw.iterators {
        let ast = &mut modules[m as usize].ast;
        let span = ast.exprs[e as usize].span;
        let inner = crate::ast::Expr { kind: std::mem::replace(&mut ast.exprs[e as usize].kind, ExprKind::Error), span };
        ast.exprs.push(inner);
        let obj = ast.exprs.len() as ExprId - 1;
        let Some(name) = ast.iterator_sym else { continue };
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj, name, name_span: span, optional: false }, span });
        let callee = ast.exprs.len() as ExprId - 1;
        ast.exprs[e as usize].kind = ExprKind::Call { callee, type_args: Vec::new(), args: Vec::new(), optional: false };
        changed = true;
    }
    for (m, e, keys) in &rw.keyed_places {
        let ast = &mut modules[*m as usize].ast;
        let span = ast.exprs[*e as usize].span;
        // (each branch its own copy of the value: each is checked where its test narrows)
        let (target, value, assign_op, update) = match ast.exprs[*e as usize].kind {
            ExprKind::Assign(op, t, v) => (t, Some(v), Some(op), None),
            ExprKind::Update { inc, prefix, target } => (target, None, None, Some((inc, prefix))),
            _ => continue,
        };
        let rebuild = |ast: &mut crate::ast::Ast, nt: ExprId| -> Option<ExprKind> {
            Some(match (assign_op, update) {
                (Some(op), _) => ExprKind::Assign(op, nt, deep_copy(ast, value?)?),
                (_, Some((inc, prefix))) => ExprKind::Update { inc, prefix, target: nt },
                _ => return None,
            })
        };
        let ExprKind::Index { obj, index, .. } = ast.exprs[target as usize].kind else { continue };
        // the last key needs no test: the others are each `k === "name"`
        let mut chain: Option<ExprId> = None;
        for (i, &name) in keys.iter().enumerate().rev() {
            let Some(o) = copy_path(ast, obj) else { break };
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj: o, name, name_span: span, optional: false }, span });
            let member = ast.exprs.len() as ExprId - 1;
            let Some(kind) = rebuild(ast, member) else { break };
            ast.exprs.push(crate::ast::Expr { kind, span });
            let branch = ast.exprs.len() as ExprId - 1;
            chain = Some(match chain {
                None if i == keys.len() - 1 => branch,
                Some(rest) => {
                    let Some(k) = copy_path(ast, index) else { break };
                    ast.exprs.push(crate::ast::Expr { kind: ExprKind::Str(name), span });
                    let lit = ast.exprs.len() as ExprId - 1;
                    ast.exprs.push(crate::ast::Expr { kind: ExprKind::Binary(crate::ast::BinOp::Eq, k, lit), span });
                    let test = ast.exprs.len() as ExprId - 1;
                    ast.quiet_tests.push(test);
                    ast.exprs.push(crate::ast::Expr { kind: ExprKind::Cond(test, branch, rest), span });
                    ast.exprs.len() as ExprId - 1
                }
                None => branch,
            });
        }
        let Some(chain) = chain else { continue };
        let kind = std::mem::replace(&mut ast.exprs[chain as usize].kind, ExprKind::Error);
        ast.exprs[*e as usize].kind = kind;
        changed = true;
    }
    for &(m, call) in &rw.identities {
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Call { args, .. } = &ast.exprs[call as usize].kind else { continue };
        let x = args[0].expr;
        ast.exprs[call as usize].kind = ExprKind::Paren(x);
        changed = true;
    }
    for (m, call, sources) in &rw.assigns {
        let ast = &mut modules[*m as usize].ast;
        let ExprKind::Call { args, .. } = &ast.exprs[*call as usize].kind else { continue };
        let target = args[0].expr;
        let span = ast.exprs[*call as usize].span;
        let mut seq = Vec::new();
        for (src, fields) in sources {
            for &(name, optional) in fields {
                let Some(t) = copy_path(ast, target) else { continue };
                ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj: t, name, name_span: span, optional: false }, span });
                let lhs = ast.exprs.len() as ExprId - 1;
                let Some(s) = copy_path(ast, *src) else { continue };
                ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj: s, name, name_span: span, optional: false }, span });
                let mut rhs = ast.exprs.len() as ExprId - 1;
                if optional {
                    let Some(t2) = copy_path(ast, target) else { continue };
                    ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj: t2, name, name_span: span, optional: false }, span });
                    let old = ast.exprs.len() as ExprId - 1;
                    ast.exprs.push(crate::ast::Expr { kind: ExprKind::Binary(crate::ast::BinOp::Nullish, rhs, old), span });
                    rhs = ast.exprs.len() as ExprId - 1;
                }
                ast.exprs.push(crate::ast::Expr { kind: ExprKind::Assign(crate::ast::AssignOp::Assign, lhs, rhs), span });
                seq.push(ast.exprs.len() as ExprId - 1);
            }
        }
        let Some(t) = copy_path(ast, target) else { continue };
        seq.push(t);
        ast.exprs[*call as usize].kind = ExprKind::Seq(seq);
        changed = true;
    }
    for &(m, e, dots) in &rw.widen_casts {
        let ast = &mut modules[m as usize].ast;
        let ExprKind::As(x, t) = ast.exprs[e as usize].kind else { continue };
        let span = ast.exprs[x as usize].span;
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Spread(x), span });
        let value = ast.exprs.len() as ExprId - 1;
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Object(vec![crate::ast::ObjField { name: dots, name_span: span, value, quoted: false }]), span });
        let obj = ast.exprs.len() as ExprId - 1;
        ast.exprs[e as usize].kind = ExprKind::As(obj, t);
        changed = true;
    }
    for &(m, e) in &rw.logical {
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Assign(crate::ast::AssignOp::Op(op), target, value) = ast.exprs[e as usize].kind else { continue };
        let Some(read) = copy_path(ast, target) else { continue };
        let span = ast.exprs[e as usize].span;
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Binary(op, read, value), span });
        let v = ast.exprs.len() as ExprId - 1;
        ast.exprs[e as usize].kind = ExprKind::Assign(crate::ast::AssignOp::Assign, target, v);
        changed = true;
    }
    for &(m, call, f) in &rw.static_calls {
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Call { callee, .. } = ast.exprs[call as usize].kind else { continue };
        let span = ast.exprs[callee as usize].span;
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Ident(f), span });
        let fe = ast.exprs.len() as ExprId - 1;
        if let ExprKind::Call { callee, .. } = &mut ast.exprs[call as usize].kind {
            *callee = fe;
        }
        changed = true;
    }
    for &(m, at, kind, opt, member) in &rw.method_values {
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Call { callee, args, .. } = &ast.exprs[at as usize].kind else { continue };
        if *callee != member {
            continue;
        }
        let ExprKind::Member { obj: inner, .. } = ast.exprs[*callee as usize].kind else { continue };
        let ExprKind::Member { obj: x, name, name_span, .. } = ast.exprs[inner as usize].kind else { continue };
        let rest: Vec<Arg> = args.iter().skip(1).map(|a| Arg { expr: a.expr, by_ref: a.by_ref }).collect();
        let span = ast.exprs[at as usize].span;
        let args = if kind == 1 {
            match rest.first() {
                Some(a) if let ExprKind::Array(xs) = &ast.exprs[a.expr as usize].kind => xs.iter().map(|&x| Arg { expr: x, by_ref: None }).collect(),
                Some(a) => {
                    ast.exprs.push(crate::ast::Expr { kind: ExprKind::Spread(a.expr), span });
                    vec![Arg { expr: ast.exprs.len() as ExprId - 1, by_ref: None }]
                }
                None => Vec::new(),
            }
        } else {
            rest
        };
        ast.exprs[inner as usize].kind = ExprKind::Member { obj: x, name, name_span, optional: opt };
        ast.exprs[at as usize].kind = ExprKind::Call { callee: inner, type_args: Vec::new(), args, optional: false };
        changed = true;
    }
    for &(m, at, k) in &rw.split_lits {
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Object(fields) = &ast.exprs[at as usize].kind else { continue };
        let Some(f) = fields.get(k) else { continue };
        let ExprKind::Cond(c, a, b) = ast.exprs[f.value as usize].kind else { continue };
        let span = ast.exprs[at as usize].span;
        let make = |ast: &mut crate::ast::Ast, v: ExprId| {
            let ExprKind::Object(fields) = &ast.exprs[at as usize].kind else { unreachable!() };
            let fs: Vec<crate::ast::ObjField> = fields.iter().enumerate().map(|(i, f)| crate::ast::ObjField { name: f.name, name_span: f.name_span, value: if i == k { v } else { f.value }, quoted: f.quoted }).collect();
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Object(fs), span });
            ast.exprs.len() as ExprId - 1
        };
        let (oa, ob) = (make(ast, a), make(ast, b));
        ast.exprs[at as usize].kind = ExprKind::Cond(c, oa, ob);
        changed = true;
    }
    for &(m, at) in &rw.empty_objects {
        let ast = &mut modules[m as usize].ast;
        if matches!(ast.exprs[at as usize].kind, ExprKind::Call { .. }) {
            ast.exprs[at as usize].kind = ExprKind::Object(Vec::new());
            changed = true;
        }
    }
    for &(m, at, kind, member) in &rw.fn_methods {
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Call { callee, args, .. } = &ast.exprs[at as usize].kind else { continue };
        if *callee != member {
            continue;
        }
        let ExprKind::Member { obj: f, .. } = ast.exprs[*callee as usize].kind else { continue };
        let span = ast.exprs[at as usize].span;
        let rest: Vec<Arg> = args.iter().skip(1).map(|a| Arg { expr: a.expr, by_ref: a.by_ref }).collect();
        ast.exprs[at as usize].kind = match kind {
            0 => ExprKind::Call { callee: f, type_args: Vec::new(), args: rest, optional: false },
            1 => {
                let args = match rest.first() {
                    // (`f.apply(t, [a, b])`: `f(a, b)`)
                    Some(a) if let ExprKind::Array(xs) = &ast.exprs[a.expr as usize].kind => xs.iter().map(|&x| Arg { expr: x, by_ref: None }).collect(),
                    Some(a) => {
                        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Spread(a.expr), span });
                        vec![Arg { expr: ast.exprs.len() as ExprId - 1, by_ref: None }]
                    }
                    None => Vec::new(),
                };
                ExprKind::Call { callee: f, type_args: Vec::new(), args, optional: false }
            }
            _ => ExprKind::Paren(f),
        };
        changed = true;
    }
    for &(m, at, arr, k, length) in &rw.at_places {
        let ast = &mut modules[m as usize].ast;
        if !matches!(ast.exprs[at as usize].kind, ExprKind::Call { .. }) {
            continue;
        }
        let span = ast.exprs[at as usize].span;
        let index = if k >= 0 {
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Int(k as u64), span });
            ast.exprs.len() as ExprId - 1
        } else {
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj: arr, name: length, name_span: span, optional: false }, span });
            let len = ast.exprs.len() as ExprId - 1;
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Int((-k) as u64), span });
            let n = ast.exprs.len() as ExprId - 1;
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Binary(crate::ast::BinOp::Sub, len, n), span });
            ast.exprs.len() as ExprId - 1
        };
        ast.exprs[at as usize].kind = ExprKind::Index { obj: arr, index, optional: false };
        changed = true;
    }
    for &(m, at, name) in &rw.str_members {
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Index { obj, index, optional } = ast.exprs[at as usize].kind else { continue };
        let name_span = ast.exprs[index as usize].span;
        ast.exprs[at as usize].kind = ExprKind::Member { obj, name, name_span, optional };
        changed = true;
    }
    for &(m, at, rest) in &rw.rest_indexes {
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Index { obj, optional, .. } = ast.exprs[at as usize].kind else { continue };
        if matches!(ast.exprs[obj as usize].kind, ExprKind::Member { name, .. } if name == rest) {
            continue;
        }
        let span = ast.exprs[obj as usize].span;
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj, name: rest, name_span: span, optional }, span });
        let member = ast.exprs.len() as ExprId - 1;
        if let ExprKind::Index { obj, optional, .. } = &mut ast.exprs[at as usize].kind {
            *obj = member;
            *optional = true;
        }
        changed = true;
    }
    for (m, at, rest, keys, dots) in &rw.rest_lits {
        let ast = &mut modules[*m as usize].ast;
        let ExprKind::Object(fields) = &mut ast.exprs[*at as usize].kind else { continue };
        let (extra, kept): (Vec<_>, Vec<_>) = std::mem::take(fields).into_iter().partition(|f| keys.contains(&f.name));
        let Some(first) = extra.first() else {
            *fields = kept;
            continue;
        };
        let span = first.name_span;
        let name_span = span;
        *fields = kept;
        // (a `$rest` already there, from a spread: its keys first, `{ ...old, extra }`)
        let old = fields.iter().position(|f| f.name == *rest).map(|i| fields.remove(i));
        let mut inner_fields: Vec<crate::ast::ObjField> = Vec::new();
        if let Some(old) = old {
            let sp = ast.exprs.len() as ExprId;
            let vspan = ast.exprs[old.value as usize].span;
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Spread(old.value), span: vspan });
            inner_fields.push(crate::ast::ObjField { name: *dots, name_span: old.name_span, value: sp, quoted: false });
        }
        inner_fields.extend(extra.into_iter().map(|f| crate::ast::ObjField { quoted: true, ..f }));
        let inner = ast.exprs.len() as ExprId;
        let fields_span = ast.exprs[*at as usize].span;
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Object(inner_fields), span: fields_span });
        if let ExprKind::Object(fields) = &mut ast.exprs[*at as usize].kind {
            fields.push(crate::ast::ObjField { name: *rest, name_span, value: inner, quoted: false });
        }
        changed = true;
    }
    for &(m, at, items, present) in &rw.elems {
        // `m[i]` → `m.items[i]` (`!` when it's read as present)
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Index { obj, optional, .. } = ast.exprs[at as usize].kind else { continue };
        if matches!(ast.exprs[obj as usize].kind, ExprKind::Member { name, .. } if name == items) {
            continue;
        }
        let span = ast.exprs[obj as usize].span;
        // (`m?.[1]`: `m?.items[1]`, the chain short-circuited where `m` isn't there)
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj, name: items, name_span: span, optional }, span });
        let member = ast.exprs.len() as ExprId - 1;
        let at_span = ast.exprs[at as usize].span;
        if let ExprKind::Index { obj, .. } = &mut ast.exprs[at as usize].kind {
            *obj = member;
        }
        if present == 1 {
            let index = std::mem::replace(&mut ast.exprs[at as usize].kind, ExprKind::Error);
            ast.exprs.push(crate::ast::Expr { kind: index, span: at_span });
            let inner = ast.exprs.len() as ExprId - 1;
            ast.exprs[at as usize].kind = ExprKind::NonNull(inner);
        } else if present == 2 {
            let index = std::mem::replace(&mut ast.exprs[at as usize].kind, ExprKind::Error);
            ast.exprs.push(crate::ast::Expr { kind: index, span: at_span });
            let inner = ast.exprs.len() as ExprId - 1;
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Template(vec![String::new()], Vec::new()), span: at_span });
            let dflt = ast.exprs.len() as ExprId - 1;
            ast.exprs[at as usize].kind = ExprKind::Binary(crate::ast::BinOp::Nullish, inner, dflt);
        }
        changed = true;
    }
    for (m, at, expansions) in &rw.spreads {
        // later fields win, at the place of the first (as JavaScript's keys)
        let ast = &mut modules[*m as usize].ast;
        let ExprKind::Object(fields) = &ast.exprs[*at as usize].kind else { continue };
        let entries: Vec<(crate::intern::Sym, Span, ExprId)> = fields.iter().map(|f| (f.name, f.name_span, f.value)).collect();
        let mut out: Vec<crate::ast::ObjField> = Vec::new();
        let mut k = 0;
        for (name, nspan, value) in entries {
            let ExprKind::Spread(x) = ast.exprs[value as usize].kind else {
                match out.iter_mut().find(|o| o.name == name) {
                    Some(o) => o.value = value,
                    None => out.push(crate::ast::ObjField { name, name_span: nspan, value, quoted: false }),
                }
                continue;
            };
            // (a literal spread: its own fields, here)
            if let ExprKind::Object(inner) = &ast.exprs[x as usize].kind
                && !inner.iter().any(|f| matches!(ast.exprs[f.value as usize].kind, ExprKind::Spread(_)))
            {
                let inner: Vec<(crate::intern::Sym, Span, ExprId)> = inner.iter().map(|f| (f.name, f.name_span, f.value)).collect();
                for (iname, ispan, ivalue) in inner {
                    match out.iter_mut().find(|o| o.name == iname) {
                        Some(o) => o.value = ivalue,
                        None => out.push(crate::ast::ObjField { name: iname, name_span: ispan, value: ivalue, quoted: false }),
                    }
                }
                continue;
            }
            let Some(names) = expansions.get(k) else { continue };
            k += 1;
            for &(fname, optional) in names {
                let obj = clone_path(ast, x);
                let span = ast.exprs[x as usize].span;
                ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj, name: fname, name_span: span, optional: false }, span });
                let read = ast.exprs.len() as ExprId - 1;
                match out.iter_mut().find(|o| o.name == fname) {
                    // (an optional field kept over an earlier one where it's absent)
                    Some(o) if optional => {
                        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Binary(crate::ast::BinOp::Nullish, read, o.value), span });
                        o.value = ast.exprs.len() as ExprId - 1;
                    }
                    Some(o) => o.value = read,
                    None => out.push(crate::ast::ObjField { name: fname, name_span: nspan, value: read, quoted: false }),
                }
            }
        }
        ast.exprs[*at as usize].kind = ExprKind::Object(out);
        changed = true;
    }
    for (m, at, keys) in &rw.keyed {
        let ast = &mut modules[*m as usize].ast;
        let ExprKind::Index { obj, index, .. } = ast.exprs[*at as usize].kind else { continue };
        let span = ast.exprs[*at as usize].span;
        let field = |ast: &mut crate::ast::Ast, name| {
            let o = clone_path(ast, obj);
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj: o, name, name_span: span, optional: false }, span });
            ast.exprs.len() as ExprId - 1
        };
        let Some((&last, rest)) = keys.split_last() else { continue };
        let mut e = field(ast, last);
        for &k in rest.iter().rev() {
            let key = clone_path(ast, index);
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Str(k), span });
            let lit = ast.exprs.len() as ExprId - 1;
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Binary(crate::ast::BinOp::Eq, key, lit), span });
            let test = ast.exprs.len() as ExprId - 1;
            ast.quiet_tests.push(test);
            let then = field(ast, k);
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Cond(test, then, e), span });
            e = ast.exprs.len() as ExprId - 1;
        }
        let kind = std::mem::replace(&mut ast.exprs[e as usize].kind, ExprKind::Error);
        ast.exprs[*at as usize].kind = kind;
        changed = true;
    }
    for &(m, at) in &rw.keys {
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Member { obj, name, name_span, optional } = ast.exprs[at as usize].kind else { continue };
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Str(name), span: name_span });
        let index = ast.exprs.len() as ExprId - 1;
        ast.exprs[at as usize].kind = ExprKind::Index { obj, index, optional };
        changed = true;
    }
    for &(m, e, v) in &rw.fn_values {
        let ast = &mut modules[m as usize].ast;
        let span = ast.exprs[e as usize].span;
        let name = std::mem::replace(&mut ast.exprs[e as usize].kind, ExprKind::Error);
        ast.exprs.push(crate::ast::Expr { kind: name, span });
        let callee = ast.exprs.len() as ExprId - 1;
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Ident(v), span });
        let arg = ast.exprs.len() as ExprId - 1;
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Call { callee, type_args: Vec::new(), args: vec![Arg { expr: arg, by_ref: None }], optional: false }, span });
        let call = ast.exprs.len() as ExprId - 1;
        let param = crate::ast::Param { name: v, span, ty: None, inout: false, optional: false, prop: None, rest: false };
        ast.exprs[e as usize].kind = ExprKind::Arrow(Box::new(crate::ast::ArrowFn { params: vec![param], ret: None, throws: None, body: crate::ast::ArrowBody::Expr(call), is_async: false }));
        changed = true;
    }
    for &(m, e, setter) in &rw.setters {
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Assign(op, target, value) = ast.exprs[e as usize].kind else { continue };
        let ExprKind::Member { obj, name_span, .. } = ast.exprs[target as usize].kind else { continue };
        let span = ast.exprs[e as usize].span;
        let value = match op {
            crate::ast::AssignOp::Assign => value,
            // (`x.name += v`: the getter's value, then the setter's call)
            crate::ast::AssignOp::Op(bop) => {
                ast.exprs.push(crate::ast::Expr { kind: ExprKind::Binary(bop, target, value), span });
                ast.exprs.len() as ExprId - 1
            }
        };
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj, name: setter, name_span, optional: false }, span });
        let callee = ast.exprs.len() as ExprId - 1;
        ast.exprs[e as usize].kind = ExprKind::Call { callee, type_args: Vec::new(), args: vec![Arg { expr: value, by_ref: None }], optional: false };
        changed = true;
    }
    for (m, e, names) in &rw.rests {
        let ast = &mut modules[*m as usize].ast;
        let ExprKind::Call { args, .. } = &ast.exprs[*e as usize].kind else { continue };
        // (the pattern's hidden variable)
        let ExprKind::Ident(src) = ast.exprs[args[0].expr as usize].kind else { continue };
        let span = ast.exprs[*e as usize].span;
        let mut fields = Vec::new();
        for &name in names {
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Ident(src), span });
            let obj = ast.exprs.len() as ExprId - 1;
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj, name, name_span: span, optional: false }, span });
            fields.push(crate::ast::ObjField { name, name_span: span, value: ast.exprs.len() as ExprId - 1, quoted: false });
        }
        ast.exprs[*e as usize].kind = ExprKind::Object(fields);
        changed = true;
    }
    for (m, e, names) in &rw.enum_names {
        let ast = &mut modules[*m as usize].ast;
        let span = ast.exprs[*e as usize].span;
        let mut xs = Vec::new();
        for &n in names {
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Str(n), span });
            xs.push(ast.exprs.len() as ExprId - 1);
        }
        ast.exprs[*e as usize].kind = ExprKind::Array(xs);
        changed = true;
    }
    for (m, e, names) in &rw.records {
        let ast = &mut modules[*m as usize].ast;
        let ExprKind::New { args, .. } = &ast.exprs[*e as usize].kind else { continue };
        let ExprKind::Array(entries) = &ast.exprs[args[0].expr as usize].kind else { continue };
        let mut fields = Vec::new();
        for (&entry, &name) in entries.iter().zip(names) {
            let ExprKind::Array(kv) = &ast.exprs[entry as usize].kind else { continue };
            let (k, v) = (kv[0], kv[1]);
            fields.push(crate::ast::ObjField { name, name_span: ast.exprs[k as usize].span, value: v, quoted: false });
        }
        ast.exprs[*e as usize].kind = ExprKind::Object(fields);
        changed = true;
    }
    for (m, cast, src, keys) in &rw.record_casts {
        let ast = &mut modules[*m as usize].ast;
        if !matches!(ast.exprs[*cast as usize].kind, ExprKind::As(..)) {
            continue;
        }
        let span = ast.exprs[*cast as usize].span;
        let mut fields = Vec::new();
        for &(name, absent) in keys {
            let Some(obj) = copy_path(ast, *src) else { continue };
            let key_name = name;
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Str(key_name), span });
            let index = ast.exprs.len() as ExprId - 1;
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Index { obj, index, optional: false }, span });
            let mut value = ast.exprs.len() as ExprId - 1;
            if !absent {
                ast.exprs.push(crate::ast::Expr { kind: ExprKind::NonNull(value), span });
                value = ast.exprs.len() as ExprId - 1;
            } else {
                // (`m["k"] ?? undefined`: read as maybe absent)
                ast.exprs.push(crate::ast::Expr { kind: ExprKind::Undefined, span });
                let u = ast.exprs.len() as ExprId - 1;
                ast.exprs.push(crate::ast::Expr { kind: ExprKind::Binary(crate::ast::BinOp::Nullish, value, u), span });
                value = ast.exprs.len() as ExprId - 1;
            }
            fields.push(crate::ast::ObjField { name, name_span: span, value, quoted: false });
        }
        ast.exprs[*cast as usize].kind = ExprKind::Object(fields);
        changed = true;
    }
    for &(m, x) in &rw.nonnulls {
        let ast = &mut modules[m as usize].ast;
        if matches!(ast.exprs[x as usize].kind, ExprKind::NonNull(_)) {
            continue;
        }
        let span = ast.exprs[x as usize].span;
        let inner = crate::ast::Expr { kind: std::mem::replace(&mut ast.exprs[x as usize].kind, ExprKind::Error), span };
        ast.exprs.push(inner);
        let copy = ast.exprs.len() as ExprId - 1;
        ast.exprs[x as usize].kind = ExprKind::NonNull(copy);
        changed = true;
    }
    for &(m, x, field) in &rw.tuple_places {
        let ast = &mut modules[m as usize].ast;
        if let ExprKind::Index { obj, index, .. } = ast.exprs[x as usize].kind {
            let name_span = ast.exprs[index as usize].span;
            ast.exprs[x as usize].kind = ExprKind::Member { obj, name: field, name_span, optional: false };
            changed = true;
        }
    }
    for &(m, mem, items) in &rw.elem_methods {
        // `m.map(f)` → `m.items.map(f)`
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Member { obj, .. } = ast.exprs[mem as usize].kind else { continue };
        if matches!(ast.exprs[obj as usize].kind, ExprKind::Member { name, .. } if name == items) {
            continue;
        }
        let span = ast.exprs[obj as usize].span;
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj, name: items, name_span: span, optional: false }, span });
        let inner = ast.exprs.len() as ExprId - 1;
        if let ExprKind::Member { obj, .. } = &mut ast.exprs[mem as usize].kind {
            *obj = inner;
        }
        changed = true;
    }
    for &(m, call, f) in &rw.ns_calls {
        // `Object.fromEntries(x)` → `__fromEntries(x)`
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Call { callee, .. } = ast.exprs[call as usize].kind else { continue };
        let ExprKind::Member { name_span, .. } = ast.exprs[callee as usize].kind else { continue };
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Ident(f), span: name_span });
        let fe = ast.exprs.len() as ExprId - 1;
        if let ExprKind::Call { callee, .. } = &mut ast.exprs[call as usize].kind {
            *callee = fe;
        }
        changed = true;
    }
    for &(m, call, levels, flat, slice) in &rw.flat_chains {
        // `xs.flat(2)` → `xs.flat().flat()`
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Call { callee, .. } = ast.exprs[call as usize].kind else { continue };
        let ExprKind::Member { obj, name_span, .. } = ast.exprs[callee as usize].kind else { continue };
        let span = ast.exprs[call as usize].span;
        let mut cur = obj;
        let steps = levels.max(1);
        for k in 0..steps {
            let method = if levels == 0 { slice } else { flat };
            ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj: cur, name: method, name_span, optional: false }, span });
            let mem = ast.exprs.len() as ExprId - 1;
            let kind = ExprKind::Call { callee: mem, type_args: Vec::new(), args: Vec::new(), optional: false };
            if k + 1 == steps {
                ast.exprs[call as usize].kind = kind;
            } else {
                ast.exprs.push(crate::ast::Expr { kind, span });
                cur = ast.exprs.len() as ExprId - 1;
            }
        }
        changed = true;
    }
    for &(m, call, o, k) in &rw.has_own {
        modules[m as usize].ast.exprs[call as usize].kind = ExprKind::Binary(crate::ast::BinOp::In, k, o);
        changed = true;
    }
    for &(m, x) in &rw.truthy {
        // `o => o.note` → `o => !!(o.note)`: the body's node becomes the double negation of a copy
        let ast = &mut modules[m as usize].ast;
        let span = ast.exprs[x as usize].span;
        let inner = crate::ast::Expr { kind: std::mem::replace(&mut ast.exprs[x as usize].kind, ExprKind::Error), span };
        ast.exprs.push(inner);
        let copy = ast.exprs.len() as ExprId - 1;
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Unary(crate::ast::UnOp::Not, copy), span });
        let not = ast.exprs.len() as ExprId - 1;
        ast.exprs[x as usize].kind = ExprKind::Unary(crate::ast::UnOp::Not, not);
        changed = true;
    }
    for &(m, x, method) in &rw.prims {
        // `Number(x)` → `Number(x.valueOf())`: the node becomes the call, on a copy of itself
        let ast = &mut modules[m as usize].ast;
        let span = ast.exprs[x as usize].span;
        let inner = crate::ast::Expr { kind: std::mem::replace(&mut ast.exprs[x as usize].kind, ExprKind::Error), span };
        ast.exprs.push(inner);
        let copy = ast.exprs.len() as ExprId - 1;
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj: copy, name: method, name_span: span, optional: false }, span });
        let callee = ast.exprs.len() as ExprId - 1;
        ast.exprs[x as usize].kind = ExprKind::Call { callee, type_args: Vec::new(), args: Vec::new(), optional: false };
        changed = true;
    }
    for &(m, x) in &rw.awaits {
        // `return p` → `return await p`: the returned expression's node becomes the await of a
        // copy of itself
        let ast = &mut modules[m as usize].ast;
        let inner = crate::ast::Expr { kind: std::mem::replace(&mut ast.exprs[x as usize].kind, ExprKind::Error), span: ast.exprs[x as usize].span };
        ast.exprs.push(inner);
        let copy = ast.exprs.len() as ExprId - 1;
        ast.exprs[x as usize].kind = ExprKind::Await(copy);
        changed = true;
    }
    for &(m, ii) in &rw.items {
        if let ItemKind::Const { mutable, .. } = &mut modules[m as usize].ast.items[ii as usize].kind
            && !*mutable
        {
            *mutable = true;
            changed = true;
        }
    }
    changed
}

/// A copy of the path `x` (`a`, `this.b.c`), for one more read of it.
fn clone_path(ast: &mut crate::ast::Ast, x: ExprId) -> ExprId {
    let span = ast.exprs[x as usize].span;
    let kind = match ast.exprs[x as usize].kind {
        ExprKind::Ident(s) => ExprKind::Ident(s),
        ExprKind::This => ExprKind::This,
        ExprKind::Paren(i) => ExprKind::Paren(clone_path(ast, i)),
        ExprKind::NonNull(i) => ExprKind::NonNull(clone_path(ast, i)),
        ExprKind::Member { obj, name, name_span, optional } => ExprKind::Member { obj: clone_path(ast, obj), name, name_span, optional },
        ExprKind::Index { obj, index, optional } => ExprKind::Index { obj: clone_path(ast, obj), index, optional },
        ExprKind::Str(s) => ExprKind::Str(s),
        _ => return x,
    };
    ast.exprs.push(crate::ast::Expr { kind, span });
    ast.exprs.len() as ExprId - 1
}

/// `x`, `this`, `x.a.b`: an expression that reads the same place however often it's evaluated.
pub(crate) fn simple_path(ast: &crate::ast::Ast, e: ExprId) -> bool {
    match &ast.expr(e).kind {
        ExprKind::Ident(_) | ExprKind::This => true,
        ExprKind::Member { obj, optional: false, .. } => simple_path(ast, *obj),
        _ => false,
    }
}

/// A copy of the `simple_path` `e` (new nodes).
fn copy_path(ast: &mut crate::ast::Ast, e: ExprId) -> Option<ExprId> {
    let span = ast.exprs[e as usize].span;
    let kind = match ast.exprs[e as usize].kind {
        ExprKind::Ident(s) => ExprKind::Ident(s),
        ExprKind::This => ExprKind::This,
        ExprKind::Member { obj, name, name_span, optional: false } => {
            let obj = copy_path(ast, obj)?;
            ExprKind::Member { obj, name, name_span, optional: false }
        }
        _ => return None,
    };
    ast.exprs.push(crate::ast::Expr { kind, span });
    Some(ast.exprs.len() as ExprId - 1)
}

/// Whether `deep_copy` can copy `e`.
pub(crate) fn copyable(ast: &crate::ast::Ast, e: ExprId) -> bool {
    match &ast.expr(e).kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Undefined | ExprKind::Null | ExprKind::Ident(_) | ExprKind::This => true,
        ExprKind::Member { obj, .. } => copyable(ast, *obj),
        ExprKind::Index { obj, index, .. } => copyable(ast, *obj) && copyable(ast, *index),
        ExprKind::Paren(x) | ExprKind::Unary(_, x) | ExprKind::NonNull(x) => copyable(ast, *x),
        ExprKind::Binary(_, a, b) => copyable(ast, *a) && copyable(ast, *b),
        ExprKind::Cond(c, a, b) => copyable(ast, *c) && copyable(ast, *a) && copyable(ast, *b),
        ExprKind::Call { callee, type_args, args, .. } => type_args.is_empty() && copyable(ast, *callee) && args.iter().all(|a| a.by_ref.is_none() && copyable(ast, a.expr)),
        ExprKind::Template(_, xs) | ExprKind::Array(xs) => xs.iter().all(|&x| copyable(ast, x)),
        _ => false,
    }
}

/// (for the parser's desugarings)
pub(crate) fn deep_copy_pub(ast: &mut crate::ast::Ast, e: ExprId) -> Option<ExprId> {
    deep_copy(ast, e)
}

/// A copy of `e` (new nodes all the way down; `copyable` says which it can make).
fn deep_copy(ast: &mut crate::ast::Ast, e: ExprId) -> Option<ExprId> {
    let span = ast.exprs[e as usize].span;
    let kind = match &ast.exprs[e as usize].kind {
        ExprKind::Int(v) => ExprKind::Int(*v),
        ExprKind::Float(v) => ExprKind::Float(*v),
        ExprKind::Str(v) => ExprKind::Str(*v),
        ExprKind::Bool(v) => ExprKind::Bool(*v),
        ExprKind::Undefined => ExprKind::Undefined,
        ExprKind::Null => ExprKind::Null,
        ExprKind::Ident(s) => ExprKind::Ident(*s),
        ExprKind::This => ExprKind::This,
        &ExprKind::Member { obj, name, name_span, optional } => ExprKind::Member { obj: deep_copy(ast, obj)?, name, name_span, optional },
        &ExprKind::Index { obj, index, optional } => {
            let obj = deep_copy(ast, obj)?;
            ExprKind::Index { obj, index: deep_copy(ast, index)?, optional }
        }
        &ExprKind::Paren(x) => ExprKind::Paren(deep_copy(ast, x)?),
        &ExprKind::NonNull(x) => ExprKind::NonNull(deep_copy(ast, x)?),
        &ExprKind::Unary(op, x) => ExprKind::Unary(op, deep_copy(ast, x)?),
        &ExprKind::Binary(op, a, b) => {
            let a = deep_copy(ast, a)?;
            ExprKind::Binary(op, a, deep_copy(ast, b)?)
        }
        &ExprKind::Cond(c, a, b) => {
            let c = deep_copy(ast, c)?;
            let a = deep_copy(ast, a)?;
            ExprKind::Cond(c, a, deep_copy(ast, b)?)
        }
        ExprKind::Call { callee, type_args, args, optional } if type_args.is_empty() => {
            let (callee, args, optional) = (*callee, args.iter().map(|a| a.expr).collect::<Vec<_>>(), *optional);
            let callee = deep_copy(ast, callee)?;
            let mut out = Vec::new();
            for a in args {
                out.push(Arg { expr: deep_copy(ast, a)?, by_ref: None });
            }
            ExprKind::Call { callee, type_args: Vec::new(), args: out, optional }
        }
        ExprKind::Template(parts, xs) => {
            let (parts, xs) = (parts.clone(), xs.clone());
            let mut out = Vec::new();
            for x in xs {
                out.push(deep_copy(ast, x)?);
            }
            ExprKind::Template(parts, out)
        }
        ExprKind::Array(xs) => {
            let xs = xs.clone();
            let mut out = Vec::new();
            for x in xs {
                out.push(deep_copy(ast, x)?);
            }
            ExprKind::Array(out)
        }
        _ => return None,
    };
    ast.exprs.push(crate::ast::Expr { kind, span });
    Some(ast.exprs.len() as ExprId - 1)
}
