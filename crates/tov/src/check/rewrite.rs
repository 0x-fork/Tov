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
    /// `m[i]` on a match: (module, the index expression, `items`, assert present), as `m.items[i]` (`!`).
    pub(crate) elems: Vec<(u32, ExprId, crate::intern::Sym, bool)>,
    /// `r.key` on a `Record<string, V>` (module, the member expression), as `r["key"]`.
    pub(crate) keys: Vec<(u32, ExprId)>,
    /// `{ ...a, y: 3 }`: (module, the literal, each spread's fields), as `{ x: a.x, y: 3 }`.
    pub(crate) spreads: Vec<(u32, ExprId, Vec<Vec<crate::intern::Sym>>)>,
    /// `p[k]`, `k` one of these field names: (module, the index), as `k === "a" ? p.a : p.b`.
    pub(crate) keyed: Vec<(u32, ExprId, Vec<crate::intern::Sym>)>,
    /// A class instance where a number is wanted (`Number(x)`, `+x`): (module, the value, the
    /// method), as `x.valueOf()` (JavaScript's conversion calls it).
    pub(crate) prims: Vec<(u32, ExprId, crate::intern::Sym)>,
    /// `{ [Color.Red]: v }` where a record type is expected (every key a string literal): (module,
    /// the `new Map(...)` it was written as, the field names), as `{ red: v }`.
    pub(crate) records: Vec<(u32, ExprId, Vec<crate::intern::Sym>)>,
}

impl Rewrites {
    pub fn is_empty(&self) -> bool {
        self.args.is_empty() && self.prims.is_empty() && self.records.is_empty() && self.consts.is_empty() && self.items.is_empty() && self.awaits.is_empty() && self.calls.is_empty() && self.elems.is_empty() && self.keys.is_empty() && self.spreads.is_empty() && self.keyed.is_empty()
    }

    pub(crate) fn extend(&mut self, o: Rewrites) {
        self.args.extend(o.args);
        self.consts.extend(o.consts);
        self.items.extend(o.items);
        self.awaits.extend(o.awaits);
        self.calls.extend(o.calls);
        self.elems.extend(o.elems);
        self.keys.extend(o.keys);
        self.spreads.extend(o.spreads);
        self.keyed.extend(o.keyed);
        self.prims.extend(o.prims);
        self.records.extend(o.records);
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
    for &(m, at, items, present) in &rw.elems {
        // `m[i]` → `m.items[i]` (`!` when it's read as present)
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Index { obj, .. } = ast.exprs[at as usize].kind else { continue };
        if matches!(ast.exprs[obj as usize].kind, ExprKind::Member { name, .. } if name == items) {
            continue;
        }
        let span = ast.exprs[obj as usize].span;
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj, name: items, name_span: span, optional: false }, span });
        let member = ast.exprs.len() as ExprId - 1;
        let at_span = ast.exprs[at as usize].span;
        if let ExprKind::Index { obj, .. } = &mut ast.exprs[at as usize].kind {
            *obj = member;
        }
        if present {
            let index = std::mem::replace(&mut ast.exprs[at as usize].kind, ExprKind::Error);
            ast.exprs.push(crate::ast::Expr { kind: index, span: at_span });
            let inner = ast.exprs.len() as ExprId - 1;
            ast.exprs[at as usize].kind = ExprKind::NonNull(inner);
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
            let Some(names) = expansions.get(k) else { continue };
            k += 1;
            for &fname in names {
                let obj = clone_path(ast, x);
                let span = ast.exprs[x as usize].span;
                ast.exprs.push(crate::ast::Expr { kind: ExprKind::Member { obj, name: fname, name_span: span, optional: false }, span });
                let read = ast.exprs.len() as ExprId - 1;
                match out.iter_mut().find(|o| o.name == fname) {
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
