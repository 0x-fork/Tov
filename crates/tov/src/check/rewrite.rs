//! TypeScript code that changes a parameter in place: the caller sees the change, as it would
//! share the object in TypeScript. Checking finds such parameters (V0101), arguments for `inout`
//! parameters passed without `&` (V0110) and `const`s passed to them (V0001, V0102); the driver
//! rewrites the program as if it said `inout`, `&` and `let`, and checks it again. A parameter the
//! function reassigns stays as it is (TypeScript would rebind only its own variable), as do
//! parameters of async functions (which can't be `inout`) and of arrow functions.

use super::*;
use crate::ast::{Arg, ExprKind, MemberKind, StmtKind};

/// What a check found to rewrite.
#[derive(Default, Clone, Debug)]
pub struct Rewrites {
    /// Parameters changed in place: (module, the parameter's span start).
    pub(crate) params: Vec<(u32, u32)>,
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
}

impl Rewrites {
    pub fn is_empty(&self) -> bool {
        self.params.is_empty() && self.args.is_empty() && self.consts.is_empty() && self.items.is_empty() && self.awaits.is_empty() && self.calls.is_empty() && self.elems.is_empty() && self.keys.is_empty()
    }

    pub(crate) fn extend(&mut self, o: Rewrites) {
        self.params.extend(o.params);
        self.args.extend(o.args);
        self.consts.extend(o.consts);
        self.items.extend(o.items);
        self.awaits.extend(o.awaits);
        self.calls.extend(o.calls);
        self.elems.extend(o.elems);
        self.keys.extend(o.keys);
    }
}

/// Applies `rw` to the program; true if anything changed.
pub fn apply(modules: &mut [Module], rw: &Rewrites) -> bool {
    let mut changed = false;
    for &(m, start) in &rw.params {
        let ast = &mut modules[m as usize].ast;
        let mut bodies: Vec<(usize, Option<usize>)> = Vec::new();
        for (ii, item) in ast.items.iter().enumerate() {
            match &item.kind {
                ItemKind::Function(_) => bodies.push((ii, None)),
                ItemKind::Class(cd) => {
                    for (mi, mem) in cd.members.iter().enumerate() {
                        if matches!(mem.kind, MemberKind::Method(_) | MemberKind::Constructor(_)) {
                            bodies.push((ii, Some(mi)));
                        }
                    }
                }
                _ => {}
            }
        }
        for (ii, mi) in bodies {
            let f = match (&ast.items[ii].kind, mi) {
                (ItemKind::Function(f), None) => f,
                (ItemKind::Class(cd), Some(mi)) => match &cd.members[mi].kind {
                    MemberKind::Method(f) | MemberKind::Constructor(f) => f,
                    _ => continue,
                },
                _ => continue,
            };
            let Some(pi) = f.params.iter().position(|p| p.span.start == start && !p.inout && !p.rest && p.prop.is_none()) else { continue };
            if f.is_async {
                continue;
            }
            let mut assigned = Vec::new();
            stmt::collect_assigned_stmt(ast, f.body, &mut assigned);
            if assigned.contains(&f.params[pi].name) {
                continue;
            }
            let f = match (&mut ast.items[ii].kind, mi) {
                (ItemKind::Function(f), None) => f,
                (ItemKind::Class(cd), Some(mi)) => match &mut cd.members[mi].kind {
                    MemberKind::Method(f) | MemberKind::Constructor(f) => f,
                    _ => continue,
                },
                _ => continue,
            };
            f.params[pi].inout = true;
            changed = true;
        }
    }
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
    for &(m, at) in &rw.keys {
        let ast = &mut modules[m as usize].ast;
        let ExprKind::Member { obj, name, name_span, optional } = ast.exprs[at as usize].kind else { continue };
        ast.exprs.push(crate::ast::Expr { kind: ExprKind::Str(name), span: name_span });
        let index = ast.exprs.len() as ExprId - 1;
        ast.exprs[at as usize].kind = ExprKind::Index { obj, index, optional };
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
