//! TypeScript code that changes a parameter in place: the caller sees the change, as it would
//! share the object in TypeScript. Checking finds such parameters (V0101), arguments for `inout`
//! parameters passed without `&` (V0110) and `const`s passed to them (V0001, V0102); the driver
//! rewrites the program as if it said `inout`, `&` and `let`, and checks it again. A parameter the
//! function reassigns stays as it is (TypeScript would rebind only its own variable), as do
//! parameters of async functions (which can't be `inout`) and of arrow functions.

use super::*;
use crate::ast::{ExprKind, MemberKind, StmtKind};

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
}

impl Rewrites {
    pub fn is_empty(&self) -> bool {
        self.params.is_empty() && self.args.is_empty() && self.consts.is_empty() && self.items.is_empty() && self.awaits.is_empty()
    }

    pub(crate) fn extend(&mut self, o: Rewrites) {
        self.params.extend(o.params);
        self.args.extend(o.args);
        self.consts.extend(o.consts);
        self.items.extend(o.items);
        self.awaits.extend(o.awaits);
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
