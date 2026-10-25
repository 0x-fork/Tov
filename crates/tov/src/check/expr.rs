use super::*;
use crate::ast::{Arg, ArrowBody, ArrowFn, AssignOp, BinOp, ExprKind, UnOp};

#[derive(Clone)]
pub(super) struct CallSig {
    pub tparams: Vec<u32>,
    pub params: Vec<FnParam>,
    pub names: Vec<String>,
    pub rest: Option<TyId>,
    pub ret: TyId,
    /// Errors the callee can throw (`NEVER`: none).
    pub throws: TyId,
}

pub(super) struct Place {
    pub ty: TyId,
    /// The place is exactly this local (`x = ...`).
    pub local: Option<LocalId>,
    /// The local the place is rooted in (`x.a[i]` → `x`).
    pub root: Option<LocalId>,
    /// The speculated `number`s its type came from (see `spec`).
    pub sites: Vec<super::spec::Site>,
    /// The place is this module variable: (module, item).
    pub global: Option<(u32, u32)>,
    /// The place is this class field (`spec::field_key`), declared `x = <integer>`.
    pub int_field: Option<(u32, u32)>,
}

pub(super) enum Root {
    Local(LocalId),
    /// A module variable: (module, item).
    ModuleConst(u32, u32),
    Temp,
    /// Inside a class instance: changes the shared object, whatever variable holds it.
    Heap,
}

impl<'a> Checker<'a> {
    /// A decimal literal must fit: 34 significant digits, 34 after the point.
    fn check_decimal_literal(&mut self, text: &str, span: Span) {
        if crate::types::decimal_literal(text).is_none() {
            self.report(Diagnostic::new("T0020", span, format!("{text} doesn't fit in `decimal` (34 significant digits, at most 34 after the point)")));
        }
    }

    /// An integer or float literal (possibly negated or in parentheses).
    pub(super) fn is_num_literal(&self, e: ExprId) -> bool {
        match &self.ast().expr(e).kind {
            ExprKind::Int(_) | ExprKind::Float(_) => true,
            ExprKind::Unary(UnOp::Neg, x) | ExprKind::Paren(x) => self.is_num_literal(*x),
            _ => false,
        }
    }

    pub(super) fn is_int_literal(&self, e: ExprId) -> bool {
        match &self.ast().expr(e).kind {
            ExprKind::Int(_) => true,
            ExprKind::Unary(UnOp::Neg, x) | ExprKind::Paren(x) => self.is_int_literal(*x),
            _ => false,
        }
    }

    fn numeric_hint(&mut self, exp: Option<TyId>) -> Option<TyId> {
        let exp = exp?;
        let members = self.flat_members(exp);
        if members.contains(&INT) {
            return Some(INT);
        }
        if members.contains(&DECIMAL) {
            return Some(DECIMAL);
        }
        members.iter().copied().find(|&m| self.types.is_numeric(m))
    }

    pub(super) fn printable(&self, t: TyId) -> bool {
        // (`never`: code that can't run, `e instanceof Error ? e.message : String(e)`)
        t == NEVER || !matches!(self.types.get(t), Ty::Func(..) | Ty::Namespace(_) | Ty::BuiltinNs(_) | Ty::Expect(_) | Ty::Void | Ty::Never)
    }

    pub(super) fn expr(&mut self, e: ExprId, exp: Option<TyId>) -> TyId {
        let want = std::mem::take(&mut self.want_optional) || exp.is_some_and(|x| self.types.has_undefined(x)) || self.absent_reads.contains(&(self.cur, e));
        // (an untyped empty array used where an array is wanted, `return graph`: those elements)
        if let Some(x) = exp
            && let Ty::Array(el) = *self.types.get(x)
            && !self.types.mentions_error(el)
            && let Some(slot) = self.evolving_slot(e)
        {
            self.fcx().evolving[slot].2.push(el);
        }
        // (an untyped map or set used where one is wanted: those keys and values)
        if let Some(x) = exp
            && matches!(self.types.get(x), Ty::Map(..) | Ty::Set(_))
            && !self.types.mentions_error(x)
            && let Some(slot) = self.evolving_collection(e)
        {
            self.fcx().evolving[slot].2.push(x);
        }
        let mut t = self.expr_inner(e, exp, want);
        // (a local holding an untyped map's value, read where a value must be: `x!`)
        if let Some(x) = exp
            && x != ERROR
            && x != UNKNOWN
            && !self.types.has_undefined(x)
            && self.types.has_undefined(t)
            && let ExprKind::Ident(s) = self.ast().expr(e).kind
            && let Some((id, _)) = self.lookup(s)
            && let Some(f) = self.fcx.last()
            && f.loose.contains(&f.locals[id].span.start)
        {
            self.rewrites.nonnulls.push((self.cur, e));
            t = self.types.without_undefined(t);
        }
        if self.spec.on {
            self.derive_sites(e);
        }
        if let Some(f) = self.facts_mut() {
            f.expr_ty[e as usize] = t;
        }
        if matches!(self.types.get(self.types_without_undef(t)), Ty::Class(..))
            && let Some(f) = self.fcx.last_mut()
        {
            f.class_valued.insert(e);
        }
        t
    }

    /// `want`: the value may be absent here (`xs[i] ?? d`, `xs[i] === undefined`), so an
    /// element read is `T | undefined` rather than a `T` that traps when out of bounds.
    fn expr_inner(&mut self, e: ExprId, exp: Option<TyId>, want: bool) -> TyId {
        let ast = self.ast();
        let node = ast.expr(e);
        let span = node.span;
        match &node.kind {
            ExprKind::Error => ERROR,
            ExprKind::Int(v) => {
                // (a float flows where this literal's record field goes: `f64`, see `lit_key`)
                let mut lit = if self.spec.float_globals.contains(&super::spec::lit_key(self.cur, e)) { F64 } else { INT };
                // (an `int` hint, as `(j % 3) + 1` gives, is no type of its own: a float may yet
                // flow where the sum goes)
                let hint = self.numeric_hint(exp);
                if lit == INT && hint.is_none_or(|h| h == INT) {
                    match self.lit_site(e) {
                        (true, _) => lit = F64,
                        (false, Some(site)) => self.note_sites(span, vec![site]),
                        _ => {}
                    }
                }
                // (one past what an `int` holds is a number, as in TypeScript: `12345678901234567890`)
                let lit = if *v > i64::MAX as u64 && hint.is_none() { F64 } else { lit };
                let t = if lit == F64 && hint == Some(INT) { F64 } else { hint.unwrap_or(lit) };
                if let Some((lo, hi)) = int_range(t)
                    && (*v as i128) > hi {
                        let msg = format!("{v} doesn't fit in `{}` (range {lo}..={hi})", self.show(t));
                        self.report(Diagnostic::new("T0020", span, msg));
                    }
                t
            }
            ExprKind::BigInt(_) => BIGINT,
            ExprKind::Decimal(s) => {
                let text = self.name(*s).to_string();
                self.check_decimal_literal(&text, span);
                DECIMAL
            }
            ExprKind::Float(_) => match self.numeric_hint(exp) {
                Some(F32) => F32,
                Some(DECIMAL) => {
                    let text = self.src(span).to_string();
                    self.check_decimal_literal(&text, span);
                    DECIMAL
                }
                _ => F64,
            },
            ExprKind::Str(s) => self.types.str_lit(*s),
            ExprKind::Template(_, exprs) => {
                for &x in exprs {
                    let t = self.expr(x, None);
                    if !self.printable(t) {
                        let msg = format!("a value of type `{}` can't be interpolated into a string", self.show(t));
                        let s = self.ast().expr(x).span;
                        self.report(Diagnostic::new("T0510", s, msg));
                    }
                }
                STR
            }
            // (`true` where a `true` is expected, as a discriminant: `{ ok: true, value }`)
            ExprKind::Bool(b) => {
                let lit = self.types.bool_lit(*b);
                match exp {
                    Some(x) if x != BOOL && self.flat_members(x).contains(&lit) => lit,
                    _ => BOOL,
                }
            }
            ExprKind::Undefined => UNDEFINED,
            ExprKind::Null => NULL,
            // `xs.map(depth)`, `depth` a generic function of one parameter: as `(v) => depth(v)`,
            // which instantiates it
            ExprKind::Ident(sym)
                if self.lookup(*sym).is_none()
                    && let Some(Decl::Fn(fm, fi)) = self.scopes[self.cur as usize].values.get(sym).map(|d| d.0)
                    && let ItemKind::Function(f) = &self.modules[fm as usize].ast.items[fi as usize].kind
                    && !f.tparams.is_empty()
                    && f.params.len() == 1
                    && !f.params[0].rest
                    && let Some(v) = self.interner.lookup("v") =>
            {
                self.rewrites.fn_values.push((self.cur, e, v));
                return ERROR;
            }
            // `xs.map(Number)`: a built-in function as a value (rewritten as `(v) => Number(v)`)
            ExprKind::Ident(sym)
                if matches!(self.name(*sym), "Number" | "String" | "Boolean" | "parseFloat")
                    && self.lookup(*sym).is_none()
                    && !self.scopes[self.cur as usize].values.contains_key(sym)
                    && let Some(v) = self.interner.lookup("v") =>
            {
                self.rewrites.fn_values.push((self.cur, e, v));
                // (until then: a function of what's expected, returning what the call does)
                let ret = match self.name(*sym) {
                    "String" => STR,
                    "Boolean" => BOOL,
                    _ => F64,
                };
                let ps = exp.and_then(|t| self.flat_members(t).into_iter().find_map(|m| match *self.types.get(m) {
                    Ty::Func(ps, _, _) => Some(self.types.params(ps).to_vec()),
                    _ => None,
                }));
                let ps = ps.unwrap_or_else(|| vec![FnParam { ty: UNKNOWN, inout: false, optional: false }]);
                self.types.func(ps, ret)
            }
            ExprKind::Ident(sym) => {
                // (a class where a constructor type is wanted: the class itself, as a value)
                if let Some(x) = exp
                    && let Some(t) = self.class_value(e, *sym, x, span)
                {
                    return t;
                }
                // (a generic function where a function type is wanted: the instance it fixes)
                if let Some(x) = exp
                    && self.lookup(*sym).is_none()
                    && let Some(Decl::Fn(m, i)) = self.scopes[self.cur as usize].values.get(sym).map(|d| d.0)
                    && let Some(sig) = self.fn_sig(m, i)
                    && !sig.tparams.is_empty()
                {
                    let ft = self.fn_value_type(sig.clone());
                    if let Some(t) = self.generic_value(e, ft, &sig.tparams, x) {
                        self.rec_ident(e, IdentFact::Fn(m, i));
                        return t;
                    }
                }
                self.ident(e, *sym, span)
            }
            ExprKind::Paren(x) => {
                self.want_optional = want;
                // `x satisfies T`
                if let Some(&te) = self.ast().satisfies.get(&e) {
                    let tscope = self.tscope();
                    let want_ty = self.resolve_type(te, &tscope);
                    let t = self.expr(*x, Some(want_ty));
                    let s = self.ast().expr(*x).span;
                    self.expect_assignable(t, want_ty, s, Some("`satisfies`".into()));
                    return t;
                }
                self.expr(*x, exp)
            }
            ExprKind::Await(x) => self.await_expr(e, *x, exp, span),
            ExprKind::Unary(op, x) => self.unary(*op, *x, exp, span),
            ExprKind::Binary(op, l, r) => self.binary(e, *op, *l, *r, exp, span),
            ExprKind::Assign(op, t, v) => {
                // `a ||= b`, `a.f &&= b`: as `a = a || b` (rewritten; the target is a variable or a
                // field path, so reading it again changes nothing)
                if matches!(op, AssignOp::Op(BinOp::And | BinOp::Or)) && super::rewrite::simple_path(self.ast(), *t) {
                    self.expr(*t, None);
                    let vt = self.expr(*v, None);
                    self.rewrites.logical.push((self.cur, e));
                    return vt;
                }
                if let Some(ty) = self.keyed_place(e, *t, Some(*v)) {
                    return ty;
                }
                if *op == AssignOp::Assign
                    && let Some(slot) = self.evolving_let(*t)
                {
                    self.evolve(slot, &[Arg { expr: *v, by_ref: None }]);
                }
                // `xs[i] = x` on an untyped `const xs = []`: its elements are what it's given
                if *op == AssignOp::Assign
                    && let ExprKind::Index { obj, .. } = self.ast().expr(*t).kind
                    && let Some(slot) = self.evolving_slot(obj)
                {
                    self.evolve(slot, &[Arg { expr: *v, by_ref: None }]);
                }
                // `seen[k] = v` on an untyped `const seen = {}`: a map of those keys and values
                if *op == AssignOp::Assign
                    && let ExprKind::Index { obj, index, .. } = self.ast().expr(*t).kind
                    && let Some(slot) = self.evolving_collection(obj)
                {
                    let mark = self.diags.len();
                    let (k, val) = (self.expr(index, None), self.expr(*v, None));
                    self.diags.truncate(mark);
                    let (k, val) = (self.types.widen(k), self.types.widen(val));
                    if !self.types.mentions_error(k) && !self.types.mentions_error(val) {
                        let m = self.types.intern(Ty::Map(k, val));
                        self.fcx().evolving[slot].2.push(m);
                    }
                }
                if let Some(setter) = self.setter_of(*t) {
                    // (rewritten as `x.__set_name(v)`, checked then)
                    self.expr(*t, None);
                    let vt = self.expr(*v, None);
                    self.rewrites.setters.push((self.cur, e, setter));
                    return vt;
                }
                self.assign(*op, *t, *v, span)
            }
            ExprKind::Update { target, .. } => {
                if let Some(t) = self.keyed_place(e, *target, None) {
                    return t;
                }
                let Some(place) = self.place(*target, false) else { return ERROR };
                if !self.types.is_num_like(place.ty) && place.ty != ERROR && place.ty != BIGINT {
                    let msg = format!("`++`/`--` need a number, found `{}`", self.show(place.ty));
                    self.report(Diagnostic::new("T0501", span, msg));
                }
                place.ty
            }
            ExprKind::Call { callee, type_args, args, optional } => {
                let t = self.call(e, *callee, type_args, args, *optional, exp, span);
                // The callee may change class instances through any reference.
                self.invalidate_heap();
                // (and run closures, unless it's a built-in method given none: `xs.at(-1)`)
                let builtin = matches!(self.facts_mut().and_then(|f| f.calls.get(&e)).map(|c| &c.callee), Some(Callee::Method(_)) | Some(Callee::Builtin { .. }));
                let arg_tys: Vec<TyId> = match self.facts_mut() {
                    Some(f) => args.iter().map(|a| f.expr_ty[a.expr as usize]).collect(),
                    None => Vec::new(),
                };
                let given_fn = args.iter().any(|a| matches!(self.ast().expr(a.expr).kind, ExprKind::Arrow(_))) || arg_tys.iter().any(|&t| matches!(self.types.get(t), Ty::Func(..)));
                if !builtin || given_fn {
                    self.forget_closure_narrowings();
                }
                if let Some(f) = self.facts_mut()
                    && let Some(c) = f.calls.get_mut(&e)
                {
                    c.ret = t;
                }
                // (`j = m.get(k)` of an untyped `new Map()`, TypeScript's `Map<any, any>`: the
                // value, checked to be there, as `m.get(k)!`)
                if let Some(x) = exp
                    && !*optional
                    && x != ERROR
                    && x != UNKNOWN
                    && !self.types.has_undefined(x)
                    && self.types.has_undefined(t)
                    && let ExprKind::Member { obj, name, optional: false, .. } = self.ast().expr(*callee).kind
                    && self.name(name) == "get"
                    && self.untyped_map(obj)
                {
                    self.rewrites.nonnulls.push((self.cur, e));
                    return self.types.without_undefined(t);
                }
                t
            }
            ExprKind::New { callee, type_args, args } => {
                let t = self.new_expr(e, *callee, type_args, args, exp, span);
                self.invalidate_heap();
                self.forget_closure_narrowings();
                t
            }
            ExprKind::Member { obj, name, name_span, optional } => {
                self.member_exp = exp;
                let t = self.member(e, *obj, *name, *name_span, *optional, span);
                self.member_exp = None;
                // (`n = g.next().value` of a `Generator<T>`, whose result TypeScript leaves `any`:
                // the value, checked to be there, as `g.next().value!`)
                if let Some(x) = exp
                    && !*optional
                    && self.ts_module()
                    && self.name(*name) == "value"
                    && x != ERROR
                    && x != UNKNOWN
                    && !self.types.has_undefined(x)
                    && self.types.has_undefined(t)
                    && let ExprKind::Call { callee, args, optional: false, .. } = &self.ast().expr(*obj).kind
                    && args.is_empty()
                    && let ExprKind::Member { obj: g, name: nn, .. } = self.ast().expr(*callee).kind
                    && self.name(nn) == "next"
                    && (self.expr_type_peek(g).is_some_and(|gt| matches!(self.types.get(gt), Ty::Gen(_)))
                        // (`m.keys().next().value` of an untyped map's)
                        || matches!(&self.ast().expr(g).kind, ExprKind::Call { callee: kc, .. }
                            if matches!(self.ast().expr(*kc).kind, ExprKind::Member { obj: coll, name: kn, .. } if matches!(self.name(kn), "keys" | "values") && self.untyped_map(coll))))
                {
                    self.rewrites.nonnulls.push((self.cur, e));
                    return self.types.without_undefined(t);
                }
                t
            }
            ExprKind::Index { obj, index, optional } => self.index(e, *obj, *index, *optional, *optional || want, span),
            ExprKind::Object(fields) if fields.iter().any(|f| matches!(self.ast().expr(f.value).kind, ExprKind::Spread(_))) => self.object_spread(e, fields, span),
            ExprKind::Object(fields) => self.object(fields, exp, span),
            ExprKind::Array(elems) => self.array_lit(elems, exp, span),
            ExprKind::Yield(v, delegate) => {
                let generator = self.fcx.last().and_then(|f| f.frames.last()).and_then(|fr| fr.generator);
                let Some(elem) = generator else {
                    self.report(Diagnostic::new("T0306", span, "`yield` is only allowed in a generator (`function*`)"));
                    if let Some(x) = v {
                        self.expr(*x, None);
                    }
                    return ERROR;
                };
                let (v, delegate) = (*v, *delegate);
                let t = match v {
                    // `yield* xs`: each of xs's elements
                    Some(x) if delegate => {
                        let t = self.expr(x, None);
                        match self.iterated(t) {
                            Some(el) => el,
                            None => {
                                if t != ERROR {
                                    let shown = self.show(t);
                                    self.report(Diagnostic::new("T0001", self.ast().expr(x).span, format!("`yield*` takes a generator or an array, found `{shown}`")));
                                }
                                ERROR
                            }
                        }
                    }
                    Some(x) => self.expr(x, elem),
                    None => UNDEFINED,
                };
                match elem {
                    Some(el) => {
                        let s = v.map(|x| self.ast().expr(x).span).unwrap_or(span);
                        self.expect_assignable(t, el, s, Some("the yielded value".into()));
                    }
                    None => {
                        if let Some(fr) = self.fcx.last_mut().and_then(|f| f.frames.last_mut()) {
                            fr.yields.push(t);
                        }
                    }
                }
                UNDEFINED
            }
            ExprKind::Seq(xs) => {
                let mut t = VOID;
                for &x in xs {
                    t = self.expr(x, None);
                }
                t
            }
            ExprKind::Spread(x) => {
                self.report(Diagnostic::new("P0001", span, "`...` spreads only into an array literal or a call's arguments"));
                self.expr(*x, None);
                ERROR
            }
            ExprKind::Arrow(f) => {
                let (t, guard) = self.arrow_guarded(f, exp, span);
                let m = self.cur;
                match guard {
                    Some(g) => self.arrow_guards.insert((m, e), g),
                    None => self.arrow_guards.remove(&(m, e)),
                };
                t
            }
            ExprKind::Cond(c, a, b) => {
                let fx = super::stmt::effects_of_expr(self.ast(), *a).with(super::stmt::effects_of_expr(self.ast(), *b));
                let entry = self.fcx().locals.len();
                let t = self.cond_expr(*c, *a, *b, exp);
                self.apply_effects(&fx, entry, &[]);
                t
            }
            ExprKind::As(x, te) => {
                if let TypeExprKind::Named { name, ns: None, args, .. } = &self.ast().ty(*te).kind
                    && self.name(*name) == "const" && args.is_empty() {
                        // `[[0, false], [1, true]] as const`: rows of tuples
                        if exp.is_none()
                            && let ExprKind::Array(xs) = &self.ast().expr(*x).kind
                            && !xs.is_empty()
                            && xs.iter().all(|&el| matches!(&self.ast().expr(el).kind, ExprKind::Array(r) if (2..=4).contains(&r.len())))
                        {
                            let xs = xs.clone();
                            let rows: Option<Vec<TyId>> = xs.iter().map(|&el| self.const_row(el)).collect();
                            if let Some(rows) = rows {
                                // (one row type: `[1, 2]` and `[0.5, 2]` are both `[number, number]`)
                                let row = rows[1..].iter().fold(rows[0], |a, &b| self.join_numbers(a, b));
                                let t = self.types.array(row);
                                return self.expr(*x, Some(t));
                            }
                        }
                        // `[k, v] as const`: a tuple, as TypeScript's (readonly) one
                        if let ExprKind::Array(xs) = &self.ast().expr(*x).kind
                            && (2..=4).contains(&xs.len())
                            && !xs.iter().any(|&el| matches!(self.ast().expr(el).kind, ExprKind::Spread(_)))
                            && exp.is_none_or(|e| self.tuple_of(e).is_none())
                        {
                            let xs = xs.clone();
                            let tys: Vec<TyId> = xs.iter().map(|&el| self.expr(el, None)).collect();
                            // (of values of one type, `["red", "green"] as const`, an array: it's
                            // used as one; a tuple of a key and a value, `[k, v] as const`)
                            let wide: Vec<TyId> = tys.iter().map(|&t| self.types.widen(t)).collect();
                            let one_type = wide.iter().all(|&t| t == wide[0]) || wide.iter().all(|&t| self.types.is_numeric(t));
                            if !tys.contains(&ERROR) && !one_type {
                                let tuple = self.tuple_ty(&tys);
                                return self.expr(*x, Some(tuple));
                            }
                        }
                        // `["free", "pro", "team"] as const`: its strings keep their literal types
                        // (an element is any of them: `tiers[i]` is `"free" | "pro" | "team"`)
                        if exp.is_none()
                            && let ExprKind::Array(xs) = &self.ast().expr(*x).kind
                            && !xs.is_empty()
                            && xs.iter().all(|&el| matches!(self.ast().expr(el).kind, ExprKind::Str(_)))
                        {
                            let lits: Vec<TyId> = xs
                                .iter()
                                .map(|&el| match self.ast().expr(el).kind {
                                    ExprKind::Str(s) => self.types.intern(Ty::StrLit(s)),
                                    _ => STR,
                                })
                                .collect();
                            let el = self.types.union(&lits);
                            let t = self.types.array(el);
                            return self.expr(*x, Some(t));
                        }
                        // `{ next: "green" } as const`: its strings keep their literal types
                        let object = matches!(self.ast().expr(*x).kind, ExprKind::Object(_));
                        let saved = std::mem::replace(&mut self.as_const, object);
                        let t = self.expr(*x, exp);
                        self.as_const = saved;
                        return t;
                    }
                let tscope = self.tscope();
                let target = self.resolve_type(*te, &tscope);
                // The target guides literal typing (`[] as int[]`), like an annotation would
                // (and is where the value goes: `JSON.parse(t) as number[]` may hold any number).
                let sites = if self.spec.on { self.type_sites(self.cur, *te) } else { Vec::new() };
                let src = self.flow(sites, |c| c.expr(*x, Some(target)));
                if self.spec.on && (src == UNKNOWN || src == JS) {
                    // (a value from outside the program may be any number)
                    let sites = self.type_sites(self.cur, *te);
                    self.blame(&sites);
                    self.spec_poison(target);
                }
                // (`v as number` of a union holding a float: the `number` it's taken as is one too)
                if self.spec.on && target == INT && self.flat_members(src).iter().any(|&m| self.types.is_float(m)) {
                    let sites = self.type_sites(self.cur, *te);
                    self.blame(&sites);
                }
                if src == ERROR || target == ERROR || src == UNKNOWN || self.assignable(src, target) || self.assignable(target, src) {
                    return target;
                }
                // (`out as R` of a type parameter with no bound, or from one: any type may be it,
                // as TypeScript allows; each instance casts its own types)
                let free = |c: &Self, t: TyId| matches!(*c.types.get(t), Ty::Param(p) if c.gparam(p).bound.is_none() && c.gparam(p).ctor.is_none());
                if free(self, target) || free(self, src) {
                    return target;
                }
                // `user as typeof user & { missing?: string }`: the record with the fields it
                // lacks absent (rewritten as `{ ...user } as T`)
                if self.adds_optional_fields(src, target)
                    && let Some(dots) = self.interner.lookup("...")
                {
                    // (once an object literal, it's checked against T: see `object`)
                    if !matches!(self.ast().expr(*x).kind, ExprKind::Object(_)) {
                        self.rewrites.widen_casts.push((self.cur, e, dots));
                    }
                    return target;
                }
                // `m.groups as { value: string; unit?: string }` of a `Record<string, V>`: the
                // record of its keys (rewritten as `{ value: m.groups["value"]!, unit: ... }`; a
                // field that may be absent reads as `undefined`)
                let src_base = self.types.without_undefined(src);
                if let Ty::Map(k, v) = *self.types.get(src_base)
                    && self.types.is_string(k)
                    && super::rewrite::simple_path(self.ast(), *x)
                    && matches!(self.types.get(target), Ty::Record(_))
                    && self.tuple_of(target).is_none()
                {
                    let fields = self.types.fields_in_order(target);
                    let fits = fields.iter().all(|f| {
                        let ft = self.types.without_undefined(f.ty);
                        self.assignable(v, ft)
                    });
                    if fits && !fields.is_empty() {
                        let keys: Vec<(Sym, bool)> = fields.iter().map(|f| (f.name, f.optional || self.types.has_undefined(f.ty))).collect();
                        self.rewrites.record_casts.push((self.cur, e, *x, keys));
                        return target;
                    }
                }
                if src == JS {
                    // a checked conversion from JavaScript (traps if the value doesn't fit)
                    if !self.js_receivable(target) {
                        let ts = self.show(target);
                        self.report(
                            Diagnostic::new("T0903", span, format!("a JavaScript value can't be converted to `{ts}`"))
                                .note("why", "conversions from JavaScript produce numbers, strings, booleans, `undefined`, arrays, records and unions of these"),
                        );
                    }
                    return target;
                }
                let (ss, ts) = (self.show(src), self.show(target));
                let text = self.src(self.ast().expr(*x).span).to_string();
                let mut d = Diagnostic::new("T0601", span, format!("can't cast `{ss}` to `{ts}`"))
                    .note("why", "`as` only narrows (e.g. from `unknown` or a union); it never converts values");
                if target == BIGINT && (self.types.is_numeric(src) || self.types.is_string(src)) {
                    d = d.fix(Applicability::Maybe, format!("convert: `BigInt({text})`"), span, format!("BigInt({text})"));
                } else if src == BIGINT && self.types.is_numeric(target) {
                    let conv = if target == F64 { "Number".to_string() } else { ts.clone() };
                    d = d.fix(Applicability::Maybe, format!("convert: `{conv}({text})`"), span, format!("{conv}({text})"));
                } else if self.types.is_numeric(src) && self.types.is_numeric(target) {
                    if self.types.is_int(target) && self.types.is_float(src) {
                        d = d.fix(Applicability::Maybe, format!("convert: `Math.trunc({text})`"), span, format!("Math.trunc({text})"));
                    } else {
                        d = d.fix(Applicability::Maybe, format!("convert: `{ts}({text})`"), span, format!("{ts}({text})"));
                    }
                } else if target == STR {
                    d = d.fix(Applicability::Maybe, format!("convert: `String({text})`"), span, format!("String({text})"));
                }
                self.report(d);
                target
            }
            ExprKind::NonNull(x) => {
                let hint = exp.map(|t| self.types.optional(t));
                let t = self.expr(*x, hint);
                self.types.without_nullish(t)
            }
            ExprKind::Typeof(x) => {
                // (`typeof seen[k]` of a missing key is "undefined")
                if matches!(self.ast().expr(*x).kind, ExprKind::Index { .. }) {
                    self.want_optional = true;
                }
                self.expr(*x, None);
                STR
            }
            ExprKind::This => self.this_expr(e, span),
            ExprKind::Super => {
                self.report(Diagnostic::new("T0801", span, "`super` is only valid as `super(...)` in a constructor or `super.method(...)`"));
                ERROR
            }
            ExprKind::Try(x) => {
                self.cur_frame().try_expr += 1;
                let t = self.expr(*x, exp);
                self.cur_frame().try_expr -= 1;
                t
            }
        }
    }

    fn cond_expr(&mut self, c: ExprId, a: ExprId, b: ExprId, exp: Option<TyId>) -> TyId {
        let (c, a, b) = (&c, &a, &b);
        self.cond(*c);
        let pos = self.narrowings(*c, true);
        let neg = self.narrowings(*c, false);
        self.push_scope();
        self.apply(&pos);
        let ta = self.expr(*a, exp);
        self.pop_scope();
        // (`c ? { left: l, right: r } : { left: null, right: null }`: one record, each field
        // either branch's)
        if exp.is_none()
            && let Some(m) = self.merged_literals(*a, *b, ta, &neg)
        {
            self.push_scope();
            self.apply(&pos);
            self.expr(*a, Some(m));
            self.pop_scope();
            self.push_scope();
            self.apply(&neg);
            self.expr(*b, Some(m));
            self.pop_scope();
            return m;
        }
        self.push_scope();
        self.apply(&neg);
        let tb = self.expr(*b, exp.or(Some(ta)));
        self.pop_scope();
        // (an int and a float branch: the float, as `int | f64` isn't a value Tov has)
        if self.types.is_int(ta) && self.types.is_float(tb) || self.types.is_float(ta) && self.types.is_int(tb) {
            return F64;
        }
        if let Some(x) = exp
            && self.assignable(ta, x) && self.assignable(tb, x) {
                return self.types.union(&[ta, tb]);
            }
        self.types.union(&[ta, tb])
    }

    /// Two object literals with the same keys whose fields don't all agree (`ta` the first's
    /// type): the record of both, each field the union of the two (checked quietly).
    fn merged_literals(&mut self, a: ExprId, b: ExprId, ta: TyId, neg: &[Narrowing]) -> Option<TyId> {
        let lit = |c: &Self, e: ExprId| {
            let mut e = e;
            while let ExprKind::Paren(x) = c.ast().expr(e).kind {
                e = x;
            }
            matches!(&c.ast().expr(e).kind, ExprKind::Object(fs) if !fs.is_empty())
        };
        if !lit(self, a) || !lit(self, b) || self.tuple_of(ta).is_some() {
            return None;
        }
        let Ty::Record(fa) = *self.types.get(ta) else { return None };
        let saved = std::mem::take(&mut self.diags);
        self.push_scope();
        self.apply(neg);
        let tb = self.expr(b, None);
        self.pop_scope();
        let failed = !self.diags.is_empty();
        self.diags = saved;
        let Ty::Record(fb) = *self.types.get(tb) else { return None };
        if failed || ta == tb {
            return None;
        }
        let (fa, fb) = (self.types.fields(fa).to_vec(), self.types.fields(fb).to_vec());
        if fa.len() != fb.len() || fa.iter().zip(&fb).any(|(x, y)| x.name != y.name || x.optional != y.optional) {
            return None;
        }
        let order = self.types.fields_in_order(ta).iter().map(|f| f.name).collect::<Vec<_>>();
        let mut fields = Vec::new();
        for (x, y) in fa.iter().zip(&fb) {
            let ty = match (x.ty, y.ty) {
                (s, t) if s == t => s,
                (NEVER, t) | (t, NEVER) => t,
                (s, t) if self.types.is_int(s) && self.types.is_float(t) || self.types.is_float(s) && self.types.is_int(t) => F64,
                (s, t) => {
                    let (s, t) = (self.types.widen(s), self.types.widen(t));
                    self.types.union(&[s, t])
                }
            };
            fields.push(Field { name: x.name, ty, optional: x.optional });
        }
        let t = self.types.record(fields);
        self.types.note_field_order(t, order);
        Some(t)
    }

    /// A call of function `(m, i)` with overload signatures: the result type of the first that
    /// takes the arguments (as checked against the implementation), if it's one of its results.
    fn overload_ret(&mut self, m: u32, i: u32, args: &[Arg], t: TyId) -> TyId {
        let ItemKind::Function(f) = &self.modules[m as usize].ast.items[i as usize].kind else { return t };
        let name = f.name;
        if t == ERROR || !self.modules[m as usize].ast.overloads.iter().any(|o| o.0 == name) {
            return t;
        }
        let cur = self.cur as usize;
        let Some(arg_tys) = self.facts.as_ref().map(|fs| args.iter().map(|a| fs[cur].expr_ty.get(a.expr as usize).copied().unwrap_or(ERROR)).collect::<Vec<TyId>>()) else { return t };
        let n = self.modules[m as usize].ast.overloads.len();
        for k in 0..n {
            let (oname, generic, nparams) = {
                let o = &self.modules[m as usize].ast.overloads[k];
                (o.0, !o.1.is_empty(), o.2.len())
            };
            if oname != name || generic || args.len() > nparams {
                continue;
            }
            let (params, ret) = self.with_module(m, |c| {
                let ast = &c.modules[m as usize].ast;
                let ps: Vec<(Option<crate::ast::TypeId>, bool)> = ast.overloads[k].2.iter().map(|p| (p.ty, p.optional)).collect();
                let r = ast.overloads[k].3;
                let params: Vec<(TyId, bool)> = ps.into_iter().map(|(te, opt)| (te.map(|te| c.resolve_type(te, &[])).unwrap_or(ERROR), opt)).collect();
                (params, r.map(|te| c.resolve_type(te, &[])))
            });
            let required = params.iter().filter(|p| !p.1).count();
            if args.len() < required || params.iter().any(|p| p.0 == ERROR) {
                continue;
            }
            if arg_tys.iter().zip(&params).all(|(&a, p)| a != ERROR && self.assignable(a, p.0)) {
                // (as the implementation's result has it: the member the overload's is)
                return match ret {
                    Some(r) if r != t && self.assignable(r, t) => {
                        let ms = self.flat_members(t);
                        ms.iter().copied().find(|&m| m == r).or_else(|| ms.iter().copied().find(|&m| self.assignable(r, m))).unwrap_or(r)
                    }
                    _ => t,
                };
            }
        }
        t
    }

    /// Class `sym` given where `want` (a constructor type's parameter, or the class it was bound
    /// to) is wanted: checked against the constructor type, and the class itself.
    fn class_value(&mut self, e: ExprId, sym: Sym, want: TyId, span: Span) -> Option<TyId> {
        if self.lookup(sym).is_some() {
            return None;
        }
        let Some(Decl::Class(c)) = self.scopes[self.cur as usize].values.get(&sym).map(|d| d.0) else { return None };
        let sig = match *self.types.get(want) {
            Ty::Param(p) => self.gparam(p).ctor.clone()?,
            Ty::ClassObj(_) => {
                self.rec_ident(e, IdentFact::Class(c));
                return Some(self.types.intern(Ty::ClassObj(c)));
            }
            _ => return None,
        };
        self.rec_ident(e, IdentFact::Class(c));
        self.resolve_class(c);
        let info = self.classes[c as usize].clone();
        let n = self.name(sym).to_string();
        let shown = {
            let tps: Vec<String> = sig.tparams.iter().map(|&p| self.name(self.gparam(p).name).to_string()).collect();
            let ps: Vec<String> = sig.params.iter().map(|p| self.show(p.ty)).collect();
            let tps = if tps.is_empty() { String::new() } else { format!("<{}>", tps.join(", ")) };
            format!("new {tps}({}) => {}", ps.join(", "), self.show(sig.ret))
        };
        let mut problems = Vec::new();
        if info.params.len() != sig.tparams.len() {
            problems.push(format!("it takes {} type parameter(s), the constructor type {}", info.params.len(), sig.tparams.len()));
        } else {
            // (its instances, with the constructor type's own type parameters for its own)
            let map: HashMap<u32, TyId> = info.params.iter().zip(&sig.tparams).map(|(&cp, &sp)| (cp, self.types.intern(Ty::Param(sp)))).collect();
            let inst = self.types.subst(info.this_ty, &map);
            if !self.assignable(inst, sig.ret) {
                problems.push(format!("its instances are `{}`, not `{}`", self.show(inst), self.show(sig.ret)));
            }
            let required = info.ctor.params.iter().filter(|p| !p.optional).count();
            if required > sig.params.len() {
                problems.push(format!("its constructor needs {required} argument(s), the constructor type gives {}", sig.params.len()));
            }
            for (i, (cp, sp)) in info.ctor.params.iter().zip(&sig.params).enumerate() {
                let cpt = self.types.subst(cp.ty, &map);
                if !self.assignable(sp.ty, cpt) {
                    problems.push(format!("argument {} is `{}`, its constructor takes `{}`", i + 1, self.show(sp.ty), self.show(cpt)));
                }
            }
        }
        if info.is_abstract {
            problems.push("it's abstract".to_string());
        }
        if !problems.is_empty() {
            self.report(Diagnostic::new("T0803", span, format!("class `{n}` doesn't fit `{shown}`")).note("problems", problems.join("; ")));
            return Some(ERROR);
        }
        Some(self.types.intern(Ty::ClassObj(c)))
    }

    fn ident(&mut self, e: ExprId, sym: Sym, span: Span) -> TyId {
        if let Some((id, ty)) = self.lookup(sym) {
            if self.spec.on {
                let sites = match self.local(id).global {
                    Some((m, i)) => self.const_sites(m, i),
                    None => self.local(id).sites.clone(),
                };
                self.note_sites(span, sites);
            }
            // (a script-local standing in for a top-level `const`/`let`: still a global read)
            if let Some((m, i)) = self.local(id).global {
                self.rec_ident(e, IdentFact::Const(m, i));
            } else {
                let key = self.local(id).span.start;
                self.rec_ident(e, IdentFact::Local(key));
            }
            return ty;
        }
        match self.scopes[self.cur as usize].values.get(&sym).map(|d| d.0) {
            Some(Decl::Fn(m, i)) => self.rec_ident(e, IdentFact::Fn(m, i)),
            Some(Decl::Const(m, i)) => self.rec_ident(e, IdentFact::Const(m, i)),
            Some(Decl::Ns(m)) => self.rec_ident(e, IdentFact::Ns(m)),
            Some(Decl::Class(c)) => self.rec_ident(e, IdentFact::Class(c)),
            Some(Decl::Npm(m, n)) => {
                self.rec_ident(e, IdentFact::Npm(m, n));
                return JS;
            }
            _ => self.rec_ident(e, IdentFact::Builtin),
        }
        match self.scopes[self.cur as usize].values.get(&sym).map(|d| d.0) {
            Some(Decl::Fn(m, i)) => {
                let Some(sig) = self.fn_sig(m, i) else {
                    let n = self.name(sym).to_string();
                    self.report(Diagnostic::new("T0303", span, format!("`{n}` is recursive, so it needs an explicit return type")));
                    return ERROR;
                };
                if !sig.tparams.is_empty() {
                    let n = self.name(sym).to_string();
                    self.report(
                        Diagnostic::new("T0304", span, format!("generic function `{n}` can't be used as a value"))
                            .note("instead", format!("wrap it in an arrow: `(x) => {n}(x)`")),
                    );
                    return ERROR;
                }
                return self.fn_value_type(sig);
            }
            Some(Decl::Const(m, i)) => {
                if self.spec.on {
                    let sites = self.const_sites(m, i);
                    self.note_sites(span, sites);
                }
                if let Some(t) = self.lookup_global((m, i)) {
                    return t;
                }
                return self.const_type(m, i);
            }
            Some(Decl::Ns(m)) => return self.types.intern(Ty::Namespace(m)),
            Some(Decl::Class(_)) => {
                let n = self.name(sym).to_string();
                self.report(
                    Diagnostic::new("T0803", span, format!("class `{n}` is not a value"))
                        .note("instead", format!("create an instance with `new {n}(...)`, or use its static members (`{n}.name`)")),
                );
                return ERROR;
            }
            _ => {}
        }
        let text = self.name(sym).to_string();
        if text == "NaN" || text == "Infinity" {
            return F64;
        }
        if self.is_ns(&text) {
            return self.types.intern(Ty::BuiltinNs(sym));
        }
        if let Some(msg) = builtins::removed_global(&text) {
            self.report(Diagnostic::new("X0032", span, msg));
            return ERROR;
        }
        if builtins::is_builtin_fn(&text) {
            self.report(Diagnostic::new("T0203", span, format!("built-in `{text}` must be called")));
            return ERROR;
        }
        if self.scopes[self.cur as usize].types.contains_key(&sym) {
            self.report(Diagnostic::new("N0001", span, format!("`{text}` is a type, not a value")));
            return ERROR;
        }
        let mut candidates: Vec<String> = Vec::new();
        if let Some(fcx) = self.fcx.last() {
            for scope in &fcx.scopes {
                candidates.extend(scope.iter().filter(|(s, _, _)| *s != super::PATH_SYM && *s != super::THIS_SYM && *s != super::GLOBAL_SYM).map(|(s, _, _)| self.name(*s).to_string()));
            }
        }
        candidates.extend(self.scopes[self.cur as usize].values.keys().map(|&s| self.name(s).to_string()));
        candidates.extend(builtins::GLOBAL_NAMES.iter().map(|s| s.to_string()));
        let sug = similar(&text, candidates.iter().map(|s| s.as_str()));
        let mut d = Diagnostic::new("N0001", span, format!("unknown name `{text}`"));
        if let Some(first) = sug.first() {
            d = d.note("did you mean", sug.join(", ")).fix(Applicability::Maybe, format!("use `{first}`"), span, first.to_string());
        }
        self.report(d);
        ERROR
    }

    fn unary(&mut self, op: UnOp, x: ExprId, exp: Option<TyId>, span: Span) -> TyId {
        match op {
            UnOp::Neg => {
                // (`-0` in TypeScript is negative zero, a float)
                if matches!(self.ast().expr(x).kind, ExprKind::Int(0)) && matches!(self.modules[self.cur as usize].path.extension().and_then(|e| e.to_str()), Some("ts" | "tsx" | "mts" | "cts")) {
                    self.expr(x, Some(F64));
                    return F64;
                }
                let t = self.expr(x, exp);
                if t != ERROR && !self.types.is_num_like(t) && t != BIGINT && self.to_primitive(x, t) {
                    return F64;
                }
                if t != ERROR && !self.types.is_num_like(t) && t != BIGINT {
                    let msg = format!("unary `-` needs a number, found `{}`", self.show(t));
                    self.report(Diagnostic::new("T0501", span, msg));
                    return ERROR;
                }
                if self.ts_module() && promoted(t) == INT && self.types.is_int(t) {
                    return INT;
                }
                if matches!(self.types.get(t), Ty::U8 | Ty::U8C | Ty::U16 | Ty::U32 | Ty::U64) && !self.is_int_literal(x) {
                    let msg = format!("unary `-` on unsigned `{}`", self.show(t));
                    self.report(Diagnostic::new("T0501", span, msg));
                }
                t
            }
            // `+x`: a number as it is, a string as `Number(s)` reads it (`NaN` if it isn't one),
            // a boolean as 0 or 1
            UnOp::Plus => {
                let t = self.expr(x, None);
                if t == ERROR || self.types.is_num_like(t) {
                    return t;
                }
                if self.types.is_string(t) {
                    return F64;
                }
                if t == BOOL {
                    return INT;
                }
                if self.to_primitive(x, t) {
                    return F64;
                }
                let shown = self.show(t);
                self.report(Diagnostic::new("X0028", span, format!("unary `+` needs a number, a string or a boolean, found `{shown}`")));
                ERROR
            }
            UnOp::Not => {
                self.cond(x);
                BOOL
            }
            UnOp::BitNot => {
                let t = self.expr(x, exp.filter(|&e| e == BIGINT || self.types.is_int(e)));
                if self.types.is_float(t) {
                    return INT; // ToInt32, as in JavaScript
                }
                if t != ERROR && !self.types.is_int(t) && t != BIGINT {
                    let msg = format!("`~` needs an integer, found `{}`", self.show(t));
                    self.report(Diagnostic::new("T0501", span, msg));
                    return ERROR;
                }
                t
            }
            UnOp::Void => {
                self.expr(x, None);
                self.report(Diagnostic::new("X0029", span, "the `void` operator is not supported"));
                UNDEFINED
            }
        }
    }


    fn binary(&mut self, e: ExprId, op: BinOp, l: ExprId, r: ExprId, exp: Option<TyId>, span: Span) -> TyId {
        use BinOp::*;
        match op {
            And | Or => {
                // JavaScript's: operands by their truthiness; a value unless both are `bool` or
                // it's a condition (`if (a || b)`)
                let want_bool = exp == Some(BOOL);
                let hint = if want_bool { Some(BOOL) } else { exp };
                // (`counts[k] || 0`: a missing key is `undefined`, so the default)
                if (op == Or || want_bool) && matches!(self.ast().expr(l).kind, ExprKind::Index { .. }) {
                    self.want_optional = true;
                }
                let tl = self.expr(l, hint);
                let n = self.narrowings(l, op == And);
                let fx = super::stmt::effects_of_expr(self.ast(), r);
                let entry = self.fcx().locals.len();
                self.push_scope();
                self.apply(&n);
                // (`if (xs[i] || xs[j])`: either may be missing)
                if want_bool && matches!(self.ast().expr(r).kind, ExprKind::Index { .. }) {
                    self.want_optional = true;
                }
                let tr = self.expr(r, if tl == BOOL && hint.is_none() { Some(BOOL) } else { hint });
                self.pop_scope();
                self.apply_effects(&fx, entry, &[]);
                if want_bool || (tl == BOOL && tr == BOOL) || tl == ERROR || tr == ERROR {
                    return BOOL;
                }
                let kept = if op == Or { self.truthy_part(tl) } else { self.falsy_part(tl) };
                match kept {
                    NEVER => tr,
                    k if self.types.is_int(k) && self.types.is_float(tr) || self.types.is_float(k) && self.types.is_int(tr) => F64,
                    k => {
                        let w = self.types.widen(tr);
                        self.types.union(&[k, w])
                    }
                }
            }
            Nullish => {
                let hint = exp.map(|t| self.types.optional(t));
                self.want_optional = true;
                let tl = self.expr(l, hint);
                let inner = self.types.without_nullish(tl);
                let tr = self.expr(r, exp.or(Some(inner)));
                if self.types.is_int(inner) && self.types.is_float(tr) || self.types.is_float(inner) && self.types.is_int(tr) {
                    return F64;
                }
                let keeps_literals = exp.map(|x| self.flat_members(x).iter().any(|&m| matches!(self.types.get(m), Ty::StrLit(_)))).unwrap_or(false)
                    || matches!(self.types.get(inner), Ty::StrLit(_));
                let tr = if keeps_literals { tr } else { self.types.widen(tr) };
                self.types.union(&[inner, tr])
            }
            // (`==` as `===`: TypeScript already rejects comparing values of different types, and
            // `x == null` tests for `undefined`)
            Eq | Ne | LooseEq | LooseNe => self.equality(e, l, r, span),
            Lt | Gt | Le | Ge => {
                let (tl, tr) = self.numeric_pair(l, r, None);
                let (tl, tr) = self.primitive_pair(l, r, tl, tr);
                if tl == BIGINT || tr == BIGINT {
                    // bigint with bigint, or with a number (compared exactly, as in JavaScript)
                    let ok = |t: TyId| t == BIGINT || t == ERROR || self.types.is_int(t) || self.types.is_float(t);
                    if !ok(tl) || !ok(tr) {
                        self.bigint_mix(op, l, r, tl, tr, span);
                    }
                    return BOOL;
                }
                if tl == DECIMAL || tr == DECIMAL {
                    self.decimal_arith(op, l, r, tl, tr, span);
                    return BOOL;
                }
                // (a type parameter with itself, as TypeScript allows: each use compares what it
                // stands for, as JavaScript does)
                let ok = (self.types.is_numeric(tl) && self.types.is_numeric(tr))
                    || (self.types.is_string(tl) && self.types.is_string(tr))
                    || (tl == tr && matches!(self.types.get(tl), Ty::Param(_)));
                if !ok && tl != ERROR && tr != ERROR {
                    let msg = format!("can't compare `{}` with `{}` using `{}`", self.show(tl), self.show(tr), op.as_str());
                    let mut d = Diagnostic::new("T0501", span, msg);
                    if self.types.has_undefined(tl) || self.types.has_undefined(tr) {
                        d = d.note("why", "one side may be `undefined`; narrow it first");
                    }
                    self.report(d);
                } else if ok && self.types.is_int(tl) && self.types.is_int(tr) && tl != tr && !(self.ts_module() && promoted(tl) == promoted(tr)) {
                    self.mixed_ints(tl, tr, span);
                }
                BOOL
            }
            Add => {
                let (tl, tr) = self.numeric_pair(l, r, exp);
                if self.types.is_stringish(tl) || self.types.is_stringish(tr) {
                    for (t, x) in [(tl, l), (tr, r)] {
                        if !self.printable(t) {
                            let msg = format!("can't concatenate `{}` to a string", self.show(t));
                            let s = self.ast().expr(x).span;
                            self.report(Diagnostic::new("T0501", s, msg));
                        }
                    }
                    return STR;
                }
                if tl == DECIMAL || tr == DECIMAL {
                    return self.decimal_arith(op, l, r, tl, tr, span);
                }
                if tl == BIGINT || tr == BIGINT {
                    return self.bigint_arith(op, l, r, tl, tr, span);
                }
                self.arith(op, tl, tr, span)
            }
            Sub | Mul | Rem | Pow => {
                let (tl, tr) = self.numeric_pair(l, r, exp);
                let (tl, tr) = self.primitive_pair(l, r, tl, tr);
                if tl == DECIMAL || tr == DECIMAL {
                    return self.decimal_arith(op, l, r, tl, tr, span);
                }
                if tl == BIGINT || tr == BIGINT {
                    return self.bigint_arith(op, l, r, tl, tr, span);
                }
                // (`2 ** 70`, `2 ** -1`, `10 ** e` in TypeScript: a number that may be no safe
                // integer, a float; `x ** 2` stays an integer)
                if op == Pow && tl == INT && tr == INT && self.ts_module() {
                    let fits = match (self.const_int(l), self.const_int(r)) {
                        (_, None) => false,
                        (_, Some(b)) if b < 0 => false,
                        (Some(a), Some(b)) => (0..=u32::MAX as i128).contains(&b).then(|| a.checked_pow(b as u32)).flatten().is_some_and(|v| v.abs() <= 1 << 53),
                        (None, Some(_)) => true,
                    };
                    if !fits {
                        return F64;
                    }
                }
                self.arith(op, tl, tr, span)
            }
            Div => {
                let (tl, tr) = self.numeric_pair(l, r, None);
                let (tl, tr) = self.primitive_pair(l, r, tl, tr);
                if tl == DECIMAL || tr == DECIMAL {
                    return self.decimal_arith(op, l, r, tl, tr, span);
                }
                if tl == BIGINT || tr == BIGINT {
                    return self.bigint_arith(op, l, r, tl, tr, span);
                }
                let t = self.arith(op, tl, tr, span);
                if t == ERROR {
                    ERROR
                } else if tl == F32 && tr == F32 {
                    F32
                } else {
                    F64
                }
            }
            BitAnd | BitOr | BitXor | Shl | Shr | UShr => {
                let (tl, tr) = self.numeric_pair(l, r, exp);
                if tl == ERROR || tr == ERROR {
                    return ERROR;
                }
                if tl == BIGINT || tr == BIGINT {
                    if op == UShr {
                        self.report(Diagnostic::new("T0501", span, "`>>>` doesn't apply to `bigint`: a bigint has no fixed width").note("instead", "`>>` (it rounds toward negative infinity), or `BigInt.asUintN(bits, x) >> n`"));
                        return ERROR;
                    }
                    return self.bigint_arith(op, l, r, tl, tr, span);
                }
                // On numbers (`int` and floats) the operators are JavaScript's: 32-bit, giving an `int`
                // (a typed array's elements too, in TypeScript)
                let (tl, tr) = if self.ts_module() { (promoted(tl), promoted(tr)) } else { (tl, tr) };
                let js = |t: TyId| t == INT || self.types.is_float(t);
                if js(tl) && js(tr) {
                    return INT;
                }
                if !self.types.is_int(tl) || !self.types.is_int(tr) {
                    let msg = format!("`{}` needs integers, found `{}` and `{}`", op.as_str(), self.show(tl), self.show(tr));
                    self.report(Diagnostic::new("T0501", span, msg).note("instead", "convert floats first with `Math.trunc(x)`"));
                    return ERROR;
                }
                if tl != tr && !matches!(op, Shl | Shr | UShr) {
                    self.mixed_ints(tl, tr, span);
                }
                tl
            }
            // `"radius" in shape`: whether it has that key (by its type; a map's keys, an
            // `unknown`'s, an optional field's presence at run time)
            In => {
                let tl = self.expr(l, Some(STR));
                let tr = self.expr(r, None);
                // (a computed key on a record or a class: its names are fixed, so known only by name)
                let lit = matches!(self.ast().expr(l).kind, ExprKind::Str(_));
                if !lit && self.flat_members(tr).iter().any(|&m| matches!(self.types.get(m), Ty::Record(_) | Ty::Interface(..) | Ty::Class(..) | Ty::Rec(..))) {
                    self.report(Diagnostic::new("X0025", span, "`in` on a record or a class needs a literal key (`\"name\" in x`)").note("instead", "for dynamic keys use a `Map` or `Record<string, V>`: `map.has(key)`"));
                }
                if tl != ERROR && !self.types.is_string(tl) {
                    let shown = self.show(tl);
                    self.report(Diagnostic::new("X0025", span, format!("`in` takes a string key, found `{shown}`")).note("instead", "for an array's index, compare it with the length"));
                }
                BOOL
            }
            Instanceof => {
                let tl = self.expr(l, None);
                let Some(c) = self.instanceof_class(r) else { return BOOL };
                let members = self.flat_members(tl);
                let possible = tl == ERROR || tl == UNKNOWN || members.iter().any(|&m| match self.class_of(m) {
                    Some((mc, _)) => self.class_descends(mc, c) || self.class_descends(c, mc),
                    None => false,
                });
                if !possible {
                    let (ls, cs) = (self.show(tl), self.class_names[c as usize].clone());
                    self.report(Diagnostic::new("T0502", span, format!("this check is always false: `{ls}` is never a `{cs}`")));
                }
                BOOL
            }
        }
    }

    /// `x` (of type `t`) is a class instance with `valueOf()`: where a number is wanted, it's
    /// read through that, as JavaScript converts it (rewritten as `x.valueOf()`).
    /// Operands of `<`, `-`, `*`, ... that are objects with a `valueOf()` (a `Date`): its value,
    /// as JavaScript compares and computes with them.
    fn primitive_pair(&mut self, l: ExprId, r: ExprId, tl: TyId, tr: TyId) -> (TyId, TyId) {
        let tl = if self.class_of(tl).is_some() && self.to_primitive(l, tl) { F64 } else { tl };
        let tr = if self.class_of(tr).is_some() && self.to_primitive(r, tr) { F64 } else { tr };
        (tl, tr)
    }

    pub(super) fn to_primitive(&mut self, x: ExprId, t: TyId) -> bool {
        let Some(sym) = self.interner.lookup("valueOf") else { return false };
        if self.class_of(t).is_none() || !matches!(self.class_member(t, sym), Some(super::class::ClassMemberRef::Method(ref mm)) if mm.params.iter().all(|p| p.optional)) {
            return false;
        }
        self.rewrites.prims.push((self.cur, x, sym));
        true
    }

    pub(super) fn cur_frame(&mut self) -> &mut Frame {
        self.fcx().frames.last_mut().expect("in a function")
    }

    /// An error of type `t` leaves the current expression: caught by an enclosing `try` block,
    /// or passed on (a call must then be marked `try`). `callee`: the call's description.
    pub(super) fn on_throw(&mut self, t: TyId, span: Span, callee: Option<&str>, call: Option<ExprId>) {
        let cur = self.cur as usize;
        let Some(frame) = self.fcx.last_mut().and_then(|f| f.frames.last_mut()) else { return };
        if let Some(tf) = frame.try_frames.last_mut() {
            tf.push(t);
            if let (Some(c), Some(f)) = (call, self.facts.as_mut()) {
                f[cur].throwing.insert(c);
            }
            return;
        }
        let (can_throw, decl, module_init) = (frame.can_throw, frame.throws_decl, frame.module_init);
        let shown = self.show(t);
        if !can_throw && module_init {
            let what = callee.map(|c| format!("`{c}` can throw `{shown}`")).unwrap_or_else(|| format!("this throws `{shown}`"));
            self.report(
                Diagnostic::new("T0832", span, format!("{what}, but a module constant's initializer can't pass errors on"))
                    .note("instead", "initialize it in a function that can throw, or catch the error in a helper function"),
            );
            return;
        }
        if !can_throw {
            if let Some(t) = self.fcx.last().and_then(|f| f.frames.last()).and_then(|f| f.exp_fn) {
                self.blame_fn_type(t);
            }
            let what = callee.map(|c| format!("`{c}` can throw `{shown}`")).unwrap_or_else(|| format!("this throws `{shown}`"));
            self.report(
                Diagnostic::new("T0832", span, format!("{what}, but this closure's type doesn't let it pass errors on"))
                    .note("instead", "catch it inside the closure: `try { ... } catch (e) { ... }`")
                    .note("or", format!("let the function type throw: `(...) => T throws {shown}`")),
            );
            return;
        }
        if let Some(d) = decl
            && !self.assignable(t, d)
        {
            let ds = self.show(d);
            self.report(
                Diagnostic::new("T0833", span, format!("this can throw `{shown}`, but the function declares `throws {ds}`"))
                    .note("instead", format!("widen the declaration (`throws {ds} | {shown}`), or catch it here")),
            );
        }
        if let Some(c) = call
            && let Some(f) = self.facts.as_mut()
        {
            f[cur].throwing.insert(c);
        }
        self.cur_frame().thrown.push(t);
    }

    /// Checks that a `throws` type is made of `Error` classes.
    pub(crate) fn check_throws_type(&mut self, t: TyId, span: Span) {
        let err = self.error_class();
        if err != ERROR && t != ERROR && !self.assignable(t, err) {
            let shown = self.show(t);
            self.report(Diagnostic::new("T0834", span, format!("`throws {shown}`: errors must be classes that extend `Error`")).note("example", "`class NotFound extends Error {}`, then `throws NotFound`"));
        }
    }

    /// A built-in namespace (`Math`, `process`, ...; `__native` only in standard-library modules).
    pub(super) fn is_ns(&self, text: &str) -> bool {
        builtins::is_builtin_ns(text) || (text == "__native" && self.modules[self.cur as usize].std)
    }

    /// A class declared by the built-in prelude (`Error`, `SyntaxError`, ...).
    pub(super) fn builtin_class(&mut self, name: &str) -> TyId {
        let b = self.modules.iter().position(|m| m.builtin);
        let sym = self.interner.lookup(name);
        match (b, sym) {
            (Some(b), Some(sym)) => match self.scopes[b].types.get(&sym).map(|d| d.0) {
                Some(Decl::Class(c)) => {
                    self.resolve_class(c);
                    self.classes[c as usize].this_ty
                }
                _ => ERROR,
            },
            _ => ERROR,
        }
    }

    /// The class named on the right of `instanceof`.
    fn instanceof_class(&mut self, r: ExprId) -> Option<u32> {
        if let ExprKind::Ident(s) = self.ast().expr(r).kind
            && self.lookup(s).is_none()
            && let Some(Decl::Class(c)) = self.scopes[self.cur as usize].values.get(&s).map(|d| d.0)
        {
            self.rec_ident(r, IdentFact::Class(c));
            self.resolve_class(c);
            return Some(c);
        }
        let span = self.ast().expr(r).span;
        let text = self.src(span).to_string();
        self.report(
            Diagnostic::new("T0803", span, format!("`instanceof` needs a class name, found `{text}`"))
                .note("note", "records and unions are told apart by a discriminant: `x.kind === \"...\"`"),
        );
        None
    }

    /// Class `c` as a subtype of class type `m` (solving `c`'s type parameters).
    fn downcast_ty(&mut self, m: TyId, c: u32) -> TyId {
        let Some((mc, _)) = self.class_of(m) else { return ERROR };
        let info = self.classes[c as usize].clone();
        let Some(up) = self.upcast_to(info.this_ty, mc) else { return ERROR };
        let mut map = HashMap::default();
        self.unify(up, m, &info.params, &mut map);
        let args: Vec<TyId> = info.params.iter().map(|p| map.get(p).copied().unwrap_or(UNKNOWN)).collect();
        self.types.class(c, &args)
    }

    /// Returns false if an error was reported.
    /// Checks two operands so an integer literal on either side adopts the other side's numeric type.
    fn numeric_pair(&mut self, l: ExprId, r: ExprId, exp: Option<TyId>) -> (TyId, TyId) {
        // (in TypeScript a typed array's element is a number: `bytes[i] * 300` doesn't hint 300 a `u8`)
        let ts = self.ts_module();
        let hint_of = |t: TyId| if ts { promoted(t) } else { t };
        if self.is_num_literal(l) && !self.is_num_literal(r) {
            let tr = self.expr(r, exp);
            let hint = if self.types.is_num_like(tr) { Some(hint_of(tr)) } else { exp };
            let tl = self.expr(l, hint);
            (tl, tr)
        } else {
            let tl = self.expr(l, exp);
            let hint = if self.types.is_num_like(tl) { Some(hint_of(tl)) } else { None };
            let tr = self.expr(r, hint);
            (tl, tr)
        }
    }

    /// An integer literal, negated or not: its value.
    fn const_int(&self, e: ExprId) -> Option<i128> {
        match self.ast().expr(e).kind {
            ExprKind::Int(v) => Some(v as i128),
            ExprKind::Unary(crate::ast::UnOp::Neg, x) => self.const_int(x).map(|v| -v),
            ExprKind::Paren(x) => self.const_int(x),
            _ => None,
        }
    }

    fn mixed_ints(&mut self, a: TyId, b: TyId, span: Span) {
        let msg = format!("mixed integer types `{}` and `{}`", self.show(a), self.show(b));
        self.report(Diagnostic::new("T0504", span, msg).note("instead", "convert one side explicitly, e.g. `int(x)`"));
    }

    /// An operator with a `decimal` operand: the other is a `decimal` or an integer (converted
    /// exactly). A float never converts implicitly: `0.1` isn't exactly a tenth.
    fn decimal_arith(&mut self, op: BinOp, l: ExprId, r: ExprId, tl: TyId, tr: TyId, span: Span) -> TyId {
        if tl == ERROR || tr == ERROR {
            return ERROR;
        }
        let comparison = matches!(op, BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge);
        if !comparison && !matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem) {
            let msg = format!("`{}` doesn't apply to `decimal`", op.as_str());
            let mut d = Diagnostic::new("T0501", span, msg);
            if op == BinOp::Pow {
                d = d.note("instead", "multiply, or convert: `decimal(Number(x) ** n)` (inexact)");
            }
            self.report(d);
            return ERROR;
        }
        let ok = |c: &Self, t: TyId| t == DECIMAL || c.types.is_int(t);
        if ok(self, tl) && ok(self, tr) {
            return DECIMAL;
        }
        let (ls, rs) = (self.show(tl), self.show(tr));
        let mut d = Diagnostic::new("T0504", span, format!("can't mix `{ls}` and `{rs}` with `{}`", op.as_str()))
            .note("why", "a `decimal` combines with decimals and integers; a float is binary, so `0.1` isn't exactly a tenth");
        let (other, ot) = if tl == DECIMAL { (r, tr) } else { (l, tl) };
        let os = self.ast().expr(other).span;
        let text = self.src(os).to_string();
        if self.types.is_float(ot) {
            let ds = self.ast().expr(if tl == DECIMAL { l } else { r }).span;
            let dtext = self.src(ds).to_string();
            d = d
                .fix(Applicability::Maybe, format!("convert the number (its shortest decimal form): `decimal({text})`"), os, format!("decimal({text})"))
                .fix(Applicability::Maybe, format!("convert the decimal: `Number({dtext})`"), ds, format!("Number({dtext})"));
        } else if ot == BIGINT {
            d = d.fix(Applicability::Maybe, format!("convert: `decimal({text})`"), os, format!("decimal({text})"));
        }
        self.report(d);
        ERROR
    }

    /// An arithmetic or bitwise operator with a `bigint` operand: both must be `bigint`, as in
    /// JavaScript (where mixing throws a TypeError).
    fn bigint_arith(&mut self, op: BinOp, l: ExprId, r: ExprId, tl: TyId, tr: TyId, span: Span) -> TyId {
        if tl == ERROR || tr == ERROR {
            return ERROR;
        }
        if tl == BIGINT && tr == BIGINT {
            return BIGINT;
        }
        self.bigint_mix(op, l, r, tl, tr, span);
        ERROR
    }

    /// `bigint` mixed with another type: the fix converts the other side (an integer literal
    /// gets its `n`).
    fn bigint_mix(&mut self, op: BinOp, l: ExprId, r: ExprId, tl: TyId, tr: TyId, span: Span) {
        let (ls, rs) = (self.show(tl), self.show(tr));
        let mut d = Diagnostic::new("T0504", span, format!("can't mix `{ls}` and `{rs}` with `{}`", op.as_str()))
            .note("why", "a `bigint` only combines with another `bigint` (JavaScript throws a TypeError)");
        let (other, ot) = if tl == BIGINT { (r, tr) } else { (l, tl) };
        let os = self.ast().expr(other).span;
        let text = self.src(os).to_string();
        if self.is_int_literal(other) && !text.contains('.') {
            d = d.fix(Applicability::Safe, format!("make it a bigint literal: `{text}n`"), os, format!("{text}n"));
        } else if self.types.is_int(ot) || self.types.is_string(ot) {
            d = d.fix(Applicability::Maybe, format!("convert: `BigInt({text})`"), os, format!("BigInt({text})"));
        } else if self.types.is_float(ot) {
            let bs = self.ast().expr(if tl == BIGINT { l } else { r }).span;
            let btext = self.src(bs).to_string();
            d = d.fix(Applicability::Maybe, format!("convert the bigint: `Number({btext})`"), bs, format!("Number({btext})"));
        }
        self.report(d);
    }

    fn arith(&mut self, op: BinOp, tl: TyId, tr: TyId, span: Span) -> TyId {
        if tl == ERROR || tr == ERROR {
            return ERROR;
        }
        // Sized integers (`u8`, `i32`, ... but not `u64`) compute as `int`, as C promotes them:
        // a typed array's elements are numbers.
        let (tl, tr) = (promoted(tl), promoted(tr));
        if !self.types.is_numeric(tl) || !self.types.is_numeric(tr) {
            let msg = format!("can't apply `{}` to `{}` and `{}`", op.as_str(), self.show(tl), self.show(tr));
            let mut d = Diagnostic::new("T0501", span, msg);
            if self.types.has_undefined(tl) || self.types.has_undefined(tr) {
                d = d.note("why", "one side may be `undefined`; narrow it or use `?? 0`");
            }
            self.report(d);
            return ERROR;
        }
        if tl == tr {
            return tl;
        }
        let (fl, fr) = (self.types.is_float(tl), self.types.is_float(tr));
        match (fl, fr) {
            (true, true) => F64,
            (true, false) if tr == INT => tl,
            (false, true) if tl == INT => tr,
            (false, false) => {
                self.mixed_ints(tl, tr, span);
                ERROR
            }
            _ => {
                let msg = format!("can't mix `{}` and `{}` implicitly", self.show(tl), self.show(tr));
                self.report(Diagnostic::new("T0504", span, msg).note("instead", "convert the integer with `f64(x)`"));
                ERROR
            }
        }
    }

    fn equality(&mut self, e: ExprId, l: ExprId, r: ExprId, span: Span) -> TyId {
        let (lu, ru) = (matches!(self.ast().expr(l).kind, ExprKind::Undefined | ExprKind::Null), matches!(self.ast().expr(r).kind, ExprKind::Undefined | ExprKind::Null));
        let (tl, tr) = if matches!(self.ast().expr(l).kind, ExprKind::Str(_) | ExprKind::Undefined | ExprKind::Null) {
            self.want_optional = lu;
            let tr = self.expr(r, None);
            let tl = self.expr(l, Some(tr));
            (tl, tr)
        } else {
            // (`seen[k] === 0` of a `Record`: a missing key is `undefined`, which isn't 0)
            let keyed = |c: &mut Self, x: ExprId| matches!(c.ast().expr(x).kind, ExprKind::Index { obj, .. } if c.expr_type_peek(obj).is_some_and(|t| matches!(c.types.get(t), Ty::Map(..))));
            self.want_optional = ru || keyed(self, l);
            let tl = self.expr(l, None);
            self.want_optional = keyed(self, r);
            let tr = self.expr(r, if self.types.is_num_like(tl) { Some(tl) } else { None });
            (tl, tr)
        };
        let undefined_literal = matches!(self.ast().expr(l).kind, ExprKind::Undefined | ExprKind::Null) || matches!(self.ast().expr(r).kind, ExprKind::Undefined | ExprKind::Null);
        // (records compared by identity, as JavaScript's objects are)
        if !undefined_literal {
            self.note_object(tl);
            self.note_object(tr);
        }
        if !undefined_literal && !self.comparable(tl, tr) && !self.ast().quiet_tests.contains(&e) {
            let (ls, rs) = (self.show(tl), self.show(tr));
            let mut d = Diagnostic::new("T0502", span, format!("this comparison is always false: `{ls}` and `{rs}` have no values in common"));
            // `x === 0` on a bigint: the literal wants its `n`
            if tl == BIGINT || tr == BIGINT {
                let other = if tl == BIGINT { r } else { l };
                let os = self.ast().expr(other).span;
                let text = self.src(os).to_string();
                if self.is_int_literal(other) && !text.contains('.') {
                    d = d.fix(Applicability::Safe, format!("make it a bigint literal: `{text}n`"), os, format!("{text}n"));
                }
            }
            // `s.kind === "circel"` → suggest the literal that exists.
            let (lit, other, lit_span) = match (*self.types.get(tl), *self.types.get(tr)) {
                (_, Ty::StrLit(s)) => (Some(s), tl, self.ast().expr(r).span),
                (Ty::StrLit(s), _) => (Some(s), tr, self.ast().expr(l).span),
                _ => (None, tl, span),
            };
            if let Some(lit) = lit {
                let options: Vec<String> = self.flat_members(other).into_iter().filter_map(|m| match self.types.get(m) {
                    Ty::StrLit(s) => Some(self.name(*s).to_string()),
                    _ => None,
                }).collect();
                if !options.is_empty() {
                    let lit_text = self.name(lit).to_string();
                    let quoted: Vec<String> = options.iter().map(|o| format!("\"{o}\"")).collect();
                    d = d.note("valid values", quoted.join(", "));
                    let sug = similar(&lit_text, options.iter().map(|s| s.as_str()));
                    if let Some(first) = sug.first() {
                        d = d.fix(Applicability::Maybe, format!("use \"{first}\""), lit_span, format!("\"{first}\""));
                    }
                }
            }
            self.report(d);
        }
        BOOL
    }

    fn assign(&mut self, op: AssignOp, target: ExprId, value: ExprId, span: Span) -> TyId {
        self.assigned_field = None;
        let t = self.assign_inner(op, target, value, span);
        // After the value: `cur = cur.left` reads the narrowed `cur.left` first.
        self.invalidate_place(target);
        // A field of a class instance may be reached through other references too.
        if !matches!(self.ast().expr(target).kind, ExprKind::Ident(_)) {
            self.invalidate_heap();
        }
        // (`current.next = new ListNode(v)`: then `current.next` is a `ListNode`, as TypeScript
        // narrows it)
        if let Some(tv) = self.assigned_field.take()
            && matches!(self.ast().expr(target).kind, ExprKind::Member { .. })
            && let Some((tg, _)) = self.target_of(target)
            && !tg.path.is_empty()
        {
            self.apply(&[tg.with(tv)]);
        }
        t
    }

    fn assign_inner(&mut self, op: AssignOp, target: ExprId, value: ExprId, span: Span) -> TyId {
        // `js.name = v`, `js[k] op= v`: a JavaScript property (assigned by JavaScript's rules)
        let js_obj = match self.ast().expr(target).kind {
            ExprKind::Member { obj, .. } | ExprKind::Index { obj, .. } => Some(obj),
            _ => None,
        };
        if let Some(obj) = js_obj
            && self.expr_type_peek(obj) == Some(JS)
        {
            self.expr(target, None);
            let v = self.expr(value, Some(JS));
            if !self.js_convertible(v) {
                let shown = self.show(v);
                let s = self.ast().expr(value).span;
                self.report(Diagnostic::new("T0901", s, format!("a `{shown}` can't be passed to JavaScript")));
            }
            let _ = (op, span);
            return JS;
        }
        self.map_place = false;
        let place = self.place(target, false);
        let map_key = std::mem::take(&mut self.map_place);
        let Some(place) = place else {
            self.expr(value, None);
            return ERROR;
        };
        let vspan = self.ast().expr(value).span;
        match op {
            AssignOp::Assign => {
                let wrap = self.is_elem_place(target);
                // (a typed array's element takes any number, wrapped or clamped: `bytes[i] = 300`
                // is a number, not a `u8` literal that doesn't fit)
                let hint = if wrap && self.types.is_int(place.ty) && promoted(place.ty) == INT { INT } else { place.ty };
                let tv = self.flow(place.sites.clone(), |c| c.expr(value, Some(hint)));
                if !self.try_promote(&place, tv) {
                    self.flow(place.sites.clone(), |c| {
                        c.wrap_store = wrap;
                        c.expect_assignable(tv, place.ty, vspan, None);
                        c.wrap_store = false;
                    });
                }
                // (`current.next = node`: the field holds that, until something may change it)
                if place.local.is_none() && matches!(self.types.get(place.ty), Ty::Union(_)) && tv != ERROR && self.assignable(tv, place.ty) {
                    let declared = self.types.members(place.ty);
                    let ms: Vec<TyId> = self.types.members(tv).into_iter().map(|m| if declared.contains(&m) { m } else { self.types.widen(m) }).collect();
                    self.assigned_field = Some(self.types.union(&ms));
                }
                if let Some(local) = place.local
                    && self.assignable(tv, place.ty) && matches!(self.types.get(self.local(local).ty), Ty::Union(_)) {
                        // (a literal the declared type doesn't name is its primitive: `s = "a"` on
                        // a `string | undefined` leaves a `string`, as TypeScript's does)
                        let declared = self.types.members(self.local(local).ty);
                        let ms: Vec<TyId> = self.types.members(tv).into_iter().map(|m| if declared.contains(&m) { m } else { self.types.widen(m) }).collect();
                        let tv = self.types.union(&ms);
                        self.narrow(local, tv);
                    }
            }
            AssignOp::Op(BinOp::Nullish) => {
                // (`m[k] ??= []`: a map's key may be missing)
                if !self.types.has_undefined(place.ty) && place.ty != ERROR && !map_key {
                    let msg = format!("`??=` needs an optional target, found `{}`", self.show(place.ty));
                    self.report(Diagnostic::new("T0503", span, msg));
                }
                let inner = self.types.without_undefined(place.ty);
                let tv = self.flow(place.sites.clone(), |c| c.expr(value, Some(inner)));
                self.flow(place.sites.clone(), |c| c.expect_assignable(tv, inner, vspan, None));
                if let Some(local) = place.local {
                    self.narrow(local, inner);
                }
            }
            AssignOp::Op(BinOp::And | BinOp::Or) => {
                self.report(Diagnostic::new("X0026", span, "`&&=` and `||=` are not supported; write the `if` explicitly"));
                self.expr(value, None);
            }
            AssignOp::Op(bop) => {
                // (a typed array's element: as for `=`, the operand is a number)
                let elem_ty = if self.is_elem_place(target) && self.types.is_int(place.ty) && promoted(place.ty) == INT { INT } else { place.ty };
                let hint = if self.types.is_num_like(place.ty) { Some(elem_ty) } else { None };
                let tv = self.expr(value, hint);
                let result = if place.ty == DECIMAL || (tv == DECIMAL && !self.types.is_string(place.ty)) {
                    self.decimal_arith(bop, target, value, place.ty, tv, span)
                } else if place.ty == BIGINT || (tv == BIGINT && !self.types.is_string(place.ty)) {
                    if bop == BinOp::UShr {
                        self.report(Diagnostic::new("T0501", span, "`>>>=` doesn't apply to `bigint`: a bigint has no fixed width"));
                        ERROR
                    } else {
                        self.bigint_arith(bop, target, value, place.ty, tv, span)
                    }
                } else if bop == BinOp::Add && self.types.is_string(place.ty) {
                    if !self.printable(tv) {
                        let msg = format!("can't concatenate `{}` to a string", self.show(tv));
                        self.report(Diagnostic::new("T0501", vspan, msg));
                    }
                    STR
                } else if matches!(bop, BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr | BinOp::UShr) {
                    let js = |t: TyId| t == INT || self.types.is_float(t);
                    if js(place.ty) && js(tv) {
                        INT
                    } else {
                    if place.ty != ERROR && tv != ERROR && (!self.types.is_int(place.ty) || !self.types.is_int(tv)) {
                        let msg = format!("`{}=` needs integers, found `{}` and `{}`", bop.as_str(), self.show(place.ty), self.show(tv));
                        self.report(Diagnostic::new("T0501", span, msg));
                    }
                    place.ty
                    }
                } else {
                    let r = self.arith(bop, place.ty, tv, span);
                    if bop == BinOp::Div && r != ERROR { if r == F32 { F32 } else { F64 } } else { r }
                };
                self.wrap_store = self.is_elem_place(target);
                let fits = self.assignable(result, place.ty);
                self.wrap_store = false;
                if !self.try_promote(&place, result)
                    && result != ERROR && !fits {
                        self.blame(&place.sites);
                        let msg = format!("`{}=` produces `{}`, which can't be stored in `{}`", bop.as_str(), self.show(result), self.show(place.ty));
                        let mut d = Diagnostic::new("T0001", span, msg);
                        if self.types.is_int(place.ty) && self.types.is_float(result) {
                            d = d.note("why", "the variable is an `int`; declare it as `f64` (e.g. `let x = 0.0`) or round the result");
                        }
                        self.report(d);
                    }
            }
        }
        place.ty
    }

    /// `let x = 0; x += 0.5` — records that `x` should be an `f64` and re-checks the function.
    fn try_promote(&mut self, place: &Place, value: TyId) -> bool {
        // `let total = 0` at a module's top level, given a float: an `f64` (checked again)
        if let Some((m, ii)) = place.global
            && place.ty == INT
            && self.types.is_float(value)
            && let ItemKind::Const { mutable: true, ty: None, init, .. } = &self.modules[m as usize].ast.items[ii as usize].kind
            && matches!(self.modules[m as usize].ast.expr(*init).kind, ExprKind::Int(_))
        {
            self.global_hits.push((m, ii));
            return true;
        }
        if let Some(key) = place.int_field
            && place.ty == INT
            && self.types.is_float(value)
        {
            self.global_hits.push(key);
            return true;
        }
        let Some(local) = place.local else { return false };
        let l = self.local(local);
        if l.promotable && l.ty == INT && self.types.is_float(value) {
            let key = l.span.start;
            self.fcx().pending.insert(key);
            return true;
        }
        false
    }

    pub(super) fn root_of(&self, e: ExprId) -> Root {
        match &self.ast().expr(e).kind {
            ExprKind::Paren(x) | ExprKind::NonNull(x) => self.root_of(*x),
            ExprKind::This => Root::Heap,
            ExprKind::Member { obj, .. } | ExprKind::Index { obj, .. } => {
                if self.fcx.last().map(|f| f.class_valued.contains(obj)).unwrap_or(false) {
                    return Root::Heap;
                }
                self.root_of(*obj)
            }
            // `m.get(k)!.push(x)`: the value in the map, changed where it is
            ExprKind::Call { callee, args, .. } if args.len() == 1 => match &self.ast().expr(*callee).kind {
                ExprKind::Member { obj, name, optional: false, .. } if self.name(*name) == "get" && self.is_map_expr(*obj) => self.root_of(*obj),
                _ => Root::Temp,
            },
            ExprKind::Ident(s) => match self.lookup(*s) {
                // (a script-local standing in for a top-level binding is still a global)
                Some((id, _)) => match self.local(id).global {
                    Some((m, i)) => Root::ModuleConst(m, i),
                    None => Root::Local(id),
                },
                None => match self.scopes[self.cur as usize].values.get(s).map(|d| d.0) {
                    Some(Decl::Const(m, i)) => Root::ModuleConst(m, i),
                    _ => Root::Temp,
                },
            },
            _ => Root::Temp,
        }
    }

    /// `xs` (an expression), when it's an untyped empty array still finding its elements (see
    /// `settle_evolving`): its entry in this check's `evolving`.
    pub(super) fn evolving_slot(&self, xs: ExprId) -> Option<usize> {
        let ExprKind::Ident(s) = self.ast().expr(xs).kind else { return None };
        let (id, _) = self.lookup(s)?;
        let fcx = self.fcx.last()?;
        let key = fcx.locals[id].span.start;
        fcx.evolving.iter().position(|e| e.0 == key && e.3 == EV_ARRAY)
    }

    /// `x` (an expression), when it's a `let x` still finding its type.
    fn evolving_let(&self, x: ExprId) -> Option<usize> {
        let ExprKind::Ident(s) = self.ast().expr(x).kind else { return None };
        let (id, _) = self.lookup(s)?;
        let fcx = self.fcx.last()?;
        let key = fcx.locals[id].span.start;
        fcx.evolving.iter().position(|e| e.0 == key && e.3 == EV_LET)
    }

    /// `m`, a map made with `new Map()`, its types found from what it's given.
    pub(super) fn untyped_map(&self, m: ExprId) -> bool {
        // (`this.m`, a field written with no type)
        if let ExprKind::Member { obj, name, .. } = self.ast().expr(m).kind
            && matches!(self.ast().expr(obj).kind, ExprKind::This)
            && let Some((_, t)) = self.lookup(super::THIS_SYM)
            && let Some((c, _)) = self.class_of(t)
        {
            return self.untyped_fields.contains(&(c, name));
        }
        let ExprKind::Ident(s) = self.ast().expr(m).kind else { return false };
        let Some((id, t)) = self.lookup(s) else { return false };
        let fcx = self.fcx.last().unwrap();
        matches!(self.types.get(t), Ty::Map(..)) && fcx.evolved.contains_key(&fcx.locals[id].span.start)
    }

    /// `x` (an expression), when it's a `new C()` still finding its type arguments.
    fn evolving_class(&self, x: ExprId) -> Option<usize> {
        let ExprKind::Ident(s) = self.ast().expr(x).kind else { return None };
        let (id, _) = self.lookup(s)?;
        let fcx = self.fcx.last()?;
        let key = fcx.locals[id].span.start;
        fcx.evolving.iter().position(|e| e.0 == key && e.3 == EV_CLASS)
    }

    /// A method call on an evolving `new C()`: the type arguments its arguments give.
    fn class_evidence(&mut self, slot: usize, recv: TyId, name: Sym, args: &[Arg]) {
        let Some((c, _)) = self.class_of(recv) else { return };
        let own = self.classes[c as usize].this_ty;
        let Some(super::class::ClassMemberRef::Method(mm)) = self.class_member(own, name) else { return };
        let (params, _) = self.method_call_params(&mm);
        let free = self.classes[c as usize].params.clone();
        let mut map = HashMap::default();
        for (p, a) in params.iter().zip(args) {
            let checked = self.facts_mut().map(|f| f.expr_ty[a.expr as usize]).filter(|&t| t != ERROR);
            if let Some(t) = checked.or_else(|| self.expr_type_peek(a.expr)) {
                self.unify(p.ty, t, &free, &mut map);
            }
        }
        let targs: Vec<TyId> = free.iter().map(|p| map.get(p).copied().unwrap_or(ERROR)).collect();
        let t = self.types.class(c, &targs);
        self.fcx().evolving[slot].2.push(t);
    }

    /// `x` (an expression), when it's a `new Map()` / `new Set()` still finding its type.
    fn evolving_collection(&self, x: ExprId) -> Option<usize> {
        let ExprKind::Ident(s) = self.ast().expr(x).kind else { return None };
        let (id, _) = self.lookup(s)?;
        let fcx = self.fcx.last()?;
        let key = fcx.locals[id].span.start;
        fcx.evolving.iter().position(|e| e.0 == key && e.3 == EV_COLLECTION)
    }

    /// The array at `slot` is given the values `args`: their types join its elements'.
    pub(super) fn evolve(&mut self, slot: usize, args: &[Arg]) {
        for a in args {
            if matches!(self.ast().expr(a.expr).kind, ExprKind::Spread(_)) {
                continue;
            }
            let mark = self.diags.len();
            let t = self.expr(a.expr, None);
            self.diags.truncate(mark);
            let t = self.types.widen(t);
            if t != ERROR && !self.types.mentions_error(t) {
                self.fcx().evolving[slot].2.push(t);
            }
        }
    }

    /// What passes the callback `a` given elements `el`, when it's a type guard.
    fn guard_arg(&mut self, mut a: ExprId, el: TyId) -> Option<TyId> {
        while let ExprKind::Paren(x) = self.ast().expr(a).kind {
            a = x;
        }
        let m = self.cur;
        if let Some(&g) = self.arrow_guards.get(&(m, a)) {
            return Some(g);
        }
        match self.predicate_of(a) {
            Some((0, g)) => Some(self.guarded(el, g)),
            _ => None,
        }
    }

    /// A function declared `(p: A, ...): p is T`: (p's position, T).
    fn predicate_of(&mut self, callee: ExprId) -> Option<(usize, TyId)> {
        let ExprKind::Ident(s) = self.ast().expr(callee).kind else { return None };
        if self.lookup(s).is_some() {
            return None;
        }
        let Some(Decl::Fn(m, i)) = self.scopes[self.cur as usize].values.get(&s).map(|d| d.0) else { return None };
        let ItemKind::Function(f) = &self.modules[m as usize].ast.items[i as usize].kind else { return None };
        let rt = f.ret?;
        let crate::ast::TypeExprKind::Predicate(name, t) = self.modules[m as usize].ast.ty(rt).kind else { return None };
        let pi = f.params.iter().position(|p| p.name == name)?;
        if !f.tparams.is_empty() {
            return None;
        }
        let saved = std::mem::replace(&mut self.cur, m);
        let t = self.resolve_type(t, &[]);
        self.cur = saved;
        Some((pi, t))
    }

    /// `e` (already checked) is a map.
    fn is_map_expr(&self, e: ExprId) -> bool {
        let t = match self.facts.as_ref() {
            Some(f) => f[self.cur as usize].expr_ty.get(e as usize).copied(),
            None => match self.ast().expr(e).kind {
                ExprKind::Ident(s) => self.lookup(s).map(|(_, t)| t),
                _ => None,
            },
        };
        t.is_some_and(|t| match *self.types.get(t) {
            Ty::Map(..) => true,
            Ty::Union(ms) => self.types.tys(ms).iter().all(|&m| m == UNDEFINED || matches!(self.types.get(m), Ty::Map(..))),
            _ => false,
        })
    }

    /// The storage at path `e` (`xs`, `p.items`, `ps[0].x`'s object) is in an array, map, set,
    /// recursive alias's box or object (see `objects`), or is one: shared by every reference, as in
    /// JavaScript.
    pub(super) fn inside_shared(&mut self, e: ExprId) -> bool {
        let mut cur = e;
        loop {
            if let Some(t) = self.path_ty(cur)
                && (self.types.members(t).into_iter().any(|m| matches!(self.types.get(m), Ty::Rec(..) | Ty::Param(_) | Ty::Interface(..)) || self.maybe_object(m))
                    || self.flat_members(t).iter().any(|&m| matches!(self.types.get(m), Ty::Array(_) | Ty::Map(..) | Ty::Set(_))))
            {
                return true;
            }
            match self.ast().expr(cur).kind {
                ExprKind::Paren(x) | ExprKind::NonNull(x) | ExprKind::Member { obj: x, .. } | ExprKind::Index { obj: x, .. } => cur = x,
                _ => return false,
            }
        }
    }

    /// The type of a path (`x`, `this.a.b`, `xs[i]`), worked out from its root's.
    fn path_ty(&mut self, e: ExprId) -> Option<TyId> {
        match self.ast().expr(e).kind {
            ExprKind::Paren(x) | ExprKind::NonNull(x) => self.path_ty(x).map(|t| self.types.without_nullish(t)),
            ExprKind::Ident(s) => match self.lookup(s) {
                Some((_, t)) => Some(t),
                None => match self.scopes[self.cur as usize].values.get(&s).map(|d| d.0) {
                    Some(Decl::Const(m, i)) => Some(self.const_type(m, i)),
                    _ => None,
                },
            },
            ExprKind::This => self.lookup(super::THIS_SYM).map(|(_, t)| t),
            ExprKind::Member { obj, name, .. } => {
                let t = self.path_ty(obj)?;
                let t = self.types.without_nullish(t);
                self.field_of(t, name)
            }
            ExprKind::Index { obj, .. } => {
                let t = self.path_ty(obj)?;
                let t = self.types.without_nullish(t);
                match *self.types.get(t) {
                    Ty::Array(el) => Some(el),
                    Ty::Map(_, v) => Some(v),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Reports an error if the value rooted at `e` can't be modified in place.
    pub(super) fn check_mutable_root(&mut self, e: ExprId, span: Span) -> Option<LocalId> {
        self.check_mutable_root_ext(e, span, false)
    }

    /// `allow_temp`: mutating a temporary (`xs.slice().sort()`) is fine for method calls.
    pub(super) fn check_mutable_root_ext(&mut self, e: ExprId, span: Span, allow_temp: bool) -> Option<LocalId> {
        // (inside an array, map, set, object or type parameter's value (a class instance or an
        // object): shared, as in JavaScript, so changing it changes no variable)
        if self.inside_shared(e) {
            return None;
        }
        match self.root_of(e) {
            Root::Temp if allow_temp => None,
            Root::Heap => None,
            Root::Local(id) => Some(id),
            // Module state is mutable, as in TypeScript (`const` only fixes the binding).
            Root::ModuleConst(m, i) => {
                if let Some(f) = self.facts_mut() {
                    f.mutated_globals.insert((m, i));
                }
                None
            }
            Root::Temp => {
                self.report(Diagnostic::new("V0103", span, "can't modify a temporary value").note("instead", "assign it to a variable first"));
                None
            }
        }
    }

    pub(super) fn place(&mut self, e: ExprId, _for_ref: bool) -> Option<Place> {
        let node = self.ast().expr(e);
        let span = node.span;
        match &node.kind {
            ExprKind::Paren(x) | ExprKind::NonNull(x) => self.place(*x, _for_ref),
            ExprKind::Ident(sym) => {
                if let Some((id, ty)) = self.lookup(*sym).filter(|(id, _)| self.local(*id).global.is_none()) {
                    let key = self.local(id).span.start;
                    self.rec_ident(e, IdentFact::Local(key));
                    let declared = self.fcx.last().unwrap().locals[id].ty;
                    if let Some(f) = self.facts_mut() {
                        f.expr_ty[e as usize] = declared;
                    }
                    let (kind, kw_span) = (self.local(id).kind, self.local(id).kw_span);
                    let declared = self.local(id).ty;
                    let n = self.name(*sym).to_string();
                    if kind == LocalKind::Const {
                        if _for_ref {
                            let start = self.local(id).span.start;
                            self.rewrites.consts.push((self.cur, start));
                        }
                        let mut d = if _for_ref {
                            Diagnostic::new("V0001", span, format!("`{n}` is a `const`, so it can't be passed as `inout`"))
                                .note("why", "the callee may replace the whole value, which would change a constant")
                        } else {
                            Diagnostic::new("V0001", span, format!("can't reassign constant `{n}`"))
                        };
                        if let Some(kw) = kw_span {
                            d = d.fix(Applicability::Safe, format!("declare `{n}` with `let`"), kw, "let");
                        }
                        self.report(d);
                        return None;
                    }
                    if kind == LocalKind::LoopVar {
                        self.report(Diagnostic::new("V0001", span, format!("can't reassign loop variable `{n}`")));
                        return None;
                    }
                    let _ = ty;
                    let sites = self.local(id).sites.clone();
                    return Some(Place { ty: declared, local: Some(id), root: Some(id), sites, global: None, int_field: None });
                }
                let n = self.name(*sym).to_string();
                match self.scopes[self.cur as usize].values.get(sym).map(|d| d.0) {
                    Some(Decl::Const(cm, ci)) => {
                        let ItemKind::Const { mutable, name_span, .. } = &self.modules[cm as usize].ast.items[ci as usize].kind else { return None };
                        if !*mutable || cm != self.cur {
                            if _for_ref && cm == self.cur {
                                self.rewrites.items.push((cm, ci));
                            }
                            let mut d = Diagnostic::new("V0102", span, format!("can't reassign module constant `{n}`"));
                            if cm == self.cur {
                                let kw = self.modules[cm as usize].ast.items[ci as usize].span;
                                let kw = Span::new(kw.file, kw.start, kw.start + if self.modules[cm as usize].ast.items[ci as usize].exported { 12 } else { 5 });
                                d = d.fix(Applicability::Safe, format!("declare `{n}` with `let`"), kw, if self.modules[cm as usize].ast.items[ci as usize].exported { "export let" } else { "let" });
                            } else {
                                d = d.note("why", "imported bindings are read-only; export a function that changes it");
                            }
                            let _ = name_span;
                            self.report(d);
                            return None;
                        }
                        let ty = self.const_type(cm, ci);
                        self.rec_ident(e, IdentFact::Const(cm, ci));
                        if let Some(f) = self.facts_mut() {
                            f.expr_ty[e as usize] = ty;
                            f.mutated_globals.insert((cm, ci));
                        }
                        // The assignment narrows the script-local standing in for it (as a
                        // local's would); the place itself is still the global.
                        let local = (cm == self.cur)
                            .then(|| self.fcx().locals.iter().position(|l| l.global == Some((cm, ci))))
                            .flatten();
                        let sites = self.const_sites(cm, ci);
                        Some(Place { ty, local, root: local, sites, global: Some((cm, ci)), int_field: None })
                    }
                    Some(Decl::Fn(..)) => {
                        self.report(Diagnostic::new("V0105", span, format!("can't assign to function `{n}`")));
                        None
                    }
                    _ => {
                        self.ident(e, *sym, span);
                        None
                    }
                }
            }
            ExprKind::Member { obj, name, name_span, optional } => {
                if *optional {
                    self.report(Diagnostic::new("V0103", span, "can't assign through `?.`").note("instead", "narrow the value first: `if (x !== undefined) x.field = v`"));
                    return None;
                }
                let n = self.name(*name).to_string();
                // `C.NAME = v`: a static field (module state, as in TypeScript)
                if let ExprKind::Ident(s) = self.ast().expr(*obj).kind
                    && self.lookup(s).is_none()
                    && let Some(Decl::Class(c)) = self.scopes[self.cur as usize].values.get(&s).map(|d| d.0)
                {
                    let ty = self.static_member(e, c, *name, *name_span);
                    if ty == ERROR {
                        return None;
                    }
                    let f = self.classes[c as usize].static_fields.iter().find(|f| f.name == *name).cloned()?;
                    if f.readonly {
                        self.report(Diagnostic::new("T0811", *name_span, format!("`{n}` is `readonly`")).note("why", "a `static readonly` field is set once, by its initializer"));
                    }
                    // (`static n = 0` given a float: an `f64`, as a field is)
                    let int_field = f.member.filter(|&mi| {
                        let decl = self.class_decl(c);
                        let m = self.classes[c as usize].module;
                        matches!(&decl.members[mi as usize].kind, crate::ast::MemberKind::Field { ty: None, init: Some(i), .. } if matches!(self.modules[m as usize].ast.expr(*i).kind, ExprKind::Int(_)))
                    }).map(|mi| super::spec::field_key(c, mi));
                    return Some(Place { ty, local: None, root: None, sites: Vec::new(), global: None, int_field });
                }
                let t = self.expr(*obj, None);
                // `xs.length = n`: drops the elements from n on, as in JavaScript
                if n == "length" && matches!(self.types.get(t), Ty::Array(_)) {
                    let root = self.check_mutable_root(*obj, span);
                    // (any number: a whole one, as JavaScript requires)
                    return Some(Place { ty: F64, local: None, root, sites: Vec::new(), global: None, int_field: None });
                }
                if let Some(cls) = self.class_of(self.types_without_undef(t)) {
                    if self.types.has_undefined(t) {
                        let d = self.possibly_undefined(*obj, span);
                        self.report(d);
                    }
                    return self.class_field_place(self.types_without_undef(t), cls.0, *obj, *name, *name_span);
                }
                // `counts.apples = 3` on a `Record<string, V>`: `counts["apples"] = 3`
                if let Ty::Map(k, v) = *self.types.get(t)
                    && matches!(*self.types.get(k), Ty::Str)
                    && builtins::method(self, t, &n).is_none()
                    && n != "size"
                {
                    self.rewrites.keys.push((self.cur, e));
                    return Some(Place { ty: v, local: None, root: None, sites: Vec::new(), global: None, int_field: None });
                }
                // (changed in place: an object, as JavaScript's are, if anything shares it)
                self.note_mutated(t);
                match *self.types.get(self.types_without_undef(t)) {
                    Ty::Param(_) => {
                        self.object_fields.insert(*name);
                    }
                    // (through an interface: whatever implements it, a class instance or an
                    // object, changes; a class field kept as an integer takes the member's type)
                    Ty::Interface(..) => {
                        self.object_fields.insert(*name);
                        if let Some(mt) = self.field_of(self.types_without_undef(t), *name)
                            && mt != INT
                        {
                            self.widen_int_fields(*name);
                        }
                    }
                    _ => {}
                }
                let ft = self.member_of(t, *name, *name_span, span, *obj)?;
                let root = self.check_mutable_root(*obj, span);
                let sites = self.member_sites_of(t, *name);
                Some(Place { ty: ft, local: None, root, sites, global: None, int_field: None })
            }
            ExprKind::Index { obj, index, optional } => {
                if *optional {
                    self.report(Diagnostic::new("V0103", span, "can't assign through `?.[]`"));
                    return None;
                }
                let t = self.expr(*obj, None);
                let elem = match *self.types.get(t) {
                    Ty::Array(e) => {
                        self.index_type(*index);
                        e
                    }
                    Ty::Error => return None,
                    // `edge[1] += w`: the tuple's element, a field (`edge.1`, rewritten so)
                    Ty::Record(_) if self.tuple_of(t).is_some() => {
                        let elems = self.tuple_of(t).unwrap();
                        match self.ast().expr(*index).kind {
                            ExprKind::Int(i) if (i as usize) < elems.len() => {
                                self.expr(*index, Some(INT));
                                let field = elems[i as usize].name;
                                self.rewrites.tuple_places.push((self.cur, e, field));
                                let root = self.check_mutable_root(*obj, span);
                                return Some(Place { ty: elems[i as usize].ty, local: None, root, sites: Vec::new(), global: None, int_field: None });
                            }
                            _ => {
                                self.expr(*index, None);
                                let shown = self.show(t);
                                self.report(Diagnostic::new("T0110", span, format!("a tuple `{shown}` is indexed with a constant from 0 to {}", elems.len() - 1)));
                                return None;
                            }
                        }
                    }
                    Ty::Map(k, v) => {
                        // `counts[w] = n`: sets the key
                        self.map_place = true;
                        let it = self.expr(*index, Some(k));
                        let s = self.ast().expr(*index).span;
                        self.expect_assignable(it, k, s, Some("the key".into()));
                        let root = self.check_mutable_root(*obj, span);
                        let sites = self.sources(self.ast().expr(*obj).span);
                        return Some(Place { ty: v, local: None, root, sites, global: None, int_field: None });
                    }
                    _ => {
                        let msg = format!("can't index into `{}`", self.show(t));
                        self.report(Diagnostic::new("T0110", span, msg));
                        return None;
                    }
                };
                let root = self.check_mutable_root(*obj, span);
                let sites = self.sources(self.ast().expr(*obj).span);
                Some(Place { ty: elem, local: None, root, sites, global: None, int_field: None })
            }
            _ => {
                self.expr(e, None);
                self.report(Diagnostic::new("V0103", span, "this expression can't be assigned to"));
                None
            }
        }
    }

    /// `obj.name = ...` on a class instance: a field (not a getter or method), visible, and
    /// `readonly` only inside the owner's constructor through `this`.
    fn class_field_place(&mut self, ty: TyId, c: u32, obj: ExprId, name: Sym, name_span: Span) -> Option<Place> {
        let n = self.name(name).to_string();
        match self.class_member(ty, name) {
            Some(super::class::ClassMemberRef::Field(f)) => {
                self.check_visible(f.owner, f.vis, name, name_span);
                if f.readonly {
                    let in_owner_ctor = self.fcx.last().map(|x| x.ctor && x.class == Some(f.owner)).unwrap_or(false) && matches!(self.ast().expr(obj).kind, ExprKind::This);
                    if !in_owner_ctor {
                        self.report(
                            Diagnostic::new("T0811", name_span, format!("`{n}` is `readonly`"))
                                .note("why", "readonly fields are set once, in the constructor (or by their initializer)"),
                        );
                    }
                }
                let sites = self.class_field_sites(&f);
                // (an unannotated field with an integer initializer, which a float can widen)
                let int_field = f.member.filter(|&mi| {
                    let decl = self.class_decl(f.owner);
                    let m = self.classes[f.owner as usize].module;
                    matches!(&decl.members[mi as usize].kind, crate::ast::MemberKind::Field { ty: None, init: Some(i), .. } if matches!(self.modules[m as usize].ast.expr(*i).kind, ExprKind::Int(_)))
                }).map(|mi| super::spec::field_key(f.owner, mi));
                Some(Place { ty: f.ty, local: None, root: None, sites, global: None, int_field })
            }
            Some(super::class::ClassMemberRef::Getter(_)) => {
                self.report(Diagnostic::new("U0019", name_span, format!("`{n}` is a getter; setters are not supported yet")));
                None
            }
            Some(super::class::ClassMemberRef::Method(_)) => {
                self.report(Diagnostic::new("V0105", name_span, format!("can't assign to method `{n}`")));
                None
            }
            None => {
                let cls = self.class_names[c as usize].clone();
                let names = self.class_member_names(ty);
                let mut d = Diagnostic::new("T0107", name_span, format!("class `{cls}` has no field `{n}`"));
                let sug = similar(&n, names.iter().map(|s| s.as_str()));
                if let Some(first) = sug.first() {
                    d = d.note("did you mean", sug.join(", ")).fix(Applicability::Maybe, format!("use `{first}`"), name_span, first.to_string());
                }
                d = d.note("note", "classes have fixed fields: declare every field in the class body");
                self.report(d);
                None
            }
        }
    }

    fn index_type(&mut self, index: ExprId) {
        let ti = self.expr(index, Some(INT));
        // (a float index must be a whole number: checked when it's used)
        if ti == ERROR || self.types.is_int(ti) || self.types.is_float(ti) {
            return;
        }
        let span = self.ast().expr(index).span;
        let text = self.src(span).to_string();
        let mut d = Diagnostic::new("T0111", span, format!("array index must be an integer, found `{}`", self.show(ti)));
        if self.types.is_float(ti) {
            d = d.fix(Applicability::Maybe, format!("use `Math.floor({text})`"), span, format!("Math.floor({text})"));
        }
        self.report(d);
    }

    fn index(&mut self, e: ExprId, obj: ExprId, index: ExprId, optional: bool, absent: bool, span: Span) -> TyId {
        // `Direction[d]`: a numeric enum's member names (rewritten as `["Up", "Down"][d]`)
        if let ExprKind::Ident(sym) = self.ast().expr(obj).kind
            && let Some(names) = self.enum_names(sym)
            && !matches!(self.ast().expr(index).kind, ExprKind::Str(_))
        {
            self.expr(obj, None);
            let it = self.expr(index, Some(INT));
            if self.types.is_numeric(it) {
                self.rewrites.enum_names.push((self.cur, obj, names));
                return STR;
            }
        }
        // (`({ "&": "&amp;", "<": "&lt;" })[c]` with a string `c`: the object is a dictionary,
        // its keys any strings; checked quietly first for the types of the key and the values)
        let mut lit = obj;
        while let ExprKind::Paren(x) = self.ast().expr(lit).kind {
            lit = x;
        }
        let dict = if matches!(self.ast().expr(lit).kind, ExprKind::Object(_)) && !matches!(self.ast().expr(index).kind, ExprKind::Str(_)) {
            let saved = std::mem::take(&mut self.diags);
            let it = self.expr(index, None);
            let found = self.expr(lit, None);
            self.diags = saved;
            let any_string = self.types.is_string(it) && self.flat_members(it).iter().all(|&m| !matches!(self.types.get(m), Ty::StrLit(_)));
            (any_string && matches!(self.types.get(found), Ty::Record(_))).then_some(found)
        } else {
            None
        };
        let t = match dict {
            Some(found) => self.as_dict(lit, found),
            None => self.expr(obj, None),
        };
        let base = if optional {
            self.types.without_nullish(t)
        } else {
            if self.types.has_nullish(t) {
                let d = self.possibly_undefined(obj, span);
                self.report(d);
            }
            self.types.without_undefined(t)
        };
        let base = self.array_alias(base);
        if base == JS {
            let it = self.expr(index, None);
            if !(self.types.is_numeric(it) || self.assignable(it, STR) || it == JS || it == ERROR) {
                let shown = self.show(it);
                let s = self.ast().expr(index).span;
                self.report(Diagnostic::new("T0902", s, format!("a JavaScript property key must be a number or a string, found `{shown}`")));
            }
            self.js_read_throw(e, span);
            return JS;
        }
        // `m[1]` on a match: `m.items[1]`
        if matches!(*self.types.get(base), Ty::Class(..)) && base == self.builtin_class("RegExpExecArray") {
            self.index_type(index);
            if let Some(items) = self.interner.lookup("items") {
                self.rewrites.elems.push((self.cur, e, items, !absent));
            }
            return if absent { self.types.optional(STR) } else { STR };
        }
        let result = match *self.types.get(base) {
            Ty::Error => return ERROR,
            // (TypeScript's type for an element: present, as it assumes; out of bounds traps)
            Ty::Array(e) => {
                self.index_type(index);
                // (in TypeScript a typed array's element is a number: `bytes[i]` reads an `int`)
                let e = if self.ts_module() { if e == F32 { F64 } else { promoted(e) } } else { e };
                // (an element that may already be `null`: past the end it's that, keeping the
                // element type TypeScript gives the read)
                if absent && !self.types.has_nullish(e) { self.types.optional(e) } else { e }
            }
            Ty::Map(k, v) => {
                // `counts[w]` on a `Record<string, V>` (a map): its value, if any
                let t = self.expr(index, Some(k));
                let s = self.ast().expr(index).span;
                self.expect_assignable(t, k, s, Some("the key".into()));
                if absent { self.types.optional(v) } else { v }
            }
            Ty::Str | Ty::StrLit(_) => {
                self.index_type(index);
                if absent { self.types.optional(STR) } else { STR }
            }
            Ty::Record(fields) if self.tuple_of(base).is_some() => {
                // `t[0]`: a tuple's element (the index is a constant)
                let elems = self.tuple_of(base).unwrap();
                match self.ast().expr(index).kind {
                    ExprKind::Int(i) if (i as usize) < elems.len() => {
                        let _ = fields;
                        self.expr(index, Some(INT));
                        return elems[i as usize].ty;
                    }
                    _ => {
                        self.expr(index, None);
                        let shown = self.show(base);
                        self.report(Diagnostic::new("T0110", span, format!("a tuple `{shown}` is indexed with a constant from 0 to {}", elems.len() - 1)));
                        return ERROR;
                    }
                }
            }
            // `p[k]` with `k: keyof P` (a union of field names): the field it names (see
            // `keyed_read`)
            Ty::Record(_) | Ty::Interface(..) | Ty::Class(..) if !matches!(self.ast().expr(index).kind, ExprKind::Str(_)) => return self.keyed_read(e, obj, index, base, span),
            Ty::Record(fields) => {
                if let ExprKind::Str(key) = self.ast().expr(index).kind {
                    let key = &self.name(key).to_string();
                    let o = self.src(self.ast().expr(obj).span).to_string();
                    if self.types.fields(fields).iter().any(|f| self.name(f.name) == key) {
                        let key = key.clone();
                        self.report(
                            Diagnostic::new("X0017", span, "records are accessed with `.field`, not `[\"field\"]`")
                                .fix(Applicability::Safe, format!("use `{o}.{key}`"), span, format!("{o}.{key}")),
                        );
                        return ERROR;
                    }
                }
                self.expr(index, None);
                self.report(Diagnostic::new("X0017", span, "records can't be indexed").note("instead", "use `.field`, or a `Map<K, V>` for dynamic keys"));
                return ERROR;
            }
            _ => {
                self.expr(index, None);
                let msg = format!("can't index into `{}`", self.show(base));
                self.report(Diagnostic::new("T0110", span, msg));
                return ERROR;
            }
        };
        if optional { self.types.optional(result) } else { result }
    }

    pub(super) fn possibly_undefined(&mut self, obj: ExprId, span: Span) -> Diagnostic {
        let ospan = self.ast().expr(obj).span;
        let text = self.src(ospan).to_string();
        let rest = self.sm.get(span.file).text[ospan.end as usize..span.end as usize].to_string();
        let (chain, insert) = if rest.starts_with('.') { (format!("{text}?{rest}"), "?") } else { (format!("{text}?.{rest}"), "?.") };
        Diagnostic::new("T0120", ospan, format!("`{text}` may be `undefined`"))
            .note("narrow first", format!("`if ({text} !== undefined) {{ ... }}`"))
            .fix(Applicability::Maybe, format!("optional chaining: `{chain}`"), ospan.empty_at_end(), insert)
            .fix(Applicability::Maybe, format!("assert defined (traps if not): `{text}!{rest}`"), ospan.empty_at_end(), "!")
    }

    fn member(&mut self, e: ExprId, obj: ExprId, name: Sym, name_span: Span, optional: bool, span: Span) -> TyId {
        // Namespaces: `ns.value`, `Math.PI`.
        if let ExprKind::Ident(s) = self.ast().expr(obj).kind
            && self.lookup(s).is_none() {
                match self.scopes[self.cur as usize].values.get(&s).map(|d| d.0) {
                    Some(Decl::Ns(m)) => {
                        let scope = &self.scopes[m as usize];
                        match scope.values.get(&name).map(|d| d.0) {
                            Some(Decl::Fn(fm, fi)) => {
                                if let Some(f) = self.facts_mut() {
                                    f.members.insert(e, MemberFact::NsFn(fm, fi));
                                }
                            }
                            Some(Decl::Const(cm, ci)) => {
                                if let Some(f) = self.facts_mut() {
                                    f.members.insert(e, MemberFact::NsConst(cm, ci));
                                }
                            }
                            _ => {}
                        }
                        return self.ns_value(m, name, name_span);
                    }
                    None => {
                        let text = self.name(s).to_string();
                        if self.is_ns(&text) {
                            if let Some(f) = self.facts_mut() {
                                f.members.insert(e, MemberFact::MathConst);
                            }
                            return self.builtin_ns_member(&text, name, name_span);
                        }
                    }
                    _ => {}
                }
            }
        // `C.NAME`: a static readonly field.
        if let ExprKind::Ident(s) = self.ast().expr(obj).kind
            && self.lookup(s).is_none()
            && let Some(Decl::Class(c)) = self.scopes[self.cur as usize].values.get(&s).map(|d| d.0)
        {
            return self.static_member(e, c, name, name_span);
        }
        if matches!(self.ast().expr(obj).kind, ExprKind::Super) {
            let n = self.name(name).to_string();
            self.report(Diagnostic::new("T0801", span, format!("`super.{n}` must be called; base fields are read through `this.{n}`")));
            return ERROR;
        }
        // `x.constructor.name`: the name of x's class (a subclass's, for one)
        if self.name(name) == "name"
            && let ExprKind::Member { obj: inst, name: cn, optional: false, .. } = self.ast().expr(obj).kind
            && self.name(cn) == "constructor"
        {
            let t = self.expr(inst, None);
            if t != ERROR && !matches!(self.types.get(t), Ty::Class(..)) {
                let msg = format!("`.constructor.name` needs a class instance, found `{}`", self.show(t));
                self.report(Diagnostic::new("T0107", span, msg));
                return ERROR;
            }
            if let Some(f) = self.facts_mut() {
                f.members.insert(e, MemberFact::ClassName(inst));
            }
            return STR;
        }
        // `m.keys().next().value`: the first key (an LRU cache's oldest), or `undefined`
        if self.name(name) == "value"
            && let ExprKind::Call { callee: next, args: a1, optional: false, .. } = &self.ast().expr(obj).kind
            && a1.is_empty()
            && let ExprKind::Member { obj: it, name: nn, optional: false, .. } = self.ast().expr(*next).kind
            && self.name(nn) == "next"
            && let ExprKind::Call { callee: kc, args: a2, optional: false, .. } = &self.ast().expr(it).kind
            && a2.is_empty()
            && let ExprKind::Member { obj: coll, name: kn, optional: false, .. } = self.ast().expr(*kc).kind
            && matches!(self.name(kn), "keys" | "values")
        {
            let values = self.name(kn) == "values";
            let ct = self.expr(coll, None);
            let t = match *self.types.get(ct) {
                Ty::Map(k, v) => if values { v } else { k },
                Ty::Set(k) => k,
                Ty::Array(el) => if values { el } else { INT },
                Ty::Error => return ERROR,
                _ => {
                    let msg = format!("`{}.{}().next()` needs a map, set or array, found `{}`", self.src(self.ast().expr(coll).span), self.name(kn), self.show(ct));
                    self.report(Diagnostic::new("T0107", span, msg).note("instead", "iterate with `for (const x of ...)`"));
                    return ERROR;
                }
            };
            if let Some(f) = self.facts_mut() {
                f.members.insert(e, MemberFact::First(coll, values));
            }
            return self.types.optional(t);
        }
        self.want_optional = optional;
        let t = self.expr(obj, None);
        if t == ERROR {
            return ERROR;
        }
        if self.types.without_undefined(t) == JS {
            // a JavaScript property (`undefined` if there's none; a getter can throw)
            self.js_read_throw(e, span);
            return JS;
        }
        // `req.params` in a `routes` handler: `{ id: string }` for "/users/:id".
        if self.name(name) == "params"
            && let ExprKind::Ident(s) = self.ast().expr(obj).kind
            && let Some((id, _)) = self.lookup(s)
            && let Some(&(_, key)) = self.fcx.last().unwrap().route_locals.iter().find(|(l, _)| *l == id)
        {
            let path = self.name(key).to_string();
            let mut fields = Vec::new();
            let mut order = Vec::new();
            for seg in path.split('/') {
                if let Some(p) = seg.strip_prefix(':')
                    && let Some(sym) = self.interner.lookup(p)
                    && !order.contains(&sym)
                {
                    fields.push(Field { name: sym, ty: STR, optional: false });
                    order.push(sym);
                }
            }
            let rt = self.types.record(fields);
            self.types.note_field_order(rt, order);
            if let Some(f) = self.facts_mut() {
                f.members.insert(e, MemberFact::RouteParams(rt));
            }
            return rt;
        }
        let base = if optional {
            self.types.without_nullish(t)
        } else {
            if self.types.has_nullish(t) {
                let d = self.possibly_undefined(obj, span);
                self.report(d);
                // (nothing but `undefined`: that's the error)
                if self.types.without_nullish(t) == NEVER {
                    return ERROR;
                }
            }
            self.types.without_undefined(t)
        };
        // `process.env.NAME`: an environment variable.
        if let Ty::BuiltinNs(s) = *self.types.get(base)
            && self.name(s) == "env"
        {
            if let Some(f) = self.facts_mut() {
                f.members.insert(e, MemberFact::Env(name));
            }
            return self.types.optional(STR);
        }
        // `t.length` on a tuple `[A, B]` (a record of fields `0`, `1`): known
        if self.name(name) == "length"
            && let Some(fs) = self.tuple_of(base)
        {
            if let Some(f) = self.facts_mut() {
                f.members.insert(e, MemberFact::TupleLength(fs.len() as u32));
            }
            return if optional { self.types.optional(INT) } else { INT };
        }
        // `groups.year` on a `Record<string, V>` (a map): `groups["year"]`, as TypeScript reads
        // an index signature
        if let Ty::Map(k, v) = *self.types.get(base)
            && matches!(*self.types.get(k), Ty::Str)
            && builtins::method(self, base, &self.name(name).to_string()).is_none()
            && self.name(name) != "size"
        {
            self.rewrites.keys.push((self.cur, e));
            return if optional { self.types.optional(v) } else { v };
        }
        // Class members: visibility, and getters are calls.
        let mut getter = false;
        for m in self.flat_members(base) {
            if self.class_of(m).is_some() {
                match self.class_member(m, name) {
                    Some(super::class::ClassMemberRef::Field(f)) => {
                        self.check_visible(f.owner, f.vis, name, name_span);
                    }
                    Some(super::class::ClassMemberRef::Getter(g)) => {
                        self.check_visible(g.owner, g.vis, name, name_span);
                        getter = true;
                    }
                    Some(super::class::ClassMemberRef::Method(_)) => {
                        let n = self.name(name).to_string();
                        self.report(
                            Diagnostic::new("T0203", name_span, format!("method `{n}` must be called: `{n}(...)`"))
                                .note("instead", format!("to pass it as a function, wrap it: `(x) => obj.{n}(x)`")),
                        );
                        return ERROR;
                    }
                    None => {}
                }
            }
        }
        if getter {
            self.invalidate_heap();
        }
        let Some(ft) = self.member_of(base, name, name_span, span, obj) else { return ERROR };
        if self.spec.on {
            let sites = self.member_sites_of(base, name);
            self.note_sites(span, sites);
        }
        if !optional
            && let Some((target, _)) = self.target_of(obj)
            && target.global.is_none()
        {
            let mut path = target.path.clone();
            path.push(name);
            if let Some((t, asserted)) = self.lookup_path_ex(target.local, &path) {
                if asserted && t != ft
                    && let Some(f) = self.facts_mut()
                {
                    f.asserted_reads.insert(e);
                }
                return t;
            }
        }
        if optional { self.types.optional(ft) } else { ft }
    }

    /// `C.NAME` for a static field of class `c`.
    fn static_member(&mut self, e: ExprId, c: u32, name: Sym, name_span: Span) -> TyId {
        self.resolve_class(c);
        let info = self.classes[c as usize].clone();
        if let Some(f) = info.static_fields.iter().find(|f| f.name == name) {
            self.check_visible(f.owner, f.vis, name, name_span);
            if let (Some(fct), Some(mi)) = (self.facts_mut(), f.member) {
                fct.members.insert(e, MemberFact::StaticField(c, mi));
            }
            return f.ty;
        }
        let n = self.name(name).to_string();
        let cls = self.class_names[c as usize].clone();
        // `Comparator.defaultCompareFn` as a value: the function (a static method; a generic one
        // as the function type wanted fixes it)
        if let Some(sm) = info.statics.iter().find(|m| m.name == name && !m.getter).cloned() {
            self.check_visible(sm.owner, sm.vis, name, name_span);
            let ret = self.method_ret(&sm);
            let throws = match sm.throws {
                Some(t) if t != UNKNOWN => t,
                _ => self.error_class(),
            };
            let ft = self.types.func_throws(sm.params.clone(), ret, throws);
            let ft = if sm.tparams.is_empty() {
                Some(ft)
            } else {
                match self.member_exp.take() {
                    Some(want) => self.generic_value(e, ft, &sm.tparams, want),
                    None => None,
                }
            };
            if let Some(ft) = ft {
                if let Some(fct) = self.facts_mut() {
                    fct.members.insert(e, MemberFact::StaticFn(c, sm.member));
                }
                return ft;
            }
        }
        if info.statics.iter().any(|m| m.name == name) {
            self.report(Diagnostic::new("T0203", name_span, format!("static method `{cls}.{n}` must be called")));
            return ERROR;
        }
        let names: Vec<String> = info.static_fields.iter().map(|f| f.name).chain(info.statics.iter().map(|m| m.name)).map(|s| self.name(s).to_string()).collect();
        let mut d = Diagnostic::new("T0107", name_span, format!("class `{cls}` has no static member `{n}`"));
        let sug = similar(&n, names.iter().map(|s| s.as_str()));
        if let Some(first) = sug.first() {
            d = d.note("did you mean", sug.join(", ")).fix(Applicability::Maybe, format!("use `{first}`"), name_span, first.to_string());
        }
        if info.fields.iter().any(|f| f.name == name) || info.methods.iter().any(|m| m.name == name) {
            d = d.note("note", format!("`{n}` is an instance member: use it on an instance (`new {cls}(...).{n}`, or `this.{n}` inside the class)"));
        }
        self.report(d);
        ERROR
    }

    fn ns_value(&mut self, m: u32, name: Sym, name_span: Span) -> TyId {
        let scope = &self.scopes[m as usize];
        let decl = scope.values.get(&name).filter(|_| scope.exported_values.contains(&name)).map(|d| d.0);
        match decl {
            Some(Decl::Fn(fm, fi)) => match self.fn_sig(fm, fi) {
                Some(sig) if sig.tparams.is_empty() => self.fn_value_type(sig),
                Some(_) => {
                    let n = self.name(name).to_string();
                    self.report(Diagnostic::new("T0304", name_span, format!("generic function `{n}` must be called")));
                    ERROR
                }
                None => ERROR,
            },
            Some(Decl::Const(cm, ci)) => self.const_type(cm, ci),
            _ => {
                let n = self.name(name).to_string();
                let mname = self.modules[m as usize].name.clone();
                let exports: Vec<String> = self.scopes[m as usize].exported_values.iter().map(|&s| self.name(s).to_string()).collect();
                let mut d = Diagnostic::new("N0102", name_span, format!("`{mname}` has no exported value `{n}`"));
                let sug = similar(&n, exports.iter().map(|s| s.as_str()));
                if let Some(first) = sug.first() {
                    d = d.note("did you mean", sug.join(", ")).fix(Applicability::Maybe, format!("use `{first}`"), name_span, first.to_string());
                }
                self.report(d);
                ERROR
            }
        }
    }

    /// Type of `base.name` for a non-optional base.
    pub(super) fn member_of(&mut self, base: TyId, name: Sym, name_span: Span, span: Span, obj: ExprId) -> Option<TyId> {
        let n = self.name(name).to_string();
        // `expect(x).not.toBe(y)`: the matcher, negated
        if n == "not" && matches!(self.types.get(base), Ty::Expect(_)) {
            return Some(base);
        }
        let members = self.flat_members(base);
        let mut found = Vec::new();
        let mut lacking = Vec::new();
        for &m in &members {
            match self.field_of(m, name) {
                Some(t) => found.push(t),
                None => lacking.push(m),
            }
        }
        if lacking.is_empty() && !found.is_empty() {
            return Some(self.types.union(&found));
        }
        if let Some(msg) = builtins::member_hint(&self.types, base, &n) {
            self.report(msg.into_diag(name_span));
            return None;
        }
        if builtins::is_method(self, base, &n) {
            self.report(Diagnostic::new("T0203", name_span, format!("method `{n}` must be called: `{n}(...)`")));
            return None;
        }
        let shown = self.show(base);
        if !found.is_empty() {
            // Some union members have the field: suggest narrowing.
            let obj_text = self.src(self.ast().expr(obj).span).to_string();
            let mut d = Diagnostic::new("T0108", name_span, format!("not every variant of `{shown}` has field `{n}`"));
            let disc = self.discriminant_values(&lacking);
            if !disc.is_empty() {
                d = d.note("missing on", disc.join(", "));
            }
            d = d.note("narrow first", format!("`switch ({obj_text}.kind)` or `if ({obj_text}.kind === ...)`"));
            self.report(d);
            let _ = span;
            return None;
        }
        let names = self.field_names(base);
        let is_record = self.flat_members(base).iter().all(|&m| matches!(self.types.get(m), Ty::Record(_) | Ty::Interface(..) | Ty::Class(..)));
        let what = if is_record { "field" } else { "member" };
        let mut d = Diagnostic::new("T0107", name_span, format!("`{shown}` has no {what} `{n}`"));
        let sug = similar(&n, names.iter().map(|s| s.as_str()));
        if let Some(first) = sug.first() {
            d = d.note("did you mean", sug.join(", ")).fix(Applicability::Maybe, format!("use `{first}`"), name_span, first.to_string());
        }
        if !names.is_empty() {
            d = d.note("available", names.join(", "));
        }
        self.report(d);
        None
    }

    fn discriminant_values(&mut self, members: &[TyId]) -> Vec<String> {
        let kind = self.syms.kind;
        let mut out = Vec::new();
        for &m in members {
            if let Ty::Record(fs) = *self.types.get(m)
                && let Some(f) = self.types.fields(fs).iter().find(|f| f.name == kind)
                    && let Ty::StrLit(s) = self.types.get(f.ty) {
                        out.push(format!("\"{}\"", self.name(*s)));
                    }
        }
        out
    }

    pub(crate) fn field_of_pub(&mut self, ty: TyId, name: Sym) -> Option<TyId> {
        self.field_of(ty, name)
    }

    pub(super) fn field_of(&mut self, ty: TyId, name: Sym) -> Option<TyId> {
        let n = self.name(name).to_string();
        match *self.types.get(ty) {
            Ty::Error => Some(ERROR),
            Ty::Record(fs) => self.types.fields(fs).iter().find(|f| f.name == name).map(|f| f.ty),
            Ty::Interface(i, args) => {
                let args = self.types.tys(args).to_vec();
                self.iface_fields_inst(i, &args).into_iter().find(|f| f.name == name).map(|f| f.ty)
            }
            Ty::Param(p) => {
                let b = self.gparam(p).bound?;
                self.field_of(b, name)
            }
            Ty::Class(..) => match self.class_member(ty, name)? {
                super::class::ClassMemberRef::Field(f) => Some(f.ty),
                super::class::ClassMemberRef::Getter(g) => Some(self.method_ret(&g)),
                super::class::ClassMemberRef::Method(_) => None,
            },
            _ => builtins::property(&self.types, ty, &n),
        }
    }

    /// Whether a value of type `ty` has key `name` (`"k" in x`): known by its type, or `None`
    /// when only the value says (an optional field, a map, an `unknown`).
    pub(crate) fn has_key(&mut self, ty: TyId, name: Sym) -> Option<bool> {
        match *self.types.get(ty) {
            Ty::Record(fs) => match self.types.fields(fs).iter().find(|f| f.name == name) {
                Some(f) if f.optional || self.types.has_undefined(f.ty) => None,
                Some(_) => Some(true),
                None => Some(false),
            },
            Ty::Interface(i, args) => {
                let args = self.types.tys(args).to_vec();
                match self.iface_fields_inst(i, &args).into_iter().find(|f| f.name == name) {
                    Some(f) if f.optional || self.types.has_undefined(f.ty) => None,
                    Some(_) => Some(true),
                    None => Some(false),
                }
            }
            Ty::Rec(..) => {
                let u = self.unfold(ty);
                let ms = self.flat_members(u);
                let answers: Vec<Option<bool>> = ms.iter().map(|&m| self.has_key(m, name)).collect();
                if answers.iter().all(|a| *a == Some(true)) { Some(true) } else if answers.iter().all(|a| *a == Some(false)) { Some(false) } else { None }
            }
            Ty::Class(..) => Some(self.class_member(ty, name).is_some()),
            Ty::Map(..) | Ty::Unknown | Ty::Error | Ty::Param(_) => None,
            // (numbers, strings, functions: no keys `in` sees as a record's)
            _ => Some(false),
        }
    }

    fn field_names(&mut self, ty: TyId) -> Vec<String> {
        let mut out = Vec::new();
        for m in self.flat_members(ty) {
            match *self.types.get(m) {
                Ty::Record(fs) => out.extend(self.types.fields(fs).iter().map(|f| self.name(f.name).to_string())),
                Ty::Interface(i, args) => {
                    let args = self.types.tys(args).to_vec();
                    let fs = self.iface_fields_inst(i, &args);
                    out.extend(fs.iter().map(|f| self.name(f.name).to_string()));
                }
                Ty::Class(..) => out.extend(self.class_member_names(m)),
                _ => out.extend(builtins::property_names(self, m).into_iter().map(|s| s.to_string())),
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// std/http's `BunRequest` (a `routes` handler's request).
    fn is_route_request(&self, t: TyId) -> bool {
        match *self.types.get(t) {
            Ty::Class(c, _) => self.class_names[c as usize] == "BunRequest" && self.modules[self.classes[c as usize].module as usize].std,
            _ => false,
        }
    }

    /// The `Record<string, V>` (string-keyed map) an object literal builds, if the context wants one.
    fn map_literal_target(&mut self, exp: TyId) -> Option<TyId> {
        let exp = self.unfold(exp);
        let exp = self.types.without_undefined(exp);
        let members: Vec<TyId> = match self.types.get(exp) {
            Ty::Union(ms) => self.types.tys(*ms).to_vec(),
            _ => vec![exp],
        };
        let mut map = None;
        for &mt in &members {
            match self.types.get(mt) {
                Ty::Map(k, _) if *k == STR || matches!(self.types.get(*k), Ty::Param(_)) => map = Some(mt),
                Ty::Record(_) | Ty::Interface(..) => return None,
                _ => {}
            }
        }
        map
    }

    fn object(&mut self, fields: &[crate::ast::ObjField], exp: Option<TyId>, span: Span) -> TyId {
        let as_const = std::mem::take(&mut self.as_const);
        let mut seen = HashSet::default();
        for f in fields {
            if !seen.insert(f.name) {
                let n = self.name(f.name).to_string();
                self.report(Diagnostic::new("N0006", f.name_span, format!("duplicate field `{n}`")));
            }
        }
        // `{ "Content-Type": v }` for a `Record<string, V>`: a map literal (`{}` of any map).
        let empty_map = |c: &mut Self, e: TyId| {
            let e = c.types.without_undefined(e);
            (fields.is_empty() && matches!(c.types.get(e), Ty::Map(..))).then_some(e)
        };
        if let Some(mt) = exp.and_then(|e| self.map_literal_target(e).or_else(|| empty_map(self, e))) {
            let Ty::Map(_, vt) = *self.types.get(mt) else { unreachable!() };
            for f in fields {
                let key = self.name(f.name).to_string();
                let saved = self.route_key.take();
                if key.contains("/:") {
                    self.route_key = Some(f.name);
                }
                let t = self.expr(f.value, Some(vt));
                self.route_key = saved;
                let vspan = self.ast().expr(f.value).span;
                self.expect_assignable(t, vt, vspan, Some(format!("key \"{key}\"")));
            }
            return mt;
        }
        for f in fields {
            let text = self.name(f.name);
            let ident = text.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$') && text.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$');
            if f.quoted && !ident {
                self.report(
                    Diagnostic::new("X0017", f.name_span, format!("\"{text}\" isn't a field name: records have identifier field names"))
                        .note("instead", "for dynamic keys, give the value a `Record<string, V>` (or `Map<string, V>`) type"),
                );
            }
        }
        let target = exp.and_then(|e| self.pick_record(fields, e, span));
        if let Some((ERROR, _)) = target {
            return ERROR;
        }
        let Some((target_ty, tfields)) = target else {
            if exp.map(|e| self.types.get(e) == &Ty::Error).unwrap_or(false) {
                for f in fields {
                    self.expr(f.value, None);
                }
                return ERROR;
            }
            // No usable context: infer a record type, widening string literals as TS does.
            let mut out = Vec::new();
            let mut lits = Vec::new();
            let mut srcs = Vec::new();
            for f in fields {
                self.as_const = as_const && matches!(self.ast().expr(f.value).kind, ExprKind::Object(_));
                let t = self.expr(f.value, None);
                self.as_const = false;
                let t = if as_const && matches!(self.types.get(t), Ty::StrLit(_)) { t } else { self.types.widen(t) };
                out.push(Field { name: f.name, ty: t, optional: false });
                // (`n: 0`, `n: -1`: an `int` a float may later need to fit)
                let mut v = f.value;
                if let ExprKind::Unary(crate::ast::UnOp::Neg, x) = self.ast().expr(v).kind {
                    v = x;
                }
                if t == INT && matches!(self.ast().expr(v).kind, ExprKind::Int(_)) {
                    lits.push((f.name, super::spec::lit_key(self.cur, v)));
                }
                if self.spec.on {
                    let sites = self.sources(self.ast().expr(f.value).span);
                    srcs.push((f.name, sites));
                }
            }
            out.dedup_by_key(|f| f.name);
            let order: Vec<Sym> = out.iter().map(|f| f.name).collect();
            let t = self.types.record(out);
            self.types.note_field_order(t, order);
            for (name, k) in lits {
                self.lit_fields.entry((t, name)).or_default().push(k);
            }
            for (name, sites) in srcs {
                if !sites.is_empty() {
                    self.spec_fields.entry((t, name)).or_default().extend(sites);
                }
            }
            return t;
        };
        let mut provided = HashSet::default();
        for f in fields {
            match tfields.iter().find(|tf| tf.name == f.name) {
                Some(tf) => {
                    provided.insert(f.name);
                    let sites = self.record_field_sites(target_ty, f.name);
                    let (fty, value) = (tf.ty, f.value);
                    let fname = self.name(f.name).to_string();
                    self.flow(sites, |c| {
                        let t = c.expr(value, Some(fty));
                        let vspan = c.ast().expr(value).span;
                        c.expect_assignable(t, fty, vspan, Some(format!("field `{fname}`")));
                    });
                }
                None => {
                    self.expr(f.value, None);
                    let n = self.name(f.name).to_string();
                    let missing: Vec<String> = tfields.iter().filter(|tf| !fields.iter().any(|f| f.name == tf.name)).map(|tf| self.name(tf.name).to_string()).collect();
                    let shown = self.show(target_ty);
                    let mut d = Diagnostic::new("T0102", f.name_span, format!("`{shown}` has no field `{n}`"));
                    let sug = similar(&n, missing.iter().map(|s| s.as_str()));
                    if let Some(first) = sug.first() {
                        d = d.fix(Applicability::Maybe, format!("rename to `{first}`"), f.name_span, first.to_string());
                    }
                    let all: Vec<String> = tfields.iter().map(|tf| self.name(tf.name).to_string()).collect();
                    d = d.note("fields", all.join(", "));
                    self.report(d);
                }
            }
        }
        let missing: Vec<&Field> = tfields.iter().filter(|tf| !tf.optional && !provided.contains(&tf.name)).collect();
        // Don't report missing fields when a misspelled field already explains it.
        let misspelled = fields.iter().any(|f| !tfields.iter().any(|tf| tf.name == f.name));
        if !missing.is_empty() && !misspelled {
            let names: Vec<String> = missing.iter().map(|f| format!("`{}`", self.name(f.name))).collect();
            let insert: Vec<String> = missing.iter().map(|f| format!("{}: <{}>", self.name(f.name), self.show(f.ty))).collect();
            let shown = self.show(target_ty);
            let close = Span::new(span.file, span.end.saturating_sub(1), span.end.saturating_sub(1));
            let sep = if fields.is_empty() { " " } else { ", " };
            let d = Diagnostic::new("T0101", span, format!("missing field{} {} for `{shown}`", if missing.len() == 1 { "" } else { "s" }, names.join(", ")))
                .fix(Applicability::Placeholder, format!("add {}", insert.join(", ")), close, format!("{sep}{} ", insert.join(", ")));
            self.report(d);
        }
        target_ty
    }

    /// `counts[k]++`, `counts[k] = v` where `k` is a union of the record's field names: as
    /// `k === "a" ? counts.a++ : counts.b++` (rewritten, see `rewrite`). The type it gives, or
    /// `None` when the target isn't one.
    fn keyed_place(&mut self, e: ExprId, target: ExprId, value: Option<ExprId>) -> Option<TyId> {
        let ExprKind::Index { obj, index, optional: false } = self.ast().expr(target).kind else { return None };
        let base = self.expr_type_peek(obj)?;
        let base = self.unfold(base);
        let Ty::Record(_) = *self.types.get(base) else { return None };
        if !super::rewrite::simple_path(self.ast(), obj) || !super::rewrite::simple_path(self.ast(), index) {
            return None;
        }
        // (each branch gets its own copy of the value)
        if value.is_some_and(|v| !super::rewrite::copyable(self.ast(), v)) {
            return None;
        }
        // (a variable or field path: checking it again when it isn't one is harmless)
        let it = self.expr(index, None);
        let mut keys = Vec::new();
        let mut tys = Vec::new();
        for m in self.flat_members(it) {
            let Ty::StrLit(n) = *self.types.get(m) else { return None };
            let ft = self.field_of(base, n)?;
            keys.push(n);
            tys.push(ft);
        }
        if keys.len() < 2 {
            return None;
        }
        self.expr(obj, None);
        let ft = self.types.union(&tys);
        if let Some(v) = value {
            let vt = self.expr(v, Some(ft));
            let vspan = self.ast().expr(v).span;
            for &t in &tys {
                if !self.assignable(vt, t) && !self.types.is_numeric(vt) {
                    self.expect_assignable(vt, t, vspan, None);
                    break;
                }
            }
        }
        self.rewrites.keyed_places.push((self.cur, e, keys));
        Some(ft)
    }

    /// `p[k]` where `k` is a union of p's field names: `k === "a" ? p.a : p.b` (see `rewrite`),
    /// of the union of their types.
    fn keyed_read(&mut self, e: ExprId, obj: ExprId, index: ExprId, base: TyId, span: Span) -> TyId {
        let it = self.expr(index, None);
        if it == ERROR {
            return ERROR;
        }
        let mut keys = Vec::new();
        let mut tys = Vec::new();
        for m in self.flat_members(it) {
            let name = match *self.types.get(m) {
                Ty::StrLit(n) => n,
                _ => {
                    let (shown, bs) = (self.show(it), self.show(base));
                    self.report(
                        Diagnostic::new("X0017", span, format!("`{bs}` can't be indexed with `{shown}`"))
                            .note("instead", "use `.field`, give the key a type of the field names (`keyof T`), or use a `Map<K, V>` for dynamic keys"),
                    );
                    return ERROR;
                }
            };
            let Some(ft) = self.field_of(base, name) else {
                let (n, bs) = (self.name(name).to_string(), self.show(base));
                self.report(Diagnostic::new("T0102", span, format!("`{bs}` has no field `{n}`")));
                return ERROR;
            };
            keys.push(name);
            tys.push(ft);
        }
        let pure = |c: &Self, x: ExprId| super::stmt::place_path(c.ast(), x) || matches!(c.ast().expr(x).kind, ExprKind::Str(_));
        if !pure(self, obj) || !pure(self, index) {
            self.report(Diagnostic::new("U0011", span, "indexing with a key of several fields needs a variable on both sides yet").note("instead", "put the object and the key in `const`s first"));
            return ERROR;
        }
        self.rewrites.keyed.push((self.cur, e, keys));
        self.types.union(&tys)
    }

    /// `{ ...a, y: 3 }`: the literal with a's fields written out (`{ x: a.x, y: 3 }`, see
    /// `rewrite`), checked again; its type for now. What's spread must be a variable or a field
    /// path (read once per field).
    fn object_spread(&mut self, e: ExprId, fields: &[crate::ast::ObjField], span: Span) -> TyId {
        let mut out: Vec<Field> = Vec::new();
        let mut expansions = Vec::new();
        let mut ok = true;
        for f in fields {
            let ExprKind::Spread(x) = self.ast().expr(f.value).kind else {
                let t = self.expr(f.value, None);
                let t = self.types.widen(t);
                match out.iter_mut().find(|o| o.name == f.name) {
                    Some(o) => o.ty = t,
                    None => out.push(Field { name: f.name, ty: t, optional: false }),
                }
                continue;
            };
            // `...{ name: "nut" }`: its fields, as written here
            if let ExprKind::Object(inner) = &self.ast().expr(x).kind
                && !inner.iter().any(|f| matches!(self.ast().expr(f.value).kind, ExprKind::Spread(_)))
            {
                let inner: Vec<(Sym, ExprId)> = inner.iter().map(|f| (f.name, f.value)).collect();
                for (name, value) in inner {
                    let t = self.expr(value, None);
                    let t = self.types.widen(t);
                    match out.iter_mut().find(|o| o.name == name) {
                        Some(o) => o.ty = t,
                        None => out.push(Field { name, ty: t, optional: false }),
                    }
                }
                continue;
            }
            let t = self.expr(x, None);
            if t == ERROR {
                ok = false;
                continue;
            }
            let base = self.types.without_undefined(t);
            let spread: Option<Vec<Field>> = match *self.types.get(base) {
                Ty::Record(_) => Some(self.types.fields_in_order(base)),
                Ty::Interface(i, args) => {
                    let args = self.types.tys(args).to_vec();
                    Some(self.iface_fields_inst(i, &args))
                }
                _ => None,
            };
            let xspan = self.ast().expr(x).span;
            let Some(sf) = spread else {
                let shown = self.show(t);
                self.report(Diagnostic::new("U0011", xspan, format!("spreading `{shown}` into an object literal is not supported yet")).note("supported", "spreading a record or an interface value"));
                ok = false;
                continue;
            };
            if !super::stmt::place_path(self.ast(), x) || base != t {
                self.report(
                    Diagnostic::new("U0011", xspan, "only a variable or a field path can be spread into an object literal yet")
                        .note("instead", "put the value in a `const` first, and spread that"),
                );
                ok = false;
                continue;
            }
            let mut names = Vec::new();
            for fl in &sf {
                let mut keep = false;
                match out.iter_mut().find(|o| o.name == fl.name) {
                    // (`{ ...a, ...changes }`, `changes.f` optional: `changes.f ?? a.f`)
                    Some(o) if fl.optional => {
                        let later = self.types.without_undefined(fl.ty);
                        o.ty = self.types.union(&[later, o.ty]);
                        keep = true;
                    }
                    Some(o) => *o = *fl,
                    None => out.push(*fl),
                }
                names.push((fl.name, keep));
            }
            expansions.push(names);
        }
        if !ok {
            return ERROR;
        }
        self.rewrites.spreads.push((self.cur, e, expansions));
        let order: Vec<Sym> = out.iter().map(|f| f.name).collect();
        let t = self.types.record(out);
        self.types.note_field_order(t, order);
        let _ = span;
        t
    }

    /// Chooses the record type an object literal should be checked against.
    fn pick_record(&mut self, fields: &[crate::ast::ObjField], exp: TyId, span: Span) -> Option<(TyId, Vec<Field>)> {
        let members = self.flat_members(exp);
        // (the record and interface members of the expected type: candidates alike)
        let mut records: Vec<(TyId, Vec<Field>)> = Vec::new();
        for &m in &members {
            match *self.types.get(m) {
                Ty::Record(fs) => records.push((m, self.types.fields(fs).to_vec())),
                Ty::Interface(i, args) => {
                    let args = self.types.tys(args).to_vec();
                    let fs = self.iface_fields_inst(i, &args);
                    records.push((m, fs));
                }
                _ => {}
            }
        }
        if records.is_empty() {
            return None;
        }
        if records.len() == 1 {
            let (m, fs) = records.into_iter().next().unwrap();
            let named = if members.len() == 1 { exp } else { m };
            return Some((named, fs));
        }
        // Discriminated union: find a literal-valued field in the object that selects a variant.
        for f in fields {
            let ExprKind::Str(val_sym) = self.ast().expr(f.value).kind else { continue };
            let val = &self.name(val_sym).to_string();
            let is_disc = records.iter().all(|(_, fs)| fs.iter().any(|x| x.name == f.name && matches!(self.types.get(x.ty), Ty::StrLit(_))));
            if !is_disc {
                continue;
            }
            let lit = self.types.str_lit(val_sym);
            // (two variants of one discriminant, `{ type: "html", ... }` of two kinds of token: the
            // one whose fields the literal's fit)
            let with: Vec<&(TyId, Vec<Field>)> = records.iter().filter(|(_, fs)| fs.iter().any(|x| x.name == f.name && x.ty == lit)).collect();
            if with.len() > 1 {
                let names: Vec<Sym> = fields.iter().map(|f| f.name).collect();
                if let Some((m, fs)) = with.iter().find(|(_, fs)| names.iter().all(|n| fs.iter().any(|x| x.name == *n)) && fs.iter().all(|x| x.optional || names.contains(&x.name))) {
                    return Some((*m, fs.clone()));
                }
            }
            if let Some((m, fs)) = with.first() {
                return Some((*m, fs.clone()));
            }
            let options: Vec<String> = records
                .iter()
                .filter_map(|(_, fs)| fs.iter().find(|x| x.name == f.name))
                .filter_map(|x| match self.types.get(x.ty) {
                    Ty::StrLit(s) => Some(self.name(*s).to_string()),
                    _ => None,
                })
                .collect();
            let fname = self.name(f.name).to_string();
            let shown = self.show(exp);
            let vspan = self.ast().expr(f.value).span;
            let quoted: Vec<String> = options.iter().map(|o| format!("\"{o}\"")).collect();
            let mut d = Diagnostic::new("T0103", vspan, format!("`{fname}: \"{val}\"` is not a variant of `{shown}`")).note("valid values", quoted.join(", "));
            let sug = similar(val, options.iter().map(|s| s.as_str()));
            if let Some(first) = sug.first() {
                d = d.fix(Applicability::Maybe, format!("use \"{first}\""), vspan, format!("\"{first}\""));
            }
            self.report(d);
            for f in fields {
                self.expr(f.value, None);
            }
            let _ = span;
            return Some((ERROR, Vec::new()));
        }
        // (`{ ok: true, value }`: a `true`/`false` field selecting the variant)
        for f in fields {
            let ExprKind::Bool(b) = self.ast().expr(f.value).kind else { continue };
            let is_disc = records.iter().all(|(_, fs)| fs.iter().any(|x| x.name == f.name && matches!(self.types.get(x.ty), Ty::BoolLit(_))));
            if !is_disc {
                continue;
            }
            let lit = self.types.bool_lit(b);
            if let Some((m, fs)) = records.iter().find(|(_, fs)| fs.iter().any(|x| x.name == f.name && x.ty == lit)) {
                return Some((*m, fs.clone()));
            }
        }
        // No discriminant: the one variant whose fields the literal's fit.
        let names: Vec<Sym> = fields.iter().map(|f| f.name).collect();
        let fitting: Vec<&(TyId, Vec<Field>)> = records.iter().filter(|(_, fs)| names.iter().all(|n| fs.iter().any(|x| x.name == *n)) && fs.iter().all(|x| x.optional || names.contains(&x.name))).collect();
        if let [one] = fitting.as_slice() {
            return Some(((*one).0, (*one).1.clone()));
        }
        None
    }

    /// `a` and `b` as one type, TypeScript's `number` being `int` or `f64` (`int[]` and `f64[]`
    /// are `f64[]`; tuples join by element), else their union.
    /// Two record types with the same fields, the same ones optional, whose types join
    /// (numbers with numbers, or the same type).
    fn same_fields(&mut self, a: TyId, b: TyId) -> bool {
        let (fa, fb) = (self.types.fields_in_order(a), self.types.fields_in_order(b));
        fa.len() == fb.len()
            && fa.iter().all(|f| {
                fb.iter().any(|g| {
                    g.name == f.name && g.optional == f.optional && (g.ty == f.ty || (self.types.is_numeric(g.ty) && self.types.is_numeric(f.ty)) || matches!((self.types.get(f.ty), self.types.get(g.ty)), (Ty::Record(_), Ty::Record(_)) | (Ty::Array(_), Ty::Array(_))))
                })
            })
    }

    fn join_numbers(&mut self, a: TyId, b: TyId) -> TyId {
        if a == b || b == NEVER {
            return a;
        }
        if a == NEVER {
            return b;
        }
        if self.types.is_numeric(a) && self.types.is_numeric(b) {
            return if self.types.is_float(a) { a } else { b };
        }
        match (*self.types.get(a), *self.types.get(b)) {
            (Ty::Array(x), Ty::Array(y)) => {
                let e = self.join_numbers(x, y);
                self.types.array(e)
            }
            _ => match (self.tuple_of(a), self.tuple_of(b)) {
                (Some(x), Some(y)) if x.len() == y.len() => {
                    let tys: Vec<TyId> = x.iter().zip(&y).map(|(p, q)| (p.ty, q.ty)).collect::<Vec<_>>().into_iter().map(|(p, q)| self.join_numbers(p, q)).collect();
                    self.tuple_ty(&tys)
                }
                // (records of the same fields, `[{ year: 0 }, { year: 12.2 }]`: field by field, a
                // `{ year: number }` as TypeScript's)
                (None, None) if matches!((self.types.get(a), self.types.get(b)), (Ty::Record(_), Ty::Record(_))) && self.same_fields(a, b) => {
                    let (fa, fb) = (self.types.fields_in_order(a), self.types.fields_in_order(b));
                    let mut fields = Vec::new();
                    for f in &fa {
                        let g = fb.iter().find(|g| g.name == f.name).unwrap();
                        let ty = self.join_numbers(f.ty, g.ty);
                        fields.push(Field { name: f.name, ty, optional: f.optional });
                    }
                    let order: Vec<Sym> = fields.iter().map(|f| f.name).collect();
                    let r = self.types.record(fields);
                    self.types.note_field_order(r, order);
                    r
                }
                _ => self.types.union(&[a, b]),
            },
        }
    }

    /// A row of `[[0, false], ...] as const`: a tuple of its (widened) elements' types, when
    /// they're of more than one.
    fn const_row(&mut self, row: ExprId) -> Option<TyId> {
        let ExprKind::Array(xs) = &self.ast().expr(row).kind else { return None };
        if xs.iter().any(|&el| matches!(self.ast().expr(el).kind, ExprKind::Spread(_))) {
            return None;
        }
        let xs = xs.clone();
        let mark = self.diags.len();
        // (an empty array's elements come from the other rows': `[[1, 2], 3]`, `[[], 0]`)
        let tys: Vec<TyId> = xs.iter().map(|&el| if matches!(&self.ast().expr(el).kind, ExprKind::Array(e) if e.is_empty()) { self.types.array(NEVER) } else { self.expr(el, None) }).collect();
        self.diags.truncate(mark);
        let wide: Vec<TyId> = tys.iter().map(|&t| self.types.widen(t)).collect();
        let one_type = wide.iter().all(|&t| t == wide[0]) || wide.iter().all(|&t| self.types.is_numeric(t));
        (!wide.contains(&ERROR) && !one_type).then(|| self.tuple_ty(&wide))
    }

    pub(super) fn array_lit(&mut self, elems: &[ExprId], exp: Option<TyId>, span: Span) -> TyId {
        // `[lo, hi]` where a tuple `[number, number]` is expected
        if let Some(t) = exp.and_then(|e| self.flat_members(e).into_iter().find(|&m| self.tuple_of(m).is_some_and(|f| f.len() == elems.len())))
            && !elems.iter().any(|&x| matches!(self.ast().expr(x).kind, ExprKind::Spread(_)))
        {
            let fields = self.tuple_of(t).unwrap();
            // (a field the hint leaves open, a type argument still to infer: the element's own type)
            let mut tys = Vec::new();
            let mut open = false;
            for (&x, f) in elems.iter().zip(&fields) {
                let ft = f.ty;
                let et = self.expr(x, if ft == ERROR { None } else { Some(ft) });
                let s = self.ast().expr(x).span;
                if ft == ERROR {
                    open = true;
                    tys.push(self.types.widen(et));
                } else {
                    self.expect_assignable(et, ft, s, None);
                    tys.push(ft);
                }
            }
            return if open { self.tuple_ty(&tys) } else { t };
        }
        let elem_exp = exp.and_then(|e| {
            self.flat_members(e).into_iter().find_map(|m| match self.types.get(m) {
                Ty::Array(t) => Some(*t),
                _ => None,
            })
        });
        if let Some(et) = elem_exp {
            // (an element hint with an open part, a type argument still to infer: the elements' own)
            let open = et != ERROR && self.types.mentions_error(et);
            let mut tys = Vec::new();
            for &x in elems {
                let s = self.ast().expr(x).span;
                let t = match self.ast().expr(x).kind {
                    ExprKind::Spread(inner) => self.spread_elem(inner, Some(et)),
                    _ => self.expr(x, Some(et)),
                };
                if open {
                    tys.push(t);
                } else {
                    self.expect_assignable(t, et, s, None);
                }
            }
            if open {
                let u = self.types.union(&tys);
                return self.types.array(u);
            }
            return self.types.array(et);
        }
        if elems.is_empty() {
            if exp == Some(ERROR) {
                return ERROR;
            }
            self.report(
                Diagnostic::new("T0301", span, "can't infer the element type of an empty array")
                    .note("instead", "annotate the binding: `const xs: T[] = []`"),
            );
            return ERROR;
        }
        let mut tys = Vec::new();
        // (`[[1, 2], []]`, `[{ xs: [1] }, { xs: [] }]`: an element holding an empty array takes
        // its type from the others)
        fn holds_empty(ast: &crate::ast::Ast, x: ExprId) -> bool {
            match &ast.expr(x).kind {
                ExprKind::Array(xs) => xs.is_empty() || xs.iter().any(|&y| holds_empty(ast, y)),
                ExprKind::Object(fs) => fs.iter().any(|f| holds_empty(ast, f.value)),
                &ExprKind::Paren(y) => holds_empty(ast, y),
                _ => false,
            }
        }
        let empty = |c: &Self, x: ExprId| holds_empty(c.ast(), x);
        for &x in elems {
            if empty(self, x) {
                continue;
            }
            let t = match self.ast().expr(x).kind {
                ExprKind::Spread(inner) => self.spread_elem(inner, None),
                _ => self.expr(x, None),
            };
            // (TypeScript widens a fresh literal, `["a"]` to `string[]`, not a declared union
            // of them: `[Color.Red, Color.Blue]` is a `Color[]`)
            let declared = matches!(self.types.get(t), Ty::Union(_)) && matches!(self.ast().expr(x).kind, ExprKind::Ident(_) | ExprKind::Member { .. } | ExprKind::Index { .. } | ExprKind::Call { .. });
            tys.push(if declared { t } else { self.types.widen(t) });
        }
        let others = (!tys.is_empty()).then(|| self.types.union(&tys));
        for &x in elems {
            if empty(self, x) {
                let t = self.expr(x, others);
                tys.push(t);
            }
        }
        if tys.iter().all(|&t| self.types.is_numeric(t)) && tys.iter().any(|&t| self.types.is_float(t)) && tys.iter().all(|&t| t == INT || t == F64) {
            return self.types.array(F64);
        }
        // (`[{ year: 0 }, { year: 12.2 }]`: one record type, `{ year: number }`, as TypeScript's)
        if tys.len() > 1 && tys.iter().all(|&t| matches!(self.types.get(t), Ty::Record(_)) && self.tuple_of(t).is_none()) && tys[1..].iter().all(|&t| self.same_fields(tys[0], t)) {
            let one = tys[1..].iter().fold(tys[0], |a, &b| self.join_numbers(a, b));
            return self.types.array(one);
        }
        let u = self.types.union(&tys);
        self.types.array(u)
    }

    /// `...x`: the type of the elements it spreads (of an array, a set, or a string's characters).
    pub(super) fn spread_elem(&mut self, x: ExprId, elem: Option<TyId>) -> TyId {
        let hint = elem.map(|e| self.types.array(e));
        let t = self.expr(x, hint);
        let base = self.types.without_undefined(t);
        let bt = *self.types.get(base);
        match bt {
            Ty::Array(e) | Ty::Set(e) | Ty::Gen(e) => e,
            Ty::Str | Ty::StrLit(_) => STR,
            // (a map's entries: `[key, value]`)
            Ty::Map(k, v) => self.tuple_ty(&[k, v]),
            Ty::Js => JS,
            Ty::Error => ERROR,
            _ if let Some(e) = self.iterator_of(base) => {
                self.rewrites.iterators.push((self.cur, x));
                e
            }
            _ => {
                let shown = self.show(t);
                let s = self.ast().expr(x).span;
                self.report(Diagnostic::new("T0402", s, format!("can't spread `{shown}`: `...` takes an array, a set, a map or a string")));
                ERROR
            }
        }
    }

    /// The arrow's type, and what passes it when it's a type guard of its first parameter:
    /// declared (`(x): x is T => ...`), or a body true exactly when the parameter is of a part
    /// of its type (`(x) => x !== undefined`, `(x) => typeof x === "string"`), as TypeScript
    /// infers.
    fn arrow_guarded(&mut self, f: &ArrowFn, exp: Option<TyId>, span: Span) -> (TyId, Option<TyId>) {
        let test_cb = std::mem::take(&mut self.test_callback);
        let exp_fn = exp.and_then(|e| {
            self.flat_members(e).into_iter().find_map(|m| match *self.types.get(m) {
                Ty::Func(ps, r, _) => Some((self.types.params(ps).to_vec(), r)),
                _ => None,
            })
        });
        // A closure can throw when its context allows it (`(req) => Response throws Error`).
        let exp_throws = exp
            .and_then(|e| {
                self.flat_members(e).into_iter().find_map(|m| match *self.types.get(m) {
                    Ty::Func(_, _, th) => Some(th),
                    _ => None,
                })
            })
            .unwrap_or(NEVER);
        let is_async = f.is_async;
        // An async arrow's context expects `(...) => Promise<T>`: its body returns `T`, and
        // whatever it throws rejects the promise.
        // (The context may also accept a plain value: `Response | Promise<Response, Error>`.)
        let (exp_fn, exp_throws) = match exp_fn {
            Some((ps, r)) if is_async => {
                let promised = self.flat_members(r).into_iter().find_map(|m| match *self.types.get(m) {
                    Ty::Promise(v, err) => Some((v, err)),
                    _ => None,
                });
                match promised {
                    Some((v, err)) => (Some((ps, v)), err),
                    None => (Some((ps, r)), NEVER),
                }
            }
            other => (other, exp_throws),
        };
        let tscope = self.tscope();
        let mut params = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
            let ty = match p.ty {
                // (`(xs: any[], x: any) => ...` where what's passed is known, `test.each`'s rows
                // say: those types, as if it were unannotated)
                Some(t) if self.mentions_any(t)
                    && let Some(fp) = exp_fn.as_ref().and_then(|(ps, _)| ps.get(i))
                    && !self.types.mentions(fp.ty, &self.infer_free)
                    && fp.ty != ERROR =>
                {
                    fp.ty
                }
                // (`(e: unknown) => ...` where an `Error` is passed: it takes the `Error`, as it
                // takes anything)
                Some(t) if self.resolve_type(t, &tscope) == UNKNOWN
                    && let Some(fp) = exp_fn.as_ref().and_then(|(ps, _)| ps.get(i))
                    && !self.types.mentions(fp.ty, &self.infer_free)
                    && fp.ty != ERROR =>
                {
                    fp.ty
                }
                Some(t) => self.resolve_type(t, &tscope),
                None => match exp_fn.as_ref().and_then(|(ps, _)| ps.get(i)) {
                    Some(fp) if !self.types.mentions(fp.ty, &self.infer_free) => fp.ty,
                    _ => {
                        if exp != Some(ERROR) {
                            let n = self.name(p.name).to_string();
                            self.report(
                                Diagnostic::new("T0301", p.span, format!("can't infer the type of parameter `{n}`"))
                                    .fix(Applicability::Placeholder, format!("annotate `{n}: T`"), p.span.empty_at_end(), ": T"),
                            );
                        }
                        ERROR
                    }
                },
            };
            let ty = if p.optional { self.types.optional(ty) } else { ty };
            // (`(...args: A) => ...` where `(...args: A) => R` is wanted: `args` is the tuple)
            let ty = match *self.types.get(ty) {
                Ty::Spread(t) if p.rest => t,
                _ => ty,
            };
            params.push(FnParam { ty, inout: p.inout, optional: p.optional });
        }
        // (the function's type takes a rest parameter of a tuple type as its elements)
        let fn_params: Vec<FnParam> = params
            .iter()
            .zip(&f.params)
            .map(|(fp, p)| if p.rest && !matches!(self.types.get(fp.ty), Ty::Array(_) | Ty::Error) { FnParam { ty: self.types.intern(Ty::Spread(fp.ty)), ..*fp } } else { *fp })
            .collect();
        let declared_ret = f.ret.map(|t| self.resolve_type(t, &tscope));
        let declared_ret = match declared_ret {
            Some(r) if is_async => Some(self.async_inner(r, self.ast().ty(f.ret.unwrap()).span)),
            r => r,
        };
        let exp_ret = exp_fn.as_ref().map(|(_, r)| *r).filter(|&r| r != VOID && !self.types.mentions(r, &self.infer_free));
        let ret = declared_ret.or(exp_ret);
        // `(): T throws E => ...` declares what it throws, as a function declaration does.
        let declared_throws = f.throws.map(|t| self.resolve_type(t, &tscope));
        if let (Some(t), Some(te)) = (declared_throws, f.throws) {
            let span = self.ast().ty(te).span;
            self.check_throws_type(t, span);
        }
        let wild = exp_throws == ERROR;
        let exp_throws = declared_throws.unwrap_or(exp_throws);
        let exp_fn_ty = exp.and_then(|e| self.flat_members(e).into_iter().find(|&m| matches!(self.types.get(m), Ty::Func(..))));
        let fcx = self.fcx.last_mut().unwrap();
        // A test's body can throw (an uncaught error fails the test); other closures only when
        // their function type says so.
        let test_body = (fcx.test_body && fcx.frames.is_empty()) || test_cb;
        // (an arrow with no function type expected, `const f = () => ...` or a nested
        // function, throws what its body does, as a function declaration)
        let can_throw = test_body || exp_throws != NEVER || is_async || exp_fn.is_none();
        let decl = if exp_throws != NEVER && exp_throws != ERROR { Some(exp_throws) } else { None };
        let mut frame = Frame::new(declared_ret, None, can_throw, decl);
        frame.is_async = is_async;
        frame.exp_fn = exp_fn_ty;
        fcx.frames.push(frame);
        fcx.scopes.push(Vec::new());
        // (an unannotated parameter or result takes the sites of the context's function type)
        let context_sites = self.spec_flow.last().cloned().unwrap_or_default();
        let m = self.cur;
        let mut first = None;
        for (p, fp) in f.params.iter().zip(&params) {
            let kind = if p.inout { LocalKind::Inout } else { LocalKind::Param };
            let id = self.declare(p.name, fp.ty, kind, p.span, None, false);
            first.get_or_insert(id);
            if self.spec.on {
                self.fcx().locals[id].sites = match p.ty {
                    Some(t) => self.type_sites(m, t),
                    None => context_sites.clone(),
                };
            }
        }
        if self.spec.on {
            let ret_sites = match f.ret {
                Some(t) => self.type_sites(m, t),
                None => context_sites.clone(),
            };
            self.fcx().frames.last_mut().unwrap().ret_sites = ret_sites;
        }
        // A `routes` handler: its request's `params` are typed from the route.
        if let (Some(key), Some(p0), Some(fp0)) = (self.route_key, f.params.first(), params.first())
            && self.is_route_request(fp0.ty)
            && let Some((id, _)) = self.lookup(p0.name)
        {
            self.fcx().route_locals.push((id, key));
        }
        let body_ty = match &f.body {
            ArrowBody::Expr(x) => {
                let ret_sites = self.fcx().frames.last().unwrap().ret_sites.clone();
                // (`() => JSON.parse(text)` where nothing reads its result: checked to be JSON)
                let parse = matches!(&self.ast().expr(*x).kind, ExprKind::Call { callee, .. } if matches!(&self.ast().expr(*callee).kind, ExprKind::Member { obj, name, .. } if self.name(*name) == "parse" && matches!(self.ast().expr(*obj).kind, ExprKind::Ident(j) if self.name(j) == "JSON")));
                let void_exp = exp_fn.as_ref().is_some_and(|(_, r)| *r == VOID);
                let ret = if parse && void_exp && declared_ret.is_none() { Some(VOID) } else { ret };
                let t = self.flow(ret_sites.clone(), |c| c.expr(*x, ret));
                // `async () => promise`: its value (`async () => await promise`, see `rewrite`)
                let t = if is_async && matches!(self.types.get(t), Ty::Promise(..)) && !ret.is_some_and(|r| self.flat_members(r).iter().any(|&m| matches!(self.types.get(m), Ty::Promise(..)))) {
                    let m = self.cur;
                    self.rewrites.awaits.push((m, *x));
                    match *self.types.get(t) {
                        Ty::Promise(v, _) => v,
                        _ => t,
                    }
                } else {
                    t
                };
                // (a value where a `boolean` is wanted, a predicate `o => o.note`: its truthiness)
                let truthy = declared_ret.is_none() && exp_ret == Some(BOOL) && !matches!(self.types.get(t), Ty::Bool | Ty::BoolLit(_) | Ty::Error | Ty::Void | Ty::Never) && self.printable(t);
                if truthy {
                    let m = self.cur;
                    self.rewrites.truthy.push((m, *x));
                    BOOL
                } else if let Some(r) = declared_ret {
                    let s = self.ast().expr(*x).span;
                    self.flow(ret_sites, |c| c.expect_assignable(t, r, s, None));
                    r
                } else if let Some(r) = exp_ret {
                    if self.assignable(t, r) { r } else { t }
                } else {
                    t
                }
            }
            ArrowBody::Block(b) => {
                if declared_ret.is_none() {
                    self.fcx().frames.last_mut().unwrap().ret = exp_ret;
                }
                self.stmt(*b);
                let frame_ret = self.fcx().frames.last().unwrap().ret;
                match frame_ret {
                    Some(r) => {
                        if r != VOID && r != ERROR && !self.always_returns(*b) && !self.types.has_undefined(r) && !self.fcx.last().unwrap().reported_nonexhaustive {
                            self.report(Diagnostic::new("F0101", span, "arrow function doesn't return a value on every path"));
                        }
                        r
                    }
                    None => {
                        let mut all = std::mem::take(&mut self.fcx().frames.last_mut().unwrap().returns);
                        if self.arrow_probe == Some(span) {
                            all.retain(|&t| !self.types.mentions_error(t));
                        }
                        if all.is_empty() || all.iter().all(|&t| t == UNDEFINED || t == VOID) {
                            VOID
                        } else {
                            if !self.always_returns(*b) {
                                all.push(UNDEFINED);
                            }
                            self.types.union(&all)
                        }
                    }
                }
            }
        };
        let guard = match (first, f.params.first(), params.first()) {
            (Some(id), Some(p0), Some(fp0)) if !p0.inout && !is_async => self.guard_of(f, id, p0.name, fp0.ty, body_ty, &tscope),
            _ => None,
        };
        let fcx = self.fcx.last_mut().unwrap();
        fcx.scopes.pop();
        let frame = fcx.frames.pop().unwrap();
        let thrown = match declared_throws {
            Some(t) => t,
            None if test_body || frame.thrown.is_empty() => NEVER,
            None => self.types.union(&frame.thrown),
        };
        if wild && thrown != NEVER && !is_async {
            self.cb_throws.push(thrown);
        }
        if is_async {
            let p = self.types.promise(body_ty, thrown);
            return (self.types.func(fn_params, p), None);
        }
        (self.types.func_throws(fn_params, body_ty, thrown), guard)
    }

    /// (see `arrow_guarded`; the parameter `id` is still in scope)
    fn guard_of(&mut self, f: &ArrowFn, id: LocalId, name: Sym, ty: TyId, body_ty: TyId, tscope: &[(Sym, TyId)]) -> Option<TyId> {
        if let Some(rt) = f.ret {
            let crate::ast::TypeExprKind::Predicate(pn, t) = self.ast().ty(rt).kind else { return None };
            if pn != name {
                return None;
            }
            let g = self.resolve_type(t, tscope);
            return Some(self.guarded(ty, g));
        }
        if body_ty != BOOL {
            return None;
        }
        let x = match &f.body {
            ArrowBody::Expr(x) => *x,
            ArrowBody::Block(b) => match &self.ast().stmt(*b).kind {
                crate::ast::StmtKind::Block(ss) if ss.len() == 1 => match &self.ast().stmt(ss[0]).kind {
                    crate::ast::StmtKind::Return(Some(x)) => *x,
                    _ => return None,
                },
                _ => return None,
            },
        };
        let of = |ns: &[Narrowing]| ns.iter().find_map(|n| match n {
            Narrowing::Local(l, t) if *l == id => Some(*t),
            _ => None,
        });
        let yes = self.narrowings(x, true);
        let no = self.narrowings(x, false);
        let (yes, no) = (of(&yes)?, of(&no)?);
        let (ym, nm) = (self.flat_members(yes), self.flat_members(no));
        (yes != ty && !ym.iter().any(|m| nm.contains(m))).then_some(yes)
    }

    /// The members of `ty` a guard for `g` lets through (`g` itself when none is one).
    pub(super) fn guarded(&mut self, ty: TyId, g: TyId) -> TyId {
        let fits: Vec<TyId> = self.flat_members(ty).into_iter().filter(|&m| self.assignable(m, g)).collect();
        if fits.is_empty() { g } else { self.types.union(&fits) }
    }

    /// `await e`: the value of a promise (a rejection throws), or `e` itself if it isn't one.
    fn await_expr(&mut self, e: ExprId, x: ExprId, exp: Option<TyId>, span: Span) -> TyId {
        if !self.fcx.last().and_then(|f| f.frames.last()).is_some_and(|f| f.is_async) {
            self.report(
                Diagnostic::new("T0851", span, "`await` is only allowed in an async function (or at the top level of a script)")
                    .note("instead", "mark the enclosing function `async`; it then returns a `Promise`"),
            );
        }
        let exp_p = exp.map(|t| self.types.promise(t, NEVER));
        let t = self.expr(x, exp_p);
        if t == JS {
            // a JavaScript promise (or any value): rejects with a JsError
            let err = self.builtin_class("JsError");
            let text = format!("await {}", self.src(self.ast().expr(x).span));
            self.on_throw(err, span, Some(&text), Some(e));
            return JS;
        }
        let mut values = Vec::new();
        let mut errors = Vec::new();
        for m in self.flat_members(t) {
            match *self.types.get(m) {
                Ty::Promise(v, err) => {
                    values.push(v);
                    if err != NEVER {
                        errors.push(err);
                    }
                }
                _ => values.push(m),
            }
        }
        if !errors.is_empty() {
            let err = self.types.union(&errors);
            let text = format!("await {}", self.src(self.ast().expr(x).span));
            self.on_throw(err, span, Some(&text), Some(e));
        }
        self.types.union(&values)
    }

    /// `new Promise((resolve, reject) => { ... })`. The value type comes from `new Promise<T>`,
    /// the expected type, or else is `void` (`await new Promise((r) => setTimeout(r, 10))`). It can
    /// reject (with an `Error`) only if the executor takes a `reject` parameter.
    fn new_promise(&mut self, e: ExprId, targs: &[TyId], args: &[Arg], exp: Option<TyId>, span: Span) -> TyId {
        let from_exp = exp.and_then(|t| {
            self.flat_members(t).into_iter().find_map(|m| match *self.types.get(m) {
                Ty::Promise(v, _) => Some(v),
                _ => None,
            })
        });
        let value = targs.first().copied().or(from_exp).unwrap_or(VOID);
        let err = self.error_class();
        let [arg] = args else {
            self.report(Diagnostic::new("T0201", span, format!("`new Promise` takes 1 argument (the executor), found {}", args.len())).note("example", "`new Promise((resolve) => setTimeout(resolve, 10))`"));
            for a in args {
                self.expr(a.expr, None);
            }
            return self.types.promise(value, err);
        };
        let rejects = match &self.ast().expr(arg.expr).kind {
            ExprKind::Arrow(f) if f.params.len() < 2 => NEVER,
            _ => err,
        };
        let optional = value == VOID || value == UNDEFINED;
        let resolve = self.types.func(vec![FnParam { ty: value, inout: false, optional }], VOID);
        let reject = self.types.func(vec![FnParam { ty: err, inout: false, optional: false }], VOID);
        let executor = self.types.func(vec![FnParam { ty: resolve, inout: false, optional: false }, FnParam { ty: reject, inout: false, optional: false }], VOID);
        let t = self.expr(arg.expr, Some(executor));
        let s = self.ast().expr(arg.expr).span;
        self.expect_assignable(t, executor, s, Some("the executor".to_string()));
        self.rec_call(e, Callee::NewPromise);
        self.types.promise(value, rejects)
    }

    /// The string `e` always is: a literal, or a field of a module constant written as one
    /// (`Color.Red` of an enum).
    fn const_str(&self, e: ExprId) -> Option<Sym> {
        match &self.ast().expr(e).kind {
            ExprKind::Str(s) => Some(*s),
            ExprKind::Paren(x) => self.const_str(*x),
            ExprKind::Member { obj, name, optional: false, .. } => {
                let ExprKind::Ident(o) = self.ast().expr(*obj).kind else { return None };
                if self.lookup(o).is_some() {
                    return None;
                }
                let Some(Decl::Const(m, i)) = self.scopes[self.cur as usize].values.get(&o).map(|d| d.0) else { return None };
                let ItemKind::Const { init, mutable: false, .. } = &self.modules[m as usize].ast.items[i as usize].kind else { return None };
                let ast = &self.modules[m as usize].ast;
                let ExprKind::Object(fs) = &ast.expr(*init).kind else { return None };
                let f = fs.iter().find(|f| f.name == *name)?;
                match ast.expr(f.value).kind {
                    ExprKind::Str(s) => Some(s),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn new_expr(&mut self, e: ExprId, callee: ExprId, type_args: &[crate::ast::TypeId], args: &[Arg], exp: Option<TyId>, span: Span) -> TyId {
        // `new Q<number>()` of a parameter given a class: what its constructor type says
        if let ExprKind::Ident(s) = self.ast().expr(callee).kind
            && let Some((_, qt)) = self.lookup(s)
            && let Ty::Param(p) = *self.types.get(qt)
            && let Some(sig) = self.gparam(p).ctor.clone()
        {
            self.ident(callee, s, self.ast().expr(callee).span);
            let names = (0..sig.params.len()).map(|i| format!("arg{i}")).collect();
            let throws = self.error_class();
            let cs = CallSig { tparams: sig.tparams.clone(), params: sig.params.clone(), names, rest: None, ret: sig.ret, throws };
            self.rec_call(e, Callee::NewOf(p, sig.tparams.clone()));
            self.pending_call = Some(e);
            let desc = format!("new {}", self.name(s));
            return self.call_sig(&cs, &desc, type_args, args, exp, span);
        }
        // `{ [Color.Red]: v }` where a record type is expected: a record literal, its keys string
        // literals (rewritten as `{ red: v }`)
        if self.ast().keyed_objects.contains(&e)
            && let Some(rt) = exp.map(|t| self.types.without_undefined(t)).filter(|&t| matches!(self.types.get(t), Ty::Record(_)))
            && let ExprKind::Array(entries) = &self.ast().expr(args[0].expr).kind
        {
            let entries = entries.clone();
            let mut names = Vec::new();
            for &entry in &entries {
                let ExprKind::Array(kv) = &self.ast().expr(entry).kind else { break };
                let k = kv[0];
                let kt = self.expr(k, None);
                if let Some(s) = self.const_str(k) {
                    names.push(s);
                    continue;
                }
                match self.flat_members(kt).as_slice() {
                    [m] => match *self.types.get(*m) {
                        Ty::StrLit(s) => names.push(s),
                        _ => break,
                    },
                    _ => break,
                }
            }
            if names.len() == entries.len() {
                self.rewrites.records.push((self.cur, e, names));
                return rt;
            }
        }
        // `new X(...)` on a JavaScript constructor (`import { X } from "pkg"`, `new pkg.X()`)
        let js_callee = match self.ast().expr(callee).kind {
            ExprKind::Ident(s) => match self.lookup(s) {
                Some((_, t)) => t == JS,
                None => matches!(self.scopes[self.cur as usize].values.get(&s).map(|d| d.0), Some(Decl::Npm(..))),
            },
            _ => true,
        };
        if js_callee {
            let t = self.expr(callee, None);
            if t == JS || t == ERROR {
                let text = format!("new {}", self.src(self.ast().expr(callee).span));
                return self.js_call(e, None, &text, args, span, true);
            }
        }
        let ExprKind::Ident(s) = self.ast().expr(callee).kind else { return ERROR };
        // (a `WeakMap` / `WeakSet`: a `Map` / `Set` of its keys by identity; with no collector to
        // drop them, the difference can't be seen)
        let name = match self.name(s) {
            "WeakMap" => "Map".to_string(),
            "WeakSet" => "Set".to_string(),
            n => n.to_string(),
        };
        let tscope = self.tscope();
        let targs: Vec<TyId> = type_args.iter().map(|&t| self.resolve_type(t, &tscope)).collect();
        let from_exp = |c: &mut Self, pick: &dyn Fn(&Ty) -> bool| exp.and_then(|e| c.flat_members(e).into_iter().find(|&m| pick(c.types.get(m))));
        match name.as_str() {
            "Map" => {
                let mut kv = if targs.len() == 2 {
                    Some((targs[0], targs[1]))
                } else {
                    from_exp(self, &|t| matches!(t, Ty::Map(..))).and_then(|t| match *self.types.get(t) {
                        Ty::Map(k, v) => Some((k, v)),
                        _ => None,
                    })
                };
                // `new Map([["a", 1], ...])` or `new Map(pairs)`: entries `[key, value]`
                if let Some(a) = args.first() {
                    if kv.is_none()
                        && let ExprKind::Array(rows) = &self.ast().expr(a.expr).kind
                        && !rows.is_empty()
                        && rows.iter().all(|&r| matches!(&self.ast().expr(r).kind, ExprKind::Array(xs) if xs.len() == 2))
                    {
                        let (mut ks, mut vs) = (Vec::new(), Vec::new());
                        for &r in rows.clone().iter() {
                            let ExprKind::Array(xs) = &self.ast().expr(r).kind else { continue };
                            let (x0, x1) = (xs[0], xs[1]);
                            let k = self.expr(x0, None);
                            ks.push(self.types.widen(k));
                            let v = self.expr(x1, None);
                            vs.push(self.types.widen(v));
                        }
                        kv = Some((self.types.union(&ks), self.types.union(&vs)));
                    }
                    let hint = kv.map(|(k, v)| {
                        let e = self.tuple_ty(&[k, v]);
                        self.types.array(e)
                    });
                    let t = self.expr(a.expr, hint);
                    let s = self.ast().expr(a.expr).span;
                    match (hint, *self.types.get(t)) {
                        // `new Map(other)`: a copy of another map
                        (_, Ty::Map(k2, v2)) => match kv {
                            Some((k, v)) => {
                                let want = self.types.intern(Ty::Map(k, v));
                                self.expect_assignable(t, want, s, Some("the map".into()));
                            }
                            None => kv = Some((k2, v2)),
                        },
                        (Some(h), _) => {
                            self.expect_assignable(t, h, s, Some("the entries".into()));
                        }
                        (None, Ty::Array(e)) if self.tuple_of(e).is_some_and(|f| f.len() == 2) => {
                            let f = self.tuple_of(e).unwrap();
                            kv = Some((f[0].ty, f[1].ty));
                        }
                        (_, Ty::Error) => {}
                        _ => {
                            let shown = self.show(t);
                            self.report(Diagnostic::new("T0001", s, format!("`new Map(...)` takes `[key, value]` entries, found `{shown}`")));
                        }
                    }
                }
                if let Some((k, v)) = kv {
                    return self.types.intern(Ty::Map(k, v));
                }
                self.report(Diagnostic::new("T0301", span, "can't infer the key and value types of this map").note("instead", "`new Map<string, int>()`, or annotate the binding"));
                ERROR
            }
            "Set" => {
                let mut elem = targs.first().copied().or_else(|| from_exp(self, &|t| matches!(t, Ty::Set(_))).and_then(|t| match self.types.get(t) {
                    Ty::Set(e) => Some(*e),
                    _ => None,
                }));
                if let Some(a) = args.first() {
                    let hint = elem.map(|e| self.types.array(e));
                    let t = self.expr(a.expr, hint);
                    match *self.types.get(t) {
                        Ty::Array(e) => elem = elem.or(Some(self.types.widen(e))),
                        // (a string's characters)
                        Ty::Str | Ty::StrLit(_) => elem = elem.or(Some(STR)),
                        Ty::Error => {}
                        _ => {
                            let msg = format!("`new Set(...)` takes an array or a string, found `{}`", self.show(t));
                            let s = self.ast().expr(a.expr).span;
                            self.report(Diagnostic::new("T0001", s, msg));
                        }
                    }
                }
                match elem {
                    Some(e) => self.types.intern(Ty::Set(e)),
                    None => {
                        self.report(Diagnostic::new("T0301", span, "can't infer the element type of this set").note("instead", "`new Set<string>()`"));
                        ERROR
                    }
                }
            }
            n if n == "Array" || builtins::typed_array_elem(n).is_some() => {
                // `new Uint8Array(n)`, `new Float64Array(xs)`, `new Array<number>(n)`: an array of
                // n zeros (or the element type's empty value), or a converted copy
                let typed = builtins::typed_array_elem(n);
                let elem = typed.or(targs.first().copied()).or_else(|| from_exp(self, &|t| matches!(t, Ty::Array(_))).and_then(|t| match self.types.get(t) {
                    Ty::Array(e) => Some(*e),
                    _ => None,
                }));
                if n == "Array" && args.len() > 1 {
                    let elems: Vec<ExprId> = args.iter().map(|a| a.expr).collect();
                    let hint = elem.map(|e| self.types.array(e));
                    return self.array_lit(&elems, hint, span);
                }
                if let Some(a) = args.first() {
                    // (elements are numbers, converted as they're stored: `new Uint8Array([300])` wraps)
                    // (a typed array's: numbers, `new Uint8Array([0.5, x / 2])`, converted as they're
                    // stored, whichever round this is)
                    let hint = elem.map(|e| self.types.array(if typed.is_some() { F64 } else { promoted(e) }));
                    let t = self.expr(a.expr, if typed.is_some() { hint } else { Some(INT) });
                    let s = self.ast().expr(a.expr).span;
                    let base = self.types.without_undefined(t);
                    match *self.types.get(base) {
                        _ if self.types.is_int(t) || self.types.is_float(t) => {}
                        Ty::Array(e) | Ty::Set(e) if typed.is_some() && self.types.is_numeric(e) => {}
                        Ty::Error => {}
                        _ => {
                            let what = if typed.is_some() { "a length or an array of numbers" } else { "a length (an integer)" };
                            let shown = self.show(t);
                            self.report(Diagnostic::new("T0001", s, format!("`new {n}(...)` takes {what}, found `{shown}`")));
                        }
                    }
                }
                match elem {
                    Some(el) if typed.is_some() || self.has_default(el) => self.types.array(el),
                    // (its holes are the `undefined` its elements may be)
                    Some(el) if n == "Array" && (el == UNKNOWN || self.types.without_undefined(el) != el) => self.types.array(el),
                    // (filled as it's made, `Array(n).fill(null)`: no hole is seen)
                    Some(el) if n == "Array" && args.len() == 1 && self.filled_at_once(e) => self.types.array(el),
                    Some(el) if n == "Array" && args.len() == 1 && super::stmt::filled_by_next_loop(self.ast(), e) => {
                        if let Some(f) = self.facts_mut() {
                            f.filled_arrays.insert(e);
                        }
                        self.types.array(el)
                    }
                    Some(e) => {
                        let shown = self.show(e);
                        self.report(
                            Diagnostic::new("T0301", span, format!("`new Array(n)` of `{shown}` would hold `undefined`s"))
                                .note("instead", format!("`Array.from({{ length: n }}, () => value)`, or `new Array<{shown}>(n).fill(value)`")),
                        );
                        ERROR
                    }
                    // (`[...Array(n).keys()]`: only its indexes are read)
                    None if n == "Array" && args.len() == 1 && self.receiver_of(e, "keys") => self.types.array(UNDEFINED),
                    None => {
                        self.report(Diagnostic::new("T0301", span, "can't infer the element type of this array").note("instead", "`new Array<number>(n)`, `new Array(n).fill(0)`, or annotate the binding"));
                        ERROR
                    }
                }
            }
            "Promise" if !self.scopes[self.cur as usize].values.contains_key(&s) => self.new_promise(e, &targs, args, exp, span),
            _ => {
                if let Some(Decl::Class(c)) = self.scopes[self.cur as usize].values.get(&s).map(|d| d.0) {
                    self.rec_ident(callee, IdentFact::Class(c));
                    self.resolve_class(c);
                    let info = self.classes[c as usize].clone();
                    if info.is_abstract {
                        self.report(Diagnostic::new("T0804", span, format!("`{name}` is abstract, so it can't be instantiated")).note("instead", "create an instance of a concrete subclass"));
                    }
                    let names = info.ctor.param_names.iter().map(|&p| self.name(p).to_string()).collect();
                    let throws = self.ctor_throws(c);
                    let cs = CallSig { tparams: info.params.clone(), params: info.ctor.params.clone(), names, rest: None, ret: info.this_ty, throws };
                    self.rec_call(e, Callee::New(c));
                    self.pending_call = Some(e);
                    let sites = self.ctor_sites(c);
                    self.set_call_spec((sites, Vec::new()));
                    let desc = format!("new {name}");
                    let t = self.call_sig(&cs, &desc, type_args, args, exp, span);
                    let _ = targs;
                    return t;
                }
                let mut d = Diagnostic::new("N0001", self.ast().expr(callee).span, format!("unknown class `{name}`"));
                let classes: Vec<String> = self.scopes[self.cur as usize].values.iter().filter(|(_, d)| matches!(d.0, Decl::Class(_))).map(|(s, _)| self.name(*s).to_string()).collect();
                let sug = similar(&name, classes.iter().map(|s| s.as_str()));
                if let Some(first) = sug.first() {
                    d = d.note("did you mean", sug.join(", ")).fix(Applicability::Maybe, format!("use `{first}`"), self.ast().expr(callee).span, first.to_string());
                }
                if self.scopes[self.cur as usize].types.contains_key(&s) {
                    d = d.note("note", format!("`{name}` is a type; records are created with object literals: `{{ ... }}`"));
                }
                self.report(d);
                for a in args {
                    self.expr(a.expr, None);
                }
                ERROR
            }
        }
    }

    // ---------------------------------------------------------------- calls

    #[allow(clippy::too_many_arguments)]
    fn call(&mut self, e: ExprId, callee: ExprId, type_args: &[crate::ast::TypeId], args: &[Arg], optional: bool, exp: Option<TyId>, span: Span) -> TyId {
        let cnode = self.ast().expr(callee);
        match &cnode.kind {
            ExprKind::Ident(s) if self.lookup(*s).is_none() => {
                let text = self.name(*s).to_string();
                match self.scopes[self.cur as usize].values.get(s).map(|d| d.0) {
                    Some(Decl::Fn(m, i)) => {
                        let Some(sig) = self.fn_sig(m, i) else {
                            self.report(Diagnostic::new("T0303", cnode.span, format!("`{text}` is recursive, so it needs an explicit return type")));
                            for a in args {
                                self.expr(a.expr, None);
                            }
                            return ERROR;
                        };
                        let (fixed, rest) = sig.call_params(&self.types);
                        let cs = CallSig { tparams: sig.tparams, params: fixed, names: sig.param_names.iter().map(|&p| self.name(p).to_string()).collect(), rest, ret: sig.ret, throws: sig.throws };
                        self.rec_call(e, Callee::Fn(m, i));
                        self.pending_call = Some(e);
                        let sites = self.fn_item_sites(m, i);
                        self.set_call_spec(sites);
                        let t = self.call_sig(&cs, &text, type_args, args, exp, span);
                        // (overloads: the result of the first whose parameters take the arguments)
                        return self.overload_ret(m, i, args, t);
                    }
                    // `Big(x)`: a class standing in for an npm package's constructor function,
                    // callable without `new` as the package's is
                    Some(Decl::Class(c)) if self.modules[self.classes[c as usize].module as usize].std => {
                        return self.new_expr(e, callee, type_args, args, exp, span);
                    }
                    Some(_) => {}
                    None if text == "__rest" => return self.object_rest(e, args, span),
                    None => {
                        self.rec_call(e, Callee::Builtin { ns: None, name: *s });
                        self.pending_call = Some(e);
                        if let Some(t) = self.global_call(&text, cnode.span, type_args, args, exp, span) {
                            return t;
                        }
                    }
                }
            }
            ExprKind::Member { obj, name, name_span, optional: mopt } => {
                let (obj, name, name_span, mopt) = (*obj, *name, *name_span, *mopt);
                let mtext = self.name(name).to_string();
                if let ExprKind::Ident(s) = self.ast().expr(obj).kind
                    && self.lookup(s).is_none() {
                        match self.scopes[self.cur as usize].values.get(&s).map(|d| d.0) {
                            Some(Decl::Ns(m)) => {
                                let scope = &self.scopes[m as usize];
                                if let Some(Decl::Fn(fm, fi)) = scope.values.get(&name).filter(|_| scope.exported_values.contains(&name)).map(|d| d.0) {
                                    let Some(sig) = self.fn_sig(fm, fi) else { return ERROR };
                                    let (fixed, rest) = sig.call_params(&self.types);
                        let cs = CallSig { tparams: sig.tparams, params: fixed, names: sig.param_names.iter().map(|&p| self.name(p).to_string()).collect(), rest, ret: sig.ret, throws: sig.throws };
                                    let desc = format!("{}.{mtext}", self.name(s));
                                    self.rec_call(e, Callee::Fn(fm, fi));
                                    self.pending_call = Some(e);
                                    let sites = self.fn_item_sites(fm, fi);
                                    self.set_call_spec(sites);
                                    return self.call_sig(&cs, &desc, type_args, args, exp, span);
                                }
                                // `ns.f(x)` of an exported constant holding a function: a value call
                                if matches!(scope.values.get(&name).filter(|_| scope.exported_values.contains(&name)).map(|d| d.0), Some(Decl::Const(..))) {
                                    let t = self.expr(callee, None);
                                    let text = self.src(self.ast().expr(callee).span).to_string();
                                    self.rec_call(e, Callee::Value);
                                    self.pending_call = Some(e);
                                    self.set_call_spec_from(callee);
                                    return self.call_value(t, &text, args, exp, span);
                                }
                            }
                            None => {
                                let ns = self.name(s).to_string();
                                if self.is_ns(&ns) {
                                    self.rec_call(e, Callee::Builtin { ns: Some(s), name });
                                    self.pending_call = Some(e);
                                    return self.builtin_ns_call(&ns, &mtext, name_span, args, exp, span);
                                }
                            }
                            Some(Decl::Class(c)) => {
                                self.resolve_class(c);
                                let info = self.classes[c as usize].clone();
                                let cls = self.class_names[c as usize].clone();
                                let Some(mm) = info.statics.iter().find(|m| m.name == name).cloned() else {
                                    self.static_member(obj, c, name, name_span);
                                    for a in args {
                                        self.expr(a.expr, None);
                                    }
                                    return ERROR;
                                };
                                self.check_visible(mm.owner, mm.vis, name, name_span);
                                let ret = self.method_ret(&mm);
                                let names = mm.param_names.iter().map(|&p| self.name(p).to_string()).collect();
                                let throws = self.method_throws(&mm);
                                let (params, rest) = self.method_call_params(&mm);
                                let cs = CallSig { tparams: mm.tparams.clone(), params, names, rest, ret, throws };
                                self.rec_call(e, Callee::StaticMethod(c, mm.member));
                                self.pending_call = Some(e);
                                let sites = self.member_sites(mm.owner, mm.member);
                                self.set_call_spec(sites);
                                return self.call_sig(&cs, &format!("{cls}.{mtext}"), type_args, args, exp, span);
                            }
                            _ => {}
                        }
                    }
                if matches!(self.ast().expr(obj).kind, ExprKind::Super) {
                    return self.super_method_call(e, name, name_span, type_args, args, exp, span);
                }
                // `new Array(n).fill(v)`: the array's type is the value's
                let mut recv_hint = None;
                if mtext == "fill"
                    && let ExprKind::New { callee: nc, type_args: nta, .. } | ExprKind::Call { callee: nc, type_args: nta, .. } = &self.ast().expr(obj).kind
                    && nta.is_empty()
                    && matches!(self.ast().expr(*nc).kind, ExprKind::Ident(s) if self.name(s) == "Array")
                    && let Some(a) = args.first()
                {
                    let exp_elem = exp.and_then(|x| self.flat_members(x).into_iter().find_map(|m| match *self.types.get(m) {
                        Ty::Array(t) => Some(t),
                        _ => None,
                    }));
                    // (`const m: number[] = Array(n).fill(1.5)` with `number` taken as `int`: the
                    // float's array, so the annotation it's given to is what turns `f64`)
                    let float_fill = exp_elem == Some(INT) && self.spec.on && matches!(self.ast().expr(a.expr).kind, ExprKind::Float(_) | ExprKind::Ident(_) | ExprKind::Unary(..)) && {
                        let marks = self.diags.len();
                        let t = self.expr(a.expr, None);
                        self.diags.truncate(marks);
                        self.types.is_float(t)
                    };
                    let exp_elem = if float_fill { None } else { exp_elem };
                    let vt = match exp_elem {
                        Some(t) if t != ERROR => t,
                        _ => {
                            let t = self.expr(a.expr, None);
                            self.types.widen(t)
                        }
                    };
                    recv_hint = Some(self.types.array(vt));
                }
                self.want_optional = mopt;
                let recv = self.expr(obj, recv_hint);
                if recv == ERROR {
                    for a in args {
                        self.expr(a.expr, None);
                    }
                    return ERROR;
                }
                if self.types.without_undefined(recv) == JS {
                    let desc = format!("{}.{mtext}", self.src(self.ast().expr(obj).span));
                    return self.js_call(e, Some(name), &desc, args, span, false);
                }
                let base = if mopt {
                    self.types.without_nullish(recv)
                } else {
                    if self.types.has_nullish(recv) {
                        let d = self.possibly_undefined(obj, span);
                        self.report(d);
                    }
                    self.types.without_undefined(recv)
                };
                // `xs.toSorted()`, `xs.findLast(f)`, `a.union(b)` (ES2023, ES2025): the prelude's
                // `__toSorted(xs)` and so on
                let prelude_method = match (*self.types.get(base), mtext.as_str(), args.len()) {
                    (Ty::Array(_), "toSorted", 0 | 1) => Some("__toSorted"),
                    (Ty::Array(_), "toReversed", 0) => Some("__toReversed"),
                    (Ty::Array(_), "with", 2) => Some("__with"),
                    (Ty::Array(_), "findLast", 1) => Some("__findLast"),
                    (Ty::Array(_), "findLastIndex", 1) => Some("__findLastIndex"),
                    (Ty::Array(_), "reduceRight", 2) => Some("__reduceRight"),
                    (Ty::Array(_), "reduceRight", 1) => Some("__reduceRight1"),
                    (Ty::Set(_), "union", 1) => Some("__setUnion"),
                    (Ty::Set(_), "intersection", 1) => Some("__setIntersection"),
                    (Ty::Set(_), "difference", 1) => Some("__setDifference"),
                    (Ty::Set(_), "symmetricDifference", 1) => Some("__setSymmetricDifference"),
                    (Ty::Set(_), "isSubsetOf", 1) => Some("__setIsSubsetOf"),
                    (Ty::Set(_), "isSupersetOf", 1) => Some("__setIsSupersetOf"),
                    (Ty::Set(_), "isDisjointFrom", 1) => Some("__setIsDisjointFrom"),
                    _ => None,
                };
                if let Some(target) = prelude_method
                    && args.iter().all(|a| a.by_ref.is_none() && !matches!(self.ast().expr(a.expr).kind, ExprKind::Spread(_)))
                    && let Some(f) = self.interner.lookup(target)
                {
                    self.rewrites.calls.push((self.cur, e, f));
                    for a in args {
                        self.expr(a.expr, None);
                    }
                    return ERROR;
                }
                // `xs.flat(2)`, `xs.flat(Infinity)`: `xs.flat().flat()`, as many levels as asked (or
                // as its type has)
                if let Ty::Array(_) = *self.types.get(base)
                    && mtext == "flat"
                    && args.len() == 1
                {
                    // (how deep its arrays go, through unions: `[1, [2, [3]]]`)
                    fn depth(types: &Types, t: TyId, budget: u32) -> u32 {
                        if budget == 0 {
                            return 0;
                        }
                        match *types.get(t) {
                            Ty::Array(x) => 1 + depth(types, x, budget - 1),
                            Ty::Union(ms) => types.tys(ms).iter().map(|&m| depth(types, m, budget)).max().unwrap_or(0),
                            _ => 0,
                        }
                    }
                    let nested = depth(&self.types, base, 32).saturating_sub(1);
                    let depth = match &self.ast().expr(args[0].expr).kind {
                        ExprKind::Int(k) => Some((*k).min(nested as u64) as u32),
                        ExprKind::Ident(i) if self.name(*i) == "Infinity" => Some(nested),
                        _ => None,
                    };
                    if let Some(d) = depth
                        && let (Some(flat), Some(slice)) = (self.interner.lookup("flat"), self.interner.lookup("slice"))
                    {
                        self.rewrites.flat_chains.push((self.cur, e, d, flat, slice));
                        return ERROR;
                    }
                    self.report(Diagnostic::new("U0011", span, "`flat` takes a literal depth (a number, or `Infinity`)"));
                    return ERROR;
                }
                // `o.hasOwnProperty(k)` of a record or a dictionary: `k in o`
                if mtext == "hasOwnProperty"
                    && args.len() == 1
                    && matches!(self.types.get(base), Ty::Record(_) | Ty::Map(..) | Ty::Unknown | Ty::Rec(..))
                {
                    self.rewrites.has_own.push((self.cur, e, obj, args[0].expr));
                    return ERROR;
                }
                // `p.then(f)`, `p.catch(g)`, `p.finally(h)`: the prelude's `__then(p, f)` and so on
                if let Ty::Promise(v, _) = *self.types.get(base)
                    && matches!(mtext.as_str(), "then" | "catch" | "finally")
                    && (1..=2).contains(&args.len())
                    && args.iter().all(|a| a.by_ref.is_none() && !matches!(self.ast().expr(a.expr).kind, ExprKind::Spread(_)))
                {
                    let target = match (mtext.as_str(), args.len()) {
                        ("then", 1) => {
                            // (a callback returning a promise: its value)
                            let hint = self.types.func(vec![FnParam { ty: v, inout: false, optional: false }], VOID);
                            let ft = self.expr(args[0].expr, Some(hint));
                            let ret = match *self.types.get(ft) {
                                Ty::Func(_, r, _) => Some(r),
                                _ => None,
                            };
                            let promised = ret.is_some_and(|r| self.flat_members(r).iter().any(|&m| matches!(self.types.get(m), Ty::Promise(..))));
                            if promised { "__thenP" } else { "__then" }
                        }
                        ("then", _) => "__then2",
                        ("catch", 1) => "__catch",
                        ("finally", 1) => "__finally",
                        _ => "",
                    };
                    if let Some(f) = self.interner.lookup(target).filter(|_| !target.is_empty()) {
                        self.rewrites.calls.push((self.cur, e, f));
                        return ERROR;
                    }
                }
                // `s.replace(/x/g, y)` and the other string methods given a RegExp: the prelude's
                // `__reReplace(s, re, y)` and so on
                if matches!(*self.types.get(base), Ty::Str | Ty::StrLit(_))
                    && matches!(mtext.as_str(), "replace" | "replaceAll" | "split" | "match" | "matchAll" | "search")
                    && let Some(a0) = args.first()
                    && a0.by_ref.is_none()
                {
                    let marks = self.diags.len();
                    let re = self.builtin_class("RegExp");
                    let t = self.expr(a0.expr, None);
                    if re != ERROR && self.types.without_undefined(t) == re {
                        let is_fn = args.get(1).is_some_and(|a| match self.ast().expr(a.expr).kind {
                            ExprKind::Arrow(_) => true,
                            ExprKind::Ident(s) => match self.lookup(s) {
                                Some(l) => matches!(*self.types.get(l.1), Ty::Func(..)),
                                None => matches!(self.scopes[self.cur as usize].values.get(&s).map(|d| d.0), Some(Decl::Fn(..))),
                            },
                            _ => false,
                        });
                        // (`s.match(/x/g)`: the matches' strings, as JavaScript gives for a global literal)
                        let global_lit = matches!(&self.ast().expr(a0.expr).kind, ExprKind::Call { callee, args: ra, .. }
                            if matches!(self.ast().expr(*callee).kind, ExprKind::Ident(f) if self.name(f) == "__reLiteral")
                            && matches!(ra.get(1).map(|a| &self.ast().expr(a.expr).kind), Some(ExprKind::Str(fl)) if self.name(*fl).contains('g')));
                        // (a callback of the match alone: no groups made for it)
                        let one_param = args.get(1).is_some_and(|a| matches!(&self.ast().expr(a.expr).kind, ExprKind::Arrow(f) if f.params.len() <= 1 && !f.params.iter().any(|p| p.rest)));
                        // (a callback past the groups of a literal: then the match's offset and the
                        // input, `(m, i) => ...`, as JavaScript passes them)
                        let lit_groups = match &self.ast().expr(a0.expr).kind {
                            ExprKind::Call { callee, args: ra, .. } if matches!(self.ast().expr(*callee).kind, ExprKind::Ident(f) if self.name(f) == "__reLiteral") => match ra.first().map(|a| &self.ast().expr(a.expr).kind) {
                                Some(ExprKind::Str(src)) => Some(capture_groups(self.name(*src))),
                                _ => None,
                            },
                            _ => None,
                        };
                        let arrow_params = args.get(1).and_then(|a| match &self.ast().expr(a.expr).kind {
                            ExprKind::Arrow(f) if !f.params.iter().any(|p| p.rest) => Some(f.params.len()),
                            _ => None,
                        });
                        let with_offset = match (lit_groups, arrow_params) {
                            (Some(k), Some(n)) if n > k + 1 && k <= 3 => Some(k),
                            _ => None,
                        };
                        let offset_fn = with_offset.map(|k| format!("__reReplaceFnG{k}"));
                        let target = match mtext.as_str() {
                            "match" if global_lit => "__reMatchGlobal",
                            "replace" | "replaceAll" if is_fn && offset_fn.is_some() => offset_fn.as_deref().unwrap(),
                            "replace" | "replaceAll" if is_fn && one_param => "__reReplaceFn1",
                            "replace" | "replaceAll" if is_fn => "__reReplaceFn",
                            "replace" | "replaceAll" => "__reReplace",
                            "split" => "__reSplit",
                            "match" => "__reMatch",
                            "matchAll" => "__reMatchAll",
                            _ => "__reSearch",
                        };
                        if let Some(f) = self.interner.lookup(target) {
                            self.rewrites.calls.push((self.cur, e, f));
                        }
                        for a in &args[1..] {
                            self.expr(a.expr, None);
                        }
                        let found = self.builtin_class("RegExpExecArray");
                        return match mtext.as_str() {
                            "replace" | "replaceAll" => STR,
                            "split" => self.types.array(STR),
                            "match" if global_lit => {
                                let strs = self.types.array(STR);
                                self.types.union(&[strs, NULL])
                            }
                            "match" => self.types.union(&[found, NULL]),
                            "matchAll" => self.types.array(found),
                            _ => INT,
                        };
                    }
                    self.diags.truncate(marks);
                }
                // `process.stdout.write(s)` / `process.stderr.write(s)`.
                if let Ty::BuiltinNs(s) = *self.types.get(base) {
                    let ns = self.name(s).to_string();
                    self.rec_call(e, Callee::Builtin { ns: Some(s), name });
                    self.pending_call = Some(e);
                    return self.builtin_ns_call(&ns, &mtext, name_span, args, exp, span);
                }
                if let Ty::Expect(subject) = *self.types.get(base) {
                    self.rec_call(e, Callee::Matcher(subject));
                    self.pending_call = Some(e);
                    return self.expect_matcher(subject, &mtext, name_span, args, span);
                }
                // `m.map(f)` on a match, an array in JavaScript: `m.items.map(f)`
                if matches!(*self.types.get(base), Ty::Class(..))
                    && base == self.builtin_class("RegExpExecArray")
                    && self.class_member(base, name).is_none()
                    && builtins::ARRAY_METHODS.contains(&mtext.as_str())
                    && let Some(items) = self.interner.lookup("items")
                {
                    self.rewrites.elem_methods.push((self.cur, callee, items));
                    for a in args {
                        self.expr(a.expr, None);
                    }
                    return ERROR;
                }
                // Methods of class instances.
                if self.class_of(base).is_some()
                    && let Some(super::class::ClassMemberRef::Method(mm)) = self.class_member(base, name)
                {
                    self.check_visible(mm.owner, mm.vis, name, name_span);
                    let ret = self.method_ret(&mm);
                    let names = mm.param_names.iter().map(|&p| self.name(p).to_string()).collect();
                    let throws = self.method_throws(&mm);
                    let (params, rest) = self.method_call_params(&mm);
                                let cs = CallSig { tparams: mm.tparams.clone(), params, names, rest, ret, throws };
                    self.rec_call(e, Callee::ClassMethod { recv: base, name, sup: false });
                    self.pending_call = Some(e);
                    let sites = self.member_sites(mm.owner, mm.member);
                    self.set_call_spec(sites);
                    let cls = self.show(base);
                    let r = self.call_sig(&cs, &format!("{cls}.{mtext}"), type_args, args, exp, span);
                    if let Some(slot) = self.evolving_class(obj) {
                        self.class_evidence(slot, base, name, args);
                    }
                    if self.spec.on && mtext == "json" && self.modules[self.classes[mm.owner as usize].module as usize].std {
                        // (a body parsed as JSON may hold any number)
                        self.spec_poison(r);
                    }
                    return if mopt || optional { self.types.optional(r) } else { r };
                }
                let fm = self.flat_members(base);
                if fm.len() > 1 && fm.iter().any(|&m| matches!(self.class_member(m, name), Some(super::class::ClassMemberRef::Method(_)))) {
                    let shown = self.show(base);
                    self.report(
                        Diagnostic::new("T0817", span, format!("can't call method `{mtext}` on `{shown}`"))
                            .note("why", "each class in the union has its own method")
                            .note("instead", "narrow first (`if (x instanceof C)`), or give the classes a common base class that declares the method"),
                    );
                    for a in args {
                        self.expr(a.expr, None);
                    }
                    return ERROR;
                }
                // A record field holding a function.
                let is_field = self.flat_members(base).iter().all(|&m| self.field_of(m, name).is_some());
                let result = if is_field {
                    let ft = self.member_of(base, name, name_span, span, obj).unwrap_or(ERROR);
                    // (`o.cb ? o.cb(x) : ...`: the field as narrowed there)
                    let ft = match self.target_of(callee).map(|(_, t)| t) {
                        Some(n) if n != ft && self.assignable(n, ft) => n,
                        _ => ft,
                    };
                    if let Some(f) = self.facts_mut() {
                        f.expr_ty[callee as usize] = ft;
                    }
                    // (`o.cb?.(x)`: called when it's there)
                    let ft = if optional { self.types.without_undefined(ft) } else { ft };
                    self.rec_call(e, Callee::Value);
                    self.pending_call = Some(e);
                    self.call_value(ft, &mtext, args, exp, span)
                } else if let mname = if mtext == "reduce" && args.len() == 1 && matches!(self.types.get(base), Ty::Array(_)) { self.reduce_from_first(base, args[0].expr) } else { &mtext }
                    && let Some(ms) = builtins::method(self, base, mname)
                {
                    if ms.mutates {
                        self.check_mutable_root_ext(obj, span, true);
                    }
                    if mtext == "toLocaleString"
                        && let Err(at) = builtins::locale_digits(self.ast(), self.interner, args)
                    {
                        self.report(Diagnostic::new("U0017", at, "`toLocaleString` supports English (`\"en-US\"`) and integer `minimumFractionDigits`/`maximumFractionDigits` options written in place"));
                    }
                    let desc = format!("{}.{mtext}", builtins::type_label(&self.types, base));
                    self.rec_call(e, Callee::Method(base));
                    self.pending_call = Some(e);
                    self.set_call_spec_from(obj);
                    let wrap = matches!(*self.types.get(base), Ty::Array(el) if promoted(el) == INT && el != INT) && matches!(mtext.as_str(), "push" | "unshift" | "fill");
                    let saved = std::mem::replace(&mut self.wrap_store, wrap);
                    // (a built-in's integer parameters take numbers, truncated as JavaScript does)
                    let saved_f = std::mem::replace(&mut self.float_args, true);
                    // (its callbacks may throw: the call then throws what they do)
                    let sig = self.throwing_callbacks(&ms.sig);
                    let mark = self.cb_throws.len();
                    // `xs.push(x)` on an untyped `const xs = []`: its elements are what it's given
                    if matches!(mtext.as_str(), "push" | "unshift")
                        && let Some(slot) = self.evolving_slot(obj)
                    {
                        self.evolve(slot, args);
                    }
                    // `m.set(k, v)` / `s.add(x)` on an untyped `new Map()` / `new Set()`
                    if matches!((mtext.as_str(), args.len()), ("set", 2) | ("add", 1))
                        && let Some(slot) = self.evolving_collection(obj)
                    {
                        let mut tys = Vec::new();
                        for a in args {
                            let mark = self.diags.len();
                            let t = self.expr(a.expr, None);
                            self.diags.truncate(mark);
                            tys.push(self.types.widen(t));
                        }
                        if !tys.iter().any(|&t| t == ERROR || self.types.mentions_error(t)) {
                            let t = if tys.len() == 2 { self.types.intern(Ty::Map(tys[0], tys[1])) } else { self.types.intern(Ty::Set(tys[0])) };
                            self.fcx().evolving[slot].2.push(t);
                        }
                    }
                    let r = self.call_sig(&sig, &desc, type_args, args, exp, span);
                    // `xs.filter((x): x is T => ...)`, `xs.find(isT)`: of what passes
                    let r = match (*self.types.get(base), mtext.as_str(), args) {
                        (Ty::Array(el), "filter" | "find" | "findLast", [a]) => match self.guard_arg(a.expr, el) {
                            Some(g) if mtext == "filter" => self.types.array(g),
                            Some(g) => self.types.optional(g),
                            None => r,
                        },
                        _ => r,
                    };
                    let mut thrown: Vec<TyId> = self.cb_throws.drain(mark..).collect();
                    for a in args {
                        if !matches!(self.ast().expr(a.expr).kind, ExprKind::Arrow(_))
                            && let Some(t) = self.fn_value_throws(a.expr)
                        {
                            thrown.push(t);
                        }
                    }
                    for t in thrown {
                        self.on_throw(t, span, Some(&desc), Some(e));
                    }
                    self.wrap_store = saved;
                    self.float_args = saved_f;
                    r
                } else {
                    self.member_of(base, name, name_span, span, obj);
                    for a in args {
                        self.expr(a.expr, None);
                    }
                    ERROR
                };
                return if mopt || optional { self.types.optional(result) } else { result };
            }
            ExprKind::Super => return self.super_ctor_call(e, args, span),
            _ => {}
        }
        let t = self.expr(callee, None);
        let text = self.src(cnode.span).to_string();
        let t = if optional { self.types.without_undefined(t) } else { t };
        if t == JS {
            return self.js_call(e, None, &text, args, span, false);
        }
        self.rec_call(e, Callee::Value);
        self.pending_call = Some(e);
        self.set_call_spec_from(callee);
        let r = self.call_value(t, &text, args, exp, span);
        if optional { self.types.optional(r) } else { r }
    }

    /// Records `from` and `to`, `to` with every field of `from` (of a type `from`'s fits) and
    /// more, all of them optional.
    fn adds_optional_fields(&mut self, from: TyId, to: TyId) -> bool {
        let (Ty::Record(ff), Ty::Record(tf)) = (*self.types.get(from), *self.types.get(to)) else { return false };
        let (ff, tf) = (self.types.fields(ff).to_vec(), self.types.fields(tf).to_vec());
        ff.iter().all(|f| tf.iter().any(|t| t.name == f.name && self.assignable(f.ty, t.ty)))
            && tf.iter().all(|t| t.optional || ff.iter().any(|f| f.name == t.name))
    }

    /// `x.name = v` where `x`'s class has a setter `name`: its method `__set_name`.
    fn setter_of(&mut self, target: ExprId) -> Option<Sym> {
        let ExprKind::Member { obj, name, .. } = self.ast().expr(target).kind else { return None };
        let setter = self.interner.lookup(&format!("__set_{}", self.name(name)))?;
        let t = match self.ast().expr(obj).kind {
            ExprKind::This => self.lookup(super::THIS_SYM).map(|(_, t)| t)?,
            _ => self.expr_type_peek(obj)?,
        };
        matches!(self.types.get(t), Ty::Class(..)).then_some(())?;
        matches!(self.class_member(t, setter), Some(super::class::ClassMemberRef::Method(_))).then_some(setter)
    }

    /// `const { a, ...others } = r`: `__rest(r, "a")`, the record of `r`'s other fields
    /// (rewritten as `{ b: r.b, c: r.c }`).
    fn object_rest(&mut self, e: ExprId, args: &[Arg], span: Span) -> TyId {
        let t = self.expr(args[0].expr, None);
        let names: Vec<Sym> = args[1..].iter().filter_map(|a| match self.ast().expr(a.expr).kind {
            ExprKind::Str(s) => Some(s),
            _ => None,
        }).collect();
        if t == ERROR {
            return ERROR;
        }
        if !matches!(self.types.get(t), Ty::Record(_)) {
            let msg = format!("rest properties take the other fields of a record, found `{}`", self.show(t));
            self.report(Diagnostic::new("U0006", span, msg));
            return ERROR;
        }
        let kept: Vec<Field> = self.types.fields_in_order(t).into_iter().filter(|f| !names.contains(&f.name)).collect();
        let order: Vec<Sym> = kept.iter().map(|f| f.name).collect();
        let r = self.types.record(kept);
        self.types.note_field_order(r, order.clone());
        self.rewrites.rests.push((self.cur, e, order));
        r
    }

    /// What iterating a `t` gives: an array's, a set's or a generator's elements (`None` if it
    /// isn't one of those).
    pub(super) fn iterated(&mut self, t: TyId) -> Option<TyId> {
        match *self.types.get(t) {
            Ty::Array(e) | Ty::Set(e) | Ty::Gen(e) => Some(e),
            Ty::Str | Ty::StrLit(_) => Some(STR),
            Ty::Error => Some(ERROR),
            _ => self.iterator_of(t),
        }
    }

    /// A class instance with a `[Symbol.iterator]()` method returning a generator: what it yields.
    pub(super) fn iterator_of(&mut self, t: TyId) -> Option<TyId> {
        let sym = self.interner.lookup("__iterator")?;
        self.class_of(t)?;
        let Some(super::class::ClassMemberRef::Method(mm)) = self.class_member(t, sym) else { return None };
        let r = self.method_ret(&mm);
        match *self.types.get(r) {
            Ty::Gen(e) => Some(e),
            _ => None,
        }
    }

    /// `xs.reduce(f)`: `reduce/1f` where `xs` holds integers and `f` (a named function) takes
    /// floats, as TypeScript's one `number` allows; else `reduce/1`.
    fn reduce_from_first(&mut self, base: TyId, f: ExprId) -> &'static str {
        if *self.types.get(base) != Ty::Array(INT) || !matches!(self.ast().expr(f).kind, ExprKind::Ident(_)) {
            return "reduce/1";
        }
        let ExprKind::Ident(sym) = self.ast().expr(f).kind else { return "reduce/1" };
        let t = match self.lookup(sym) {
            Some((_, t)) => t,
            None => match self.scopes[self.cur as usize].values.get(&sym).map(|d| d.0) {
                Some(Decl::Fn(m, i)) => match self.fn_sig(m, i) {
                    Some(sig) => return if sig.params.first().is_some_and(|p| self.types.is_float(p.ty)) { "reduce/1f" } else { "reduce/1" },
                    None => return "reduce/1",
                },
                _ => return "reduce/1",
            },
        };
        match *self.types.get(t) {
            Ty::Func(ps, _, _) if self.types.params(ps).first().is_some_and(|p| self.types.is_float(p.ty)) => "reduce/1f",
            _ => "reduce/1",
        }
    }

    /// `sig` with its callback parameters allowed to throw (anything: `error` stands for that).
    fn throwing_callbacks(&mut self, sig: &CallSig) -> CallSig {
        let mut sig = sig.clone();
        for p in sig.params.iter_mut() {
            let members = self.flat_members(p.ty);
            if !members.iter().any(|&m| matches!(self.types.get(m), Ty::Func(_, _, NEVER))) {
                continue;
            }
            let lenient: Vec<TyId> = members
                .into_iter()
                .map(|m| match *self.types.get(m) {
                    Ty::Func(ps, r, NEVER) => {
                        let ps = self.types.params(ps).to_vec();
                        self.types.func_throws(ps, r, ERROR)
                    }
                    _ => m,
                })
                .collect();
            p.ty = self.types.union(&lenient);
        }
        sig
    }

    /// What a function value passed as a callback (a function's name, a variable) can throw.
    fn fn_value_throws(&mut self, e: ExprId) -> Option<TyId> {
        let ExprKind::Ident(s) = self.ast().expr(e).kind else { return None };
        let t = match self.lookup(s) {
            Some((_, t)) => t,
            None => match self.scopes[self.cur as usize].values.get(&s).map(|d| d.0) {
                Some(Decl::Fn(m, i)) => self.fn_sig(m, i)?.throws,
                _ => return None,
            },
        };
        match *self.types.get(t) {
            Ty::Func(_, _, th) if th != NEVER && th != ERROR => Some(th),
            _ if t != NEVER && t != ERROR && !matches!(self.types.get(t), Ty::Func(..)) && self.lookup(s).is_none() => Some(t),
            _ => None,
        }
    }

    /// `super(...)` in a derived class's constructor.
    fn super_ctor_call(&mut self, e: ExprId, args: &[Arg], span: Span) -> TyId {
        let (class, ctor) = self.fcx.last().map(|f| (f.class, f.ctor)).unwrap_or((None, false));
        let base = class.and_then(|c| self.classes[c as usize].base);
        let (Some(_), true, Some(base)) = (class, ctor, base) else {
            self.report(Diagnostic::new("T0801", span, "`super(...)` is only valid in the constructor of a class that `extends` another"));
            for a in args {
                self.expr(a.expr, None);
            }
            return ERROR;
        };
        let (b, bargs) = self.class_of(base).unwrap();
        let binfo = self.classes[b as usize].clone();
        let map: HashMap<u32, TyId> = binfo.params.iter().copied().zip(bargs.iter().copied()).collect();
        let params: Vec<FnParam> = binfo.ctor.params.iter().map(|p| FnParam { ty: self.types.subst(p.ty, &map), ..*p }).collect();
        let names = binfo.ctor.param_names.iter().map(|&p| self.name(p).to_string()).collect();
        let throws = self.ctor_throws(b);
        let throws = self.types.subst(throws, &map);
        let cs = CallSig { tparams: Vec::new(), params, names, rest: None, ret: VOID, throws };
        self.rec_call(e, Callee::SuperCtor(base));
        self.pending_call = Some(e);
        let sites = self.ctor_sites(b);
        self.set_call_spec((sites, Vec::new()));
        self.call_sig(&cs, "super", &[], args, None, span)
    }

    /// `super.name(...)`: the base class's implementation.
    #[allow(clippy::too_many_arguments)]
    fn super_method_call(&mut self, e: ExprId, name: Sym, name_span: Span, type_args: &[crate::ast::TypeId], args: &[Arg], exp: Option<TyId>, span: Span) -> TyId {
        let class = self.fcx.last().and_then(|f| f.class);
        let base = class.and_then(|c| self.classes[c as usize].base);
        let has_this = self.lookup(super::THIS_SYM).is_some();
        let (Some(base), true) = (base, has_this) else {
            self.report(Diagnostic::new("T0801", span, "`super.method(...)` is only valid in methods of a class that `extends` another"));
            for a in args {
                self.expr(a.expr, None);
            }
            return ERROR;
        };
        let mtext = self.name(name).to_string();
        match self.class_member(base, name) {
            Some(super::class::ClassMemberRef::Method(mm)) => {
                if mm.is_abstract {
                    self.report(Diagnostic::new("T0806", name_span, format!("`super.{mtext}` is abstract in the base class")));
                }
                self.check_visible(mm.owner, mm.vis, name, name_span);
                let ret = self.method_ret(&mm);
                let names = mm.param_names.iter().map(|&p| self.name(p).to_string()).collect();
                let throws = self.method_throws(&mm);
                let (params, rest) = self.method_call_params(&mm);
                let cs = CallSig { tparams: Vec::new(), params, names, rest, ret, throws };
                self.rec_call(e, Callee::ClassMethod { recv: base, name, sup: true });
                self.pending_call = Some(e);
                let sites = self.member_sites(mm.owner, mm.member);
                self.set_call_spec(sites);
                self.call_sig(&cs, &format!("super.{mtext}"), type_args, args, exp, span)
            }
            _ => {
                let shown = self.show(base);
                self.report(Diagnostic::new("T0107", name_span, format!("base class `{shown}` has no method `{mtext}`")));
                for a in args {
                    self.expr(a.expr, None);
                }
                ERROR
            }
        }
    }

    fn call_value(&mut self, t: TyId, desc: &str, args: &[Arg], exp: Option<TyId>, span: Span) -> TyId {
        match *self.types.get(t) {
            Ty::Func(params, ret, throws) => {
                let params = self.types.params(params).to_vec();
                let names = (0..params.len()).map(|i| format!("arg{i}")).collect();
                let cs = CallSig { tparams: Vec::new(), params, names, rest: None, ret, throws };
                self.call_sig(&cs, desc, &[], args, exp, span)
            }
            Ty::Error => {
                for a in args {
                    self.expr(a.expr, None);
                }
                ERROR
            }
            _ => {
                for a in args {
                    self.expr(a.expr, None);
                }
                let msg = format!("`{desc}` is not a function (its type is `{}`)", self.show(t));
                self.report(Diagnostic::new("T0202", span, msg));
                ERROR
            }
        }
    }

    pub(super) fn describe_sig(&self, cs: &CallSig, desc: &str) -> String {
        let mut parts: Vec<String> = cs
            .params
            .iter()
            .zip(&cs.names)
            .map(|(p, n)| format!("{}{}{}: {}", if p.inout { "inout " } else { "" }, n, if p.optional { "?" } else { "" }, self.show(if p.optional { self.types_without_undef(p.ty) } else { p.ty })))
            .collect();
        if let Some(r) = cs.rest {
            parts.push(format!("...items: {}[]", self.show(r)));
        }
        format!("{desc}({}): {}", parts.join(", "), self.show(cs.ret))
    }

    // ------------------------------------------------------------ JavaScript values (npm)

    /// A function used as a value: its type keeps `throws`, so a call through the value passes the
    /// error on as a direct call does (a `throws` still being inferred reads as `Error`).
    fn fn_value_type(&mut self, sig: Sig) -> TyId {
        let throws = if sig.throws == UNKNOWN { self.error_class() } else { sig.throws };
        self.types.func_throws(sig.params, sig.ret, throws)
    }

    /// A JavaScript read (`o.x`, `o[k]`), whose getter or Proxy can throw: like a JavaScript call
    /// where errors can pass on or a `try { }` block catches them. Where they can't (a closure
    /// whose type doesn't throw, a module constant's initializer), it stays a trap and nothing is
    /// reported, so `xs.map((x: Js) => x.name)` compiles; a JavaScript call there is refused.
    fn js_read_throw(&mut self, e: ExprId, span: Span) {
        let Some(frame) = self.fcx.last().and_then(|f| f.frames.last()) else { return };
        if frame.try_frames.is_empty() && (!frame.can_throw || frame.module_init) {
            // (a closure's function type may throw then, as in TypeScript: checked again)
            if let Some(t) = frame.exp_fn.filter(|_| !frame.module_init) {
                self.blame_fn_type(t);
            }
            return;
        }
        let err = self.builtin_class("JsError");
        self.on_throw(err, span, Some("this JavaScript read"), Some(e));
    }

    /// Can a Tov value of type `t` go to JavaScript? It's converted on the way: numbers,
    /// strings, booleans, `undefined`, arrays, records and unions of these, `Js` itself, and
    /// functions whose parameters are `Js` (JavaScript calls them back).
    pub(super) fn js_convertible(&mut self, t: TyId) -> bool {
        match *self.types.get(t) {
            Ty::Js | Ty::Error | Ty::Never | Ty::Str | Ty::StrLit(_) | Ty::Bool | Ty::Undefined | Ty::Null | Ty::Void => true,
            Ty::Int | Ty::F64 | Ty::F32 | Ty::I8 | Ty::I16 | Ty::I32 | Ty::U8 | Ty::U8C | Ty::U16 | Ty::U32 | Ty::U64 => true,
            Ty::Array(e) => self.js_convertible(e),
            // (a JavaScript promise, settled when this one is)
            Ty::Promise(v, _) => self.js_convertible(v),
            Ty::Record(fs) => {
                let fs = self.types.fields(fs).to_vec();
                fs.iter().all(|f| self.js_convertible(f.ty))
            }
            Ty::Union(ms) => {
                let ms = self.types.tys(ms).to_vec();
                ms.iter().all(|&m| self.js_convertible(m))
            }
            // (an error the function passes on is thrown into JavaScript)
            Ty::Func(ps, r, _) => {
                let ps = self.types.params(ps).to_vec();
                ps.iter().all(|p| p.ty == JS && !p.inout) && self.js_convertible(r)
            }
            _ => false,
        }
    }

    /// The type of `e` if it's a plain name (a local, or an npm import): enough to tell a
    /// JavaScript object without checking `e` twice. Other expressions: `None` (checked normally).
    fn expr_type_peek(&mut self, e: ExprId) -> Option<TyId> {
        match self.ast().expr(e).kind {
            ExprKind::Ident(s) => match self.lookup(s) {
                Some((_, t)) => Some(self.types.without_undefined(t)),
                None => match self.scopes[self.cur as usize].values.get(&s).map(|d| d.0) {
                    Some(Decl::Npm(..)) => Some(JS),
                    Some(Decl::Const(m, i)) => {
                        let t = self.const_type(m, i);
                        Some(self.types.without_undefined(t))
                    }
                    _ => None,
                },
            },
            ExprKind::Member { obj, .. } | ExprKind::Index { obj, .. } => (self.expr_type_peek(obj) == Some(JS)).then_some(JS),
            ExprKind::Paren(x) => self.expr_type_peek(x),
            _ => None,
        }
    }

    /// Can a JavaScript value be converted to `t` (`x as T`)?
    pub(super) fn js_receivable(&mut self, t: TyId) -> bool {
        match *self.types.get(t) {
            Ty::Js | Ty::Error | Ty::Str | Ty::StrLit(_) | Ty::Bool | Ty::Undefined | Ty::Null | Ty::Void => true,
            Ty::Int | Ty::F64 | Ty::F32 | Ty::I8 | Ty::I16 | Ty::I32 | Ty::U8 | Ty::U8C | Ty::U16 | Ty::U32 | Ty::U64 => true,
            Ty::Array(e) => self.js_receivable(e),
            Ty::Record(fs) => {
                let fs = self.types.fields(fs).to_vec();
                fs.iter().all(|f| self.js_receivable(f.ty))
            }
            Ty::Union(ms) => {
                let ms = self.types.tys(ms).to_vec();
                ms.iter().all(|&m| self.js_receivable(m))
            }
            _ => false,
        }
    }

    /// An argument to a JavaScript function: arrows get `Js` parameters (and return `Js`).
    fn js_arg(&mut self, a: &Arg) {
        let x = a.expr;
        let exp = match &self.ast().expr(x).kind {
            ExprKind::Arrow(f) => {
                let n = f.params.len();
                let ps = vec![FnParam { ty: JS, inout: false, optional: false }; n];
                // a block that returns nothing: `void` (JavaScript sees `undefined`)
                let ret = match f.body {
                    crate::ast::ArrowBody::Block(b) if !super::returns_value(self.ast(), b) => VOID,
                    _ => JS,
                };
                // (it may pass errors on: they're thrown into JavaScript)
                let err = self.error_class();
                Some(self.types.func_throws(ps, ret, err))
            }
            _ => Some(JS),
        };
        let mut t = self.expr(x, exp);
        // A named function passed by name keeps its `throws` here, so the trampoline throws its
        // errors into JavaScript. (Function values elsewhere don't carry it yet.)
        if let ExprKind::Ident(sym) = self.ast().expr(x).kind
            && self.lookup(sym).is_none()
            && let Some(Decl::Fn(fm, fi)) = self.scopes[self.cur as usize].values.get(&sym).map(|d| d.0)
            && let Some(sig) = self.fn_sig(fm, fi)
            && sig.throws != NEVER
            && let Ty::Func(ps, r, _) = *self.types.get(t)
        {
            let throws = if sig.throws == UNKNOWN { self.error_class() } else { sig.throws };
            let ps = self.types.params(ps).to_vec();
            t = self.types.func_throws(ps, r, throws);
            if let Some(f) = self.facts_mut() {
                f.expr_ty[x as usize] = t;
            }
        }
        if !self.js_convertible(t) {
            let shown = self.show(t);
            let s = self.ast().expr(x).span;
            self.report(
                Diagnostic::new("T0901", s, format!("a `{shown}` can't be passed to JavaScript"))
                    .note("why", "values are converted for JavaScript: numbers, strings, booleans, `undefined`, arrays, records, `Js` values, and functions taking `Js`"),
            );
        }
    }

    /// A call to a JavaScript function or method, or `new` on a JavaScript constructor: returns
    /// a `Js`, and can throw a `JsError`.
    fn js_call(&mut self, e: ExprId, method: Option<Sym>, desc: &str, args: &[Arg], span: Span, new: bool) -> TyId {
        for a in args {
            if let Some(s) = a.by_ref {
                self.report(Diagnostic::new("T0904", s, "JavaScript functions take values, not `&` references"));
            }
            self.js_arg(a);
        }
        self.rec_call(e, if new { Callee::JsNew } else { Callee::Js { method } });
        let err = self.builtin_class("JsError");
        self.on_throw(err, span, Some(desc), Some(e));
        JS
    }

    fn types_without_undef(&self, t: TyId) -> TyId {
        match *self.types.get(t) {
            Ty::Union(ms) if self.types.tys(ms).len() == 2 && self.types.tys(ms).contains(&UNDEFINED) => *self.types.tys(ms).iter().find(|&&m| m != UNDEFINED).unwrap(),
            _ => t,
        }
    }

    pub(super) fn call_sig(&mut self, cs: &CallSig, desc: &str, type_args: &[crate::ast::TypeId], args: &[Arg], exp: Option<TyId>, span: Span) -> TyId {
        // (`f(...t)` with `t` a tuple: its elements fill as many parameters)
        let spread_tuples = args.iter().any(|a| matches!(self.ast().expr(a.expr).kind, ExprKind::Spread(_)));
        let widened = if spread_tuples { self.tuple_spread_sig(cs, args) } else { None };
        let cs = widened.as_ref().unwrap_or(cs);
        let (mut params, mut ret) = self.call_spec.take().unwrap_or_default();
        if !self.spec.on {
            return self.call_sig_inner(cs, desc, type_args, args, exp, span, &[]);
        }
        // A generic call links its arguments with its type arguments and with where its result
        // goes (the expected type can fix a type parameter).
        if !cs.tparams.is_empty() {
            let mut linked: Vec<super::spec::Site> = type_args.iter().flat_map(|&t| self.type_sites(self.cur, t)).collect();
            linked.extend(self.spec_flow.last().cloned().unwrap_or_default());
            if !linked.is_empty() {
                if params.is_empty() {
                    params.push(Vec::new());
                }
                for p in &mut params {
                    p.extend_from_slice(&linked);
                }
                ret.extend(linked);
            }
        }
        let t = self.call_sig_inner(cs, desc, type_args, args, exp, span, &params);
        self.note_sites(span, ret);
        t
    }

    /// `cs` with the parameters a spread tuple fills (`f(...t)`, `t: [A, B]`) as one, the tuple
    /// spread (`Ty::Spread`); `None` if no argument spreads a tuple into fixed parameters.
    fn tuple_spread_sig(&mut self, cs: &CallSig, args: &[Arg]) -> Option<CallSig> {
        let mut params = Vec::new();
        let mut names = Vec::new();
        let mut pi = 0;
        let mut any = false;
        for a in args {
            let tuple = match self.ast().expr(a.expr).kind {
                ExprKind::Spread(inner) if pi < cs.params.len() && matches!(self.ast().expr(inner).kind, ExprKind::Ident(_) | ExprKind::Member { .. } | ExprKind::Paren(_)) => {
                    let marks = self.diags.len();
                    let t = self.expr(inner, None);
                    self.diags.truncate(marks);
                    self.types.tuple_elems(t)
                }
                _ => None,
            };
            match tuple {
                Some(elems) if pi + elems.len() <= cs.params.len() => {
                    let tys: Vec<TyId> = cs.params[pi..pi + elems.len()].iter().map(|p| p.ty).collect();
                    let tup = self.types.tuple(&tys);
                    params.push(FnParam { ty: self.types.intern(Ty::Spread(tup)), inout: false, optional: false });
                    names.push(cs.names.get(pi).cloned().unwrap_or_default());
                    pi += elems.len();
                    any = true;
                }
                _ => {
                    if let Some(p) = cs.params.get(pi) {
                        params.push(*p);
                        names.push(cs.names.get(pi).cloned().unwrap_or_default());
                    }
                    pi += 1;
                }
            }
        }
        if !any {
            return None;
        }
        params.extend(cs.params.iter().skip(pi).copied());
        names.extend(cs.names.iter().skip(pi).cloned());
        Some(CallSig { params, names, ..cs.clone() })
    }

    /// The sites of the parameter argument `i` goes to.
    fn arg_sites(params: &[Vec<super::spec::Site>], i: usize) -> Vec<super::spec::Site> {
        params.get(i).or(params.last()).cloned().unwrap_or_default()
    }

    #[allow(clippy::too_many_arguments)]
    fn call_sig_inner(&mut self, cs: &CallSig, desc: &str, type_args: &[crate::ast::TypeId], args: &[Arg], exp: Option<TyId>, span: Span, sites: &[Vec<super::spec::Site>]) -> TyId {
        let call_expr = self.pending_call.take();
        // (a built-in's integer parameters take numbers: only the arguments themselves, not
        // what's checked inside them)
        let float_ok = std::mem::take(&mut self.float_args);
        let required = cs.params.iter().filter(|p| !p.optional).count();
        let max = if cs.rest.is_some() { usize::MAX } else { cs.params.len() };
        if args.len() < required || args.len() > max {
            let expected = if cs.rest.is_some() {
                format!("at least {required}")
            } else if required == cs.params.len() {
                format!("{required}")
            } else {
                format!("{required} to {}", cs.params.len())
            };
            let d = Diagnostic::new("T0201", span, format!("`{desc}` takes {expected} argument{}, found {}", if expected == "1" { "" } else { "s" }, args.len()))
                .note("signature", self.describe_sig(cs, desc));
            self.report(d);
        }
        let mut map: HashMap<u32, TyId> = HashMap::default();
        if !type_args.is_empty() {
            if type_args.len() != cs.tparams.len() {
                let msg = format!("`{desc}` takes {} type argument(s), found {}", cs.tparams.len(), type_args.len());
                self.report(Diagnostic::new("T0010", span, msg));
            }
            let tscope = self.tscope();
            for (&p, &ta) in cs.tparams.iter().zip(type_args) {
                let t = self.resolve_type(ta, &tscope);
                map.insert(p, t);
            }
        }
        // The expected type can fix type parameters the arguments don't mention (`const s: Stack<int> = new Stack()`),
        // and hints the arguments; what an argument shows wins (`total += xs.reduce((a, b) => a + b, 0.0)` with an int
        // `total` that the f64 result promotes).
        let mut soft: HashMap<u32, TyId> = HashMap::default();
        if type_args.is_empty()
            && !cs.tparams.is_empty()
            && let Some(x) = exp
            && x != ERROR
        {
            let x = self.types.without_undefined(x);
            let free = cs.tparams.clone();
            let ret = self.types.without_undefined(cs.ret);
            self.unify(ret, x, &free, &mut soft);
        }
        let saved_free = std::mem::take(&mut self.infer_free);
        self.infer_free = cs.tparams.iter().copied().filter(|p| !map.contains_key(p)).collect();
        let param_for = |i: usize| -> Option<FnParam> { cs.params.get(i).cloned().or_else(|| cs.rest.map(|ty| FnParam { ty, inout: false, optional: false })) };
        let mut arg_tys: Vec<Option<TyId>> = vec![None; args.len()];
        // Pass 1: non-closure arguments drive inference; pass 2: closures get the inferred parameter
        // types. An object literal holding closures (`{ port, data, socket: { open(s) {...} } }`)
        // is both: its other fields infer in pass 1, the whole object is checked in pass 2.
        // (an object literal for a parameter that mentions open type parameters: its fields infer
        // them in pass 1, and the whole is checked against what they found in pass 2)
        let deferred: Vec<bool> = args
            .iter()
            .enumerate()
            .map(|(i, a)| {
                matches!(self.ast().expr(a.expr).kind, ExprKind::Object(_))
                    && !self.infer_free.is_empty()
                    && param_for(i).is_some_and(|p| {
                        let open: Vec<u32> = self.infer_free.iter().copied().filter(|x| !map.contains_key(x)).collect();
                        self.types.mentions(p.ty, &open)
                    })
            })
            .collect();
        // type parameters only an int literal argument bound (`xs.reduce((a, b) => a + b, 0)`): a
        // callback that makes them a float (as TypeScript's one `number` would be) rebinds them
        let mut literal_bound: Vec<u32> = Vec::new();
        for pass in 0..2 {
            if pass == 1 {
                // what the arguments left open takes the expected type's, or its default (`<T = undefined>`)
                for (&p, &t) in &soft {
                    map.entry(p).or_insert(t);
                }
                for &p in &cs.tparams {
                    if !map.contains_key(&p)
                        && let Some(d) = self.gparam(p).default
                    {
                        map.insert(p, d);
                    }
                }
            }
            for (i, a) in args.iter().enumerate() {
                let with_closures = deferred[i];
                let is_arrow = matches!(self.ast().expr(a.expr).kind, ExprKind::Arrow(_)) || with_closures;
                if pass == 0 && with_closures {
                    if let Some(p) = param_for(i) {
                        let free = self.infer_free.clone();
                        self.infer_from_object(a.expr, p.ty, &free, &mut map);
                    }
                    continue;
                }
                if (pass == 0) == is_arrow {
                    continue;
                }
                let Some(p) = param_for(i) else {
                    self.expr(a.expr, None);
                    continue;
                };
                let pty = if pass == 0 && !soft.is_empty() {
                    let mut hinted = soft.clone();
                    hinted.extend(map.iter().map(|(&k, &v)| (k, v)));
                    self.types.subst(p.ty, &hinted)
                } else {
                    self.types.subst(p.ty, &map)
                };
                // (`fn: (e: E) => void` as `(e: number) => void` here: a closure that throws
                // through it blames the parameter's type as written)
                if self.spec.on && pty != p.ty {
                    self.inherit_fn_sites(p.ty, pty);
                }
                let still_free: Vec<u32> = self.infer_free.iter().copied().filter(|x| !map.contains_key(x)).collect();
                let unhinted: Vec<u32> = still_free.iter().copied().filter(|x| !(pass == 0 && soft.contains_key(x))).collect();
                // (a constructor type's parameter: a class given for it is checked against it)
                let ctor = matches!(*self.types.get(pty), Ty::Param(q) if self.gparam(q).ctor.is_some());
                let hint = if is_arrow || ctor || !self.types.mentions(pty, &unhinted) { Some(pty) } else { None };
                // (`chunk([], 3)` for a `T[]`: an array of nothing, `T` `never`, as TypeScript's)
                let hint = match hint {
                    None if matches!(&self.ast().expr(a.expr).kind, ExprKind::Array(xs) if xs.is_empty()) && matches!(self.types.get(pty), Ty::Array(_)) => Some(self.types.array(NEVER)),
                    // (`fromEntries([["a", 1]])` for a `[string, V][]`: its shape, so the pairs are
                    // tuples, `V` left to what they hold)
                    None if matches!(&self.ast().expr(a.expr).kind, ExprKind::Array(xs) if !xs.is_empty() && xs.iter().all(|&x| matches!(self.ast().expr(x).kind, ExprKind::Array(_))))
                        && matches!(self.types.get(pty), Ty::Array(el) if self.tuple_of(*el).is_some()) =>
                    {
                        let holes: HashMap<u32, TyId> = unhinted.iter().map(|&u| (u, ERROR)).collect();
                        Some(self.types.subst(pty, &holes))
                    }
                    h => h,
                };
                self.infer_free = still_free;
                let marks = self.diags.len();
                if self.spec.on {
                    self.spec_flow.push(Self::arg_sites(sites, i));
                }
                let mut t = match self.ast().expr(a.expr).kind {
                    // (`f(...xs)` spreads into the rest parameter)
                    ExprKind::Spread(inner) if i >= cs.params.len() && cs.rest.is_some() => self.spread_elem(inner, hint),
                    // (`f(...args)` for `f: (...args: A) => R`: the tuple's elements)
                    ExprKind::Spread(inner) if let Ty::Spread(want) = *self.types.get(pty) => {
                        let t = self.expr(inner, Some(want));
                        self.types.intern(Ty::Spread(t))
                    }
                    ExprKind::Spread(inner) => {
                        let s = self.ast().expr(a.expr).span;
                        self.report(Diagnostic::new("U0011", s, format!("`...` here would fill fixed parameters of `{desc}`; only a rest parameter (`...xs: T[]`) takes a spread")));
                        self.spread_elem(inner, None);
                        ERROR
                    }
                    _ => self.expr(a.expr, hint),
                };
                if is_arrow && literal_bound.iter().any(|u| map.get(u) == Some(&INT) && self.types.mentions(p.ty, &[*u])) {
                    let mut scratch = map.clone();
                    for u in &literal_bound {
                        scratch.remove(u);
                    }
                    self.unify(p.ty, t, &cs.tparams, &mut scratch);
                    let floats: Vec<u32> = literal_bound.iter().copied().filter(|u| scratch.get(u).is_some_and(|&x| self.types.is_float(x))).collect();
                    if !floats.is_empty() {
                        self.diags.truncate(marks);
                        for u in floats {
                            map.insert(u, scratch[&u]);
                        }
                        let pty = self.types.subst(p.ty, &map);
                        t = self.expr(a.expr, Some(pty));
                    }
                }
                if self.spec.on {
                    self.spec_flow.pop();
                }
                arg_tys[i] = Some(t);
                let free = self.infer_free.clone();
                if pass == 0
                    && self.is_int_literal(a.expr)
                    && let Ty::Param(u) = *self.types.get(p.ty)
                    && free.contains(&u)
                    && !map.contains_key(&u)
                {
                    literal_bound.push(u);
                }
                self.unify(p.ty, t, &free, &mut map);
            }
        }
        self.infer_free = saved_free;
        // Check each argument against the instantiated parameter type.
        let mut refs: Vec<(LocalId, Span)> = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let Some(p) = param_for(i) else { continue };
            let Some(t) = arg_tys[i] else { continue };
            let pty = self.types.subst(p.ty, &map);
            let aspan = self.ast().expr(a.expr).span;
            let pname = cs.names.get(i).cloned().unwrap_or_else(|| "items".to_string());
            match (p.inout, a.by_ref) {
                // (a fresh value, `walk(3, [])`: the callee's changes are its own, as in TypeScript)
                (true, None) if !super::stmt::place_path(self.ast(), a.expr) => {}
                (true, None) => {
                    self.rewrites.args.push((self.cur, a.expr));
                    let text = self.src(aspan).to_string();
                    self.report(
                        Diagnostic::new("V0110", aspan, format!("parameter `{pname}` of `{desc}` is `inout`; pass the argument with `&`"))
                            .fix(Applicability::Safe, format!("pass `&{text}`"), aspan.empty_at_start(), "&"),
                    );
                }
                (false, Some(amp)) => {
                    self.report(
                        Diagnostic::new("V0111", amp.to(aspan), format!("parameter `{pname}` of `{desc}` is not `inout`; pass the value without `&`"))
                            .fix(Applicability::Safe, "remove `&`", amp, ""),
                    );
                }
                (true, Some(_)) => {
                    self.invalidate_place(a.expr);
                    if let Some(place) = self.place(a.expr, true) {
                        if let Some(root) = place.root {
                            if let Some(&(_, prev)) = refs.iter().find(|(r, _)| *r == root) {
                                let n = self.name(self.local(root).name).to_string();
                                let _ = prev;
                                self.report(Diagnostic::new("V0113", aspan, format!("`{n}` is passed as `inout` more than once in this call")).note("why", "each `inout` argument must be a separate value"));
                            }
                            refs.push((root, aspan));
                        }
                        if t != pty && t != ERROR && pty != ERROR {
                            let msg = format!("`inout` argument must be exactly `{}`, found `{}`", self.show(pty), self.show(t));
                            self.report(Diagnostic::new("T0001", aspan, msg));
                        }
                    }
                    continue;
                }
                (false, None) => {}
            }
            // (only parameters that are integers by nature: positions, counts, lengths, digits;
            // not an element's type, which may be a `number` taken as an `int`)
            self.float_args = float_ok && matches!(pname.as_str(), "index" | "start" | "end" | "count" | "length" | "digits" | "precision" | "radix" | "from" | "position" | "pos" | "fromIndex" | "deleteCount");
            let fits = self.assignable(t, pty);
            self.float_args = false;
            if !fits {
                let d = self.flow(Self::arg_sites(sites, i), |c| c.mismatch(t, pty, aspan, Some(format!("argument `{pname}` of `{desc}`"))));
                self.report(d);
            } else if self.spec.on && t == JS {
                self.flow(Self::arg_sites(sites, i), |c| c.spec_poison(pty));
            }
        }
        let mut unresolved: Vec<u32> = cs.tparams.iter().copied().filter(|p| !map.contains_key(p) && self.types.mentions(cs.ret, &[*p])).collect();
        // (the rest from where the result goes: `const c: Cache<string> = new Cache(10)`)
        if !unresolved.is_empty()
            && let Some(x) = exp
            && !self.types.mentions_error(x)
        {
            self.unify(cs.ret, x, &unresolved, &mut map);
            unresolved.retain(|p| !map.contains_key(p));
        }
        // (`const lru = new LRU(size)`: found from its method calls, see `EV_CLASS`)
        if !unresolved.is_empty() && self.lenient_new == Some(span) && desc.starts_with("new ") {
            self.lenient_new = None;
            for p in unresolved {
                map.insert(p, ERROR);
            }
            self.lenient_hit = true;
            return self.types.subst(cs.ret, &map);
        }
        if !unresolved.is_empty() {
            let names: Vec<String> = unresolved.iter().map(|&p| self.name(self.gparam(p).name).to_string()).collect();
            let _ = exp;
            self.report(
                Diagnostic::new("T0305", span, format!("can't infer type argument `{}` of `{desc}`", names.join("`, `")))
                    .note("instead", format!("pass it explicitly: `{desc}<{}>(...)`", names.join(", "))),
            );
            return ERROR;
        }
        let ret = self.types.subst(cs.ret, &map);
        if cs.throws != NEVER && cs.throws != ERROR {
            let thrown = if cs.throws == UNKNOWN { self.error_class() } else { self.types.subst(cs.throws, &map) };
            self.on_throw(thrown, span, Some(desc), call_expr);
        }
        if let Some(ce) = call_expr
            && self.facts.is_some()
        {
            let params: Vec<FnParam> = cs.params.iter().map(|p| FnParam { ty: self.types.subst(p.ty, &map), ..*p }).collect();
            let rest = cs.rest.map(|r| self.types.subst(r, &map));
            let targs: Vec<(u32, TyId)> = cs.tparams.iter().filter_map(|p| map.get(p).map(|t| (*p, *t))).collect();
            if let Some(f) = self.facts_mut()
                && let Some(c) = f.calls.get_mut(&ce)
            {
                c.params = params;
                c.rest = rest;
                c.targs = targs;
                c.ret = ret;
                c.own_ret = ret;
            }
        }
        ret
    }

    /// Does an object literal hold a closure (at any depth of object literals)?
    fn has_closure(&self, e: ExprId) -> bool {
        match &self.ast().expr(e).kind {
            ExprKind::Arrow(_) => true,
            ExprKind::Object(fs) => fs.iter().any(|f| self.has_closure(f.value)),
            _ => false,
        }
    }

    /// Infers type parameters from an object literal's fields that aren't closures, against the
    /// record type the parameter expects (closures are checked later, with what this finds).
    fn infer_from_object(&mut self, e: ExprId, pattern: TyId, free: &[u32], map: &mut HashMap<u32, TyId>) {
        let ExprKind::Object(fields) = &self.ast().expr(e).kind else { return };
        let pat = self.types.without_undefined(pattern);
        let pat = self.unfold(pat);
        let Ty::Record(fs) = *self.types.get(pat) else { return };
        let fs = self.types.fields(fs).to_vec();
        for f in fields {
            let Some(ft) = fs.iter().find(|x| x.name == f.name).map(|x| x.ty) else { continue };
            match self.ast().expr(f.value).kind {
                ExprKind::Arrow(_) => {}
                ExprKind::Object(_) if self.has_closure(f.value) => self.infer_from_object(f.value, ft, free, map),
                _ => {
                    let open: Vec<u32> = free.iter().copied().filter(|p| !map.contains_key(p)).collect();
                    let pty = self.types.subst(ft, map);
                    let hint = if self.types.mentions(pty, &open) { None } else { Some(pty) };
                    let t = self.expr(f.value, hint);
                    self.unify(ft, t, free, map);
                }
            }
        }
    }

    pub(super) fn unify(&mut self, pattern: TyId, actual: TyId, free: &[u32], map: &mut HashMap<u32, TyId>) {
        if actual == ERROR {
            return;
        }
        // (`Result<number>` given where its body is the pattern: the body)
        // (but a type parameter takes the alias itself: `xs.map(x => evaluate(x))` is a `Val[]`)
        let actual = if matches!(self.types.get(actual), Ty::Rec(..)) && !matches!(self.types.get(pattern), Ty::Rec(..) | Ty::Param(_)) { self.unfold(actual) } else { actual };
        match *self.types.get(pattern) {
            Ty::Param(p) if free.contains(&p) => match map.get(&p).copied() {
                None => {
                    let w = self.types.widen(actual);
                    map.insert(p, w);
                }
                Some(bound) => {
                    if !self.assignable(actual, bound) && self.assignable(bound, actual) {
                        let w = self.types.widen(actual);
                        map.insert(p, w);
                    }
                }
            },
            Ty::Array(pe) => {
                if let Ty::Array(ae) = *self.types.get(actual) {
                    self.unify(pe, ae, free, map);
                }
            }
            Ty::Set(pe) => {
                if let Ty::Set(ae) = *self.types.get(actual) {
                    self.unify(pe, ae, free, map);
                }
            }
            Ty::Gen(pe) => {
                if let Ty::Gen(ae) | Ty::Array(ae) = *self.types.get(actual) {
                    self.unify(pe, ae, free, map);
                }
            }
            Ty::Map(pk, pv) => {
                if let Ty::Map(ak, av) = *self.types.get(actual) {
                    self.unify(pk, ak, free, map);
                    self.unify(pv, av, free, map);
                }
            }
            Ty::Promise(pv, pe) => {
                if let Ty::Promise(av, ae) = *self.types.get(actual) {
                    self.unify(pv, av, free, map);
                    if ae != NEVER {
                        self.unify(pe, ae, free, map);
                    }
                }
            }
            Ty::Spread(p) => {
                if let Ty::Spread(a) = *self.types.get(actual) {
                    self.unify(p, a, free, map);
                }
            }
            Ty::Func(pps, pr, _) => {
                if let Ty::Func(aps, ar, _) = *self.types.get(actual) {
                    let (pps, aps) = (self.types.params(pps).to_vec(), self.types.params(aps).to_vec());
                    for (k, (pp, ap)) in pps.iter().zip(&aps).enumerate() {
                        // (`(...args: A) => R` given `(i: number, j: number) => number`: `A` is
                        // `[number, number]`)
                        if let Ty::Spread(st) = *self.types.get(pp.ty)
                            && !matches!(self.types.get(ap.ty), Ty::Spread(_))
                        {
                            let elems: Vec<TyId> = aps[k..].iter().map(|p| p.ty).collect();
                            let tup = self.types.tuple(&elems);
                            self.unify(st, tup, free, map);
                            break;
                        }
                        self.unify(pp.ty, ap.ty, free, map);
                    }
                    if let Some(pp) = pps.get(aps.len())
                        && let Ty::Spread(st) = *self.types.get(pp.ty)
                    {
                        let tup = self.types.tuple(&[]);
                        self.unify(st, tup, free, map);
                    }
                    self.unify(pr, ar, free, map);
                }
            }
            Ty::Record(pfs) => {
                if let Ty::Record(afs) = *self.types.get(actual) {
                    let (pfs, afs) = (self.types.fields(pfs).to_vec(), self.types.fields(afs).to_vec());
                    for pf in &pfs {
                        if let Some(af) = afs.iter().find(|f| f.name == pf.name) {
                            self.unify(pf.ty, af.ty, free, map);
                        }
                    }
                }
            }
            Ty::Union(pms) => {
                // (`N<T> | null`, `T | undefined`: the type that's there)
                let pattern_rest: Vec<TyId> = self.types.tys(pms).iter().copied().filter(|&m| m != UNDEFINED && m != NULL).collect();
                if pattern_rest.len() == 1 {
                    let actual_rest = self.types.without_nullish(actual);
                    self.unify(pattern_rest[0], actual_rest, free, map);
                } else {
                    // (variants of records: each with the actual one of the same fields)
                    let names = |c: &Self, t: TyId| match *c.types.get(t) {
                        Ty::Record(fs) => Some(c.types.fields(fs).iter().map(|f| f.name).collect::<Vec<Sym>>()),
                        _ => None,
                    };
                    let actuals = self.flat_members(actual);
                    for pm in pattern_rest {
                        let Some(pn) = names(self, pm) else { continue };
                        if let Some(&am) = actuals.iter().find(|&&a| names(self, a).as_ref() == Some(&pn)) {
                            self.unify(pm, am, free, map);
                        }
                    }
                }
            }
            Ty::Class(pd, pargs) => {
                if let Some(up) = self.upcast_to(actual, pd)
                    && let Ty::Class(_, aargs) = *self.types.get(up)
                {
                    let (pargs, aargs) = (self.types.tys(pargs).to_vec(), self.types.tys(aargs).to_vec());
                    for (p, a) in pargs.iter().zip(&aargs) {
                        self.unify(*p, *a, free, map);
                    }
                }
            }
            Ty::Rec(pd, pargs) | Ty::Interface(pd, pargs) => {
                if let Ty::Rec(ad, aargs) | Ty::Interface(ad, aargs) = *self.types.get(actual)
                    && pd == ad {
                        let (pargs, aargs) = (self.types.tys(pargs).to_vec(), self.types.tys(aargs).to_vec());
                        for (p, a) in pargs.iter().zip(&aargs) {
                            self.unify(*p, *a, free, map);
                        }
                        return;
                    }
                // (a class or an object given for an interface, `drain(stack)` for a
                // `Collection<T>`: its members' types against the interface's)
                if let Ty::Interface(i, pargs) = *self.types.get(pattern)
                    && matches!(self.types.get(actual), Ty::Class(..) | Ty::Record(_) | Ty::Interface(..))
                {
                    let pargs = self.types.tys(pargs).to_vec();
                    let fields = self.iface_fields_inst(i, &pargs);
                    for f in fields {
                        if !self.types.mentions(f.ty, free) {
                            continue;
                        }
                        let mt = match *self.types.get(actual) {
                            Ty::Class(..) => self.class_member(actual, f.name).map(|mr| self.member_type(&mr)),
                            _ => self.field_of(actual, f.name),
                        };
                        if let Some(mt) = mt {
                            self.unify(f.ty, mt, free, map);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- conditions and narrowing

    fn condition_ok(&mut self, t: TyId) -> bool {
        // any value, by its truthiness, as in JavaScript
        t != VOID
    }

    /// What of `t` is truthy (`a || b` keeps it): all but `undefined` and `null`.
    fn truthy_part(&mut self, t: TyId) -> TyId {
        self.types.without_nullish(t)
    }

    /// What of `t` can be falsy (`a && b` keeps it): `undefined`, and numbers, strings and
    /// booleans (`0`, `""`, `false`); objects never are.
    fn falsy_part(&mut self, t: TyId) -> TyId {
        let keep: Vec<TyId> = self.flat_members(t).into_iter().filter(|&m| m == UNDEFINED || m == NULL || self.types.is_primitive(m) || m == BOOL).collect();
        if keep.is_empty() { NEVER } else { self.types.union(&keep) }
    }

    pub(super) fn cond(&mut self, e: ExprId) {
        // (`if (seen[k])`: a missing element or key is `undefined`, so false, as in JavaScript)
        if matches!(self.ast().expr(e).kind, ExprKind::Index { .. }) {
            self.want_optional = true;
        }
        let t = self.expr(e, Some(BOOL));
        if self.condition_ok(t) {
            return;
        }
        let span = self.ast().expr(e).span;
        let text = self.src(span).to_string();
        let shown = self.show(t);
        let mut d = Diagnostic::new("T0401", span, format!("condition must be `bool`, found `{shown}`"))
            .note("why", "no implicit truthiness: `0`, `\"\"` and `undefined` are not `false`");
        let inner = self.types.without_undefined(t);
        if self.types.has_undefined(t) {
            d = d.fix(Applicability::Maybe, format!("test for presence: `{text} !== undefined`"), span, format!("{text} !== undefined"));
        } else if self.types.is_numeric(inner) {
            d = d.fix(Applicability::Maybe, format!("`{text} !== 0`"), span, format!("{text} !== 0"));
        } else if inner == DECIMAL {
            d = d.fix(Applicability::Maybe, format!("`{text} !== 0`"), span, format!("{text} !== 0"));
        } else if inner == BIGINT {
            d = d.fix(Applicability::Maybe, format!("`{text} !== 0n`"), span, format!("{text} !== 0n"));
        } else if self.types.is_string(inner) {
            d = d.fix(Applicability::Maybe, format!("`{text} !== \"\"`"), span, format!("{text} !== \"\""));
        } else if matches!(self.types.get(inner), Ty::Array(_)) {
            d = d.fix(Applicability::Maybe, format!("`{text}.length > 0`"), span, format!("{text}.length > 0"));
        }
        self.report(d);
    }

    pub(super) fn apply(&mut self, ns: &[Narrowing]) {
        for n in ns {
            match n {
                Narrowing::Local(l, t) => self.narrow(*l, *t),
                Narrowing::Global(key, t) => self.narrow_global(*key, *t),
                Narrowing::Path(l, p, t, heap) => {
                    self.narrow_path(*l, p.clone(), Some(*t));
                    if *heap {
                        self.fcx().heap_paths.push((*l, p.clone()));
                    }
                }
            }
        }
    }

    /// What we learn about locals when `e` evaluates to `positive`.
    pub(super) fn narrowings(&mut self, e: ExprId, positive: bool) -> Vec<Narrowing> {
        let ast = self.ast();
        match &ast.expr(e).kind {
            ExprKind::Paren(x) => self.narrowings(*x, positive),
            // `if (token = next())`: the variable, as just assigned, tested
            ExprKind::Assign(AssignOp::Assign, t, _) if matches!(ast.expr(*t).kind, ExprKind::Ident(_)) => self.narrowings(*t, positive),
            ExprKind::Unary(UnOp::Not, x) => self.narrowings(*x, !positive),
            ExprKind::Binary(BinOp::And, l, r) if positive => {
                let mut v = self.narrowings(*l, true);
                self.push_scope();
                self.apply(&v);
                v.extend(self.narrowings(*r, true));
                self.pop_scope();
                v
            }
            ExprKind::Binary(BinOp::Or, l, r) if !positive => {
                let mut v = self.narrowings(*l, false);
                self.push_scope();
                self.apply(&v);
                v.extend(self.narrowings(*r, false));
                self.pop_scope();
                v
            }
            ExprKind::Binary(op @ (BinOp::Eq | BinOp::Ne | BinOp::LooseEq | BinOp::LooseNe), l, r) => {
                let eq = matches!(op, BinOp::Eq | BinOp::LooseEq) == positive;
                let (l, r) = (*l, *r);
                let (subject, other) = if matches!(ast.expr(l).kind, ExprKind::Undefined | ExprKind::Null | ExprKind::Str(_) | ExprKind::Bool(_)) { (r, l) } else { (l, r) };
                let loose = matches!(op, BinOp::LooseEq | BinOp::LooseNe);
                match &ast.expr(other).kind {
                    // (`x == null` and `x == undefined` match both)
                    ExprKind::Undefined | ExprKind::Null if loose => {
                        let Some((target, ty)) = self.target_of(subject) else { return Vec::new() };
                        let t = if eq {
                            let ms: Vec<TyId> = self.flat_members(ty).into_iter().filter(|&m| m == UNDEFINED || m == NULL).collect();
                            if ms.is_empty() { NEVER } else { self.types.union(&ms) }
                        } else {
                            self.types.without_nullish(ty)
                        };
                        vec![target.with(t)]
                    }
                    ExprKind::Null => {
                        let Some((target, ty)) = self.target_of(subject) else { return Vec::new() };
                        let has = self.flat_members(ty).contains(&NULL);
                        let t = if eq {
                            if has { NULL } else { NEVER }
                        } else {
                            let ms: Vec<TyId> = self.flat_members(ty).into_iter().filter(|&m| m != NULL).collect();
                            self.types.union(&ms)
                        };
                        vec![target.with(t)]
                    }
                    ExprKind::Undefined => {
                        let Some((target, ty)) = self.target_of(subject) else { return Vec::new() };
                        let t = if eq { if self.types.has_undefined(ty) { UNDEFINED } else { NEVER } } else { self.types.without_undefined(ty) };
                        vec![target.with(t)]
                    }
                    // `r.ok === false`: the variants whose `ok` is `false`
                    ExprKind::Bool(b) => {
                        let b = *b;
                        match &ast.expr(subject).kind {
                            ExprKind::Member { obj, name, optional: false, .. } => self.bool_field_narrowing(*obj, *name, b == eq),
                            ExprKind::Ident(x) => self.linked_narrowing(*x, b == eq).unwrap_or_default(),
                            _ => Vec::new(),
                        }
                    }
                    ExprKind::Str(lit) => {
                        let lit = *lit;
                        let lit_ty = self.types.str_lit(lit);
                        match &ast.expr(subject).kind {
                            // (`t?.type === "op"`: t isn't `undefined` when it holds)
                            ExprKind::Member { obj, name, optional, .. } if !*optional || eq => {
                                let Some((target, ty)) = self.target_of(*obj) else { return Vec::new() };
                                let ty = if *optional { self.types.without_nullish(ty) } else { ty };
                                let members = self.flat_members(ty);
                                let keep: Vec<TyId> = members
                                    .into_iter()
                                    .filter(|&m| match *self.types.get(m) {
                                        Ty::Record(fs) => match self.types.fields(fs).iter().find(|f| f.name == *name) {
                                            Some(f) if matches!(self.types.get(f.ty), Ty::StrLit(_)) => (f.ty == lit_ty) == eq,
                                            _ => true,
                                        },
                                        _ => true,
                                    })
                                    .collect();
                                vec![target.with(self.types.union(&keep))]
                            }
                            ExprKind::Typeof(x) => {
                                let Some((target, ty)) = self.target_of(*x) else { return Vec::new() };
                                let kind = self.name(lit).to_string();
                                let members = self.flat_members(ty);
                                let mut keep: Vec<TyId> = members.iter().copied().filter(|&m| m != UNKNOWN && (typeof_name(&self.types, m) == Some(kind.as_str())) == eq).collect();
                                // (`unknown`: the type of that kind, or still `unknown` where it isn't)
                                if members.contains(&UNKNOWN) {
                                    let of_kind = match kind.as_str() {
                                        "number" => Some(F64),
                                        "string" => Some(STR),
                                        "boolean" => Some(BOOL),
                                        "undefined" => Some(UNDEFINED),
                                        "bigint" => Some(BIGINT),
                                        _ => None,
                                    };
                                    match (eq, of_kind) {
                                        (true, Some(t)) => keep.push(t),
                                        _ => keep.push(UNKNOWN),
                                    }
                                }
                                vec![target.with(self.types.union(&keep))]
                            }
                            _ => {
                                let Some((target, ty)) = self.target_of(subject) else { return Vec::new() };
                                let members = self.flat_members(ty);
                                if !members.iter().all(|&m| matches!(self.types.get(m), Ty::StrLit(_)) || m == UNDEFINED) {
                                    if eq {
                                        return vec![target.with(lit_ty)];
                                    }
                                    return Vec::new();
                                }
                                let keep: Vec<TyId> = members.into_iter().filter(|&m| (m == lit_ty) == eq).collect();
                                vec![target.with(self.types.union(&keep))]
                            }
                        }
                    }
                    _ => Vec::new(),
                }
            }
            // `"radius" in shape`: the members that have it (or may), or that don't (or may not)
            ExprKind::Binary(BinOp::In, l, r) => {
                let ExprKind::Str(key) = ast.expr(*l).kind else { return Vec::new() };
                let Some((target, ty)) = self.target_of(*r) else { return Vec::new() };
                let mut keep = Vec::new();
                for m in self.flat_members(ty) {
                    match self.has_key(m, key) {
                        Some(has) if has != positive => {}
                        _ => keep.push(m),
                    }
                }
                vec![target.with(self.types.union(&keep))]
            }
            ExprKind::Binary(BinOp::Instanceof, l, r) => {
                let ExprKind::Ident(s) = ast.expr(*r).kind else { return Vec::new() };
                let Some(Decl::Class(c)) = self.scopes[self.cur as usize].values.get(&s).map(|d| d.0) else { return Vec::new() };
                if self.lookup(s).is_some() {
                    return Vec::new();
                }
                let Some((target, ty)) = self.target_of(*l) else { return Vec::new() };
                let mut keep = Vec::new();
                for m in self.flat_members(ty) {
                    // (`unknown` that's a `C`: one, if `C` isn't generic)
                    if m == UNKNOWN {
                        self.resolve_class(c);
                        if positive && self.classes[c as usize].params.is_empty() {
                            keep.push(self.classes[c as usize].this_ty);
                        } else {
                            keep.push(m);
                        }
                        continue;
                    }
                    let Some((mc, _)) = self.class_of(m) else {
                        if !positive {
                            keep.push(m);
                        }
                        continue;
                    };
                    let is_c = self.class_descends(mc, c);
                    if positive {
                        if is_c {
                            keep.push(m);
                        } else if self.class_descends(c, mc) {
                            let d = self.downcast_ty(m, c);
                            keep.push(d);
                        }
                    } else if !is_c {
                        keep.push(m);
                    }
                }
                vec![target.with(self.types.union(&keep))]
            }
            // `isFish(pet)` where `isFish(p: ...): p is Fish`: pet is a `Fish`, or not
            ExprKind::Call { callee, args, .. } if self.predicate_of(*callee).is_some() => {
                let (pi, pt) = self.predicate_of(*callee).unwrap();
                let Some(a) = args.get(pi) else { return Vec::new() };
                let Some((target, ty)) = self.target_of(a.expr) else { return Vec::new() };
                let members = self.flat_members(ty);
                let keep: Vec<TyId> = if positive {
                    let fits: Vec<TyId> = members.iter().copied().filter(|&m| self.assignable(m, pt)).collect();
                    if fits.is_empty() { vec![pt] } else { fits }
                } else {
                    members.into_iter().filter(|&m| !self.assignable(m, pt)).collect()
                };
                vec![target.with(self.types.union(&keep))]
            }
            // `Array.isArray(x)`: the array members of x's union, or the others
            ExprKind::Call { callee, args, .. } if args.len() == 1 && self.is_array_is_array(*callee) => {
                let Some((target, ty)) = self.target_of(args[0].expr) else { return Vec::new() };
                let keep: Vec<TyId> = self.flat_members(ty).into_iter().filter(|&m| matches!(self.types.get(m), Ty::Array(_)) == positive).collect();
                vec![target.with(self.types.union(&keep))]
            }
            // `if (r.ok)`: the variants whose `ok` is `true` (`false` when it fails)
            ExprKind::Member { obj, name, optional: false, .. } if self.bool_discriminant(*obj, *name) => self.bool_field_narrowing(*obj, *name, positive),
            // `const { done, value } = it.next(); if (done) break`: `value` is what it yields
            ExprKind::Ident(x) if let Some(v) = self.linked_narrowing(*x, positive) => v,
            // `if (m?.groups)`: `m` is there, and its `groups`
            ExprKind::Member { obj, optional: true, .. } if positive => {
                let mut v = Vec::new();
                if let Some((target, ty)) = self.target_of(*obj)
                    && self.types.has_nullish(ty)
                {
                    let t = self.types.without_nullish(ty);
                    v.push(target.with(t));
                }
                if let Some((target, ty)) = self.target_of(e)
                    && self.types.has_nullish(ty)
                {
                    let t = self.types.without_nullish(ty);
                    v.push(target.with(t));
                }
                v
            }
            ExprKind::Ident(_) | ExprKind::Member { optional: false, .. } => {
                let Some((target, ty)) = self.target_of(e) else { return Vec::new() };
                if !self.types.has_nullish(ty) {
                    return Vec::new();
                }
                // (falsy: `undefined`/`null`, or `0` / `""` / `false` when it may hold one of those)
                let inner = self.types.without_nullish(ty);
                let can_be_falsy = self.flat_members(inner).iter().any(|&m| self.types.is_primitive(m) || m == BOOL);
                if !positive && can_be_falsy {
                    return Vec::new();
                }
                let t = if positive {
                    inner
                } else {
                    let ms: Vec<TyId> = self.flat_members(ty).into_iter().filter(|&m| m == UNDEFINED || m == NULL).collect();
                    self.types.union(&ms)
                };
                vec![target.with(t)]
            }
            _ => Vec::new(),
        }
    }

    /// `x.name` where x is a union of records, some with `name` a `true` or `false`.
    fn bool_discriminant(&mut self, obj: ExprId, name: Sym) -> bool {
        let Some((_, ty)) = self.target_of(obj) else { return false };
        let members = self.flat_members(ty);
        members.len() > 1
            && members.iter().any(|&m| match *self.types.get(m) {
                Ty::Record(fs) => self.types.fields(fs).iter().any(|f| f.name == name && matches!(self.types.get(f.ty), Ty::BoolLit(_))),
                _ => false,
            })
    }

    /// A generic function value of type `ft` (type parameters `tparams`) where `want` is wanted:
    /// the instance `want` fixes, if it fixes them all (recorded for code generation).
    fn generic_value(&mut self, e: ExprId, ft: TyId, tparams: &[u32], want: TyId) -> Option<TyId> {
        let want = self.types.without_undefined(want);
        if !matches!(self.types.get(want), Ty::Func(..)) {
            return None;
        }
        let mut map: HashMap<u32, TyId> = HashMap::default();
        self.unify(ft, want, tparams, &mut map);
        if !tparams.iter().all(|p| map.contains_key(p)) {
            return None;
        }
        let t = self.types.subst(ft, &map);
        let targs: Vec<(u32, TyId)> = tparams.iter().map(|p| (*p, map[p])).collect();
        if let Some(f) = self.facts_mut() {
            f.value_targs.insert(e, targs);
        }
        Some(t)
    }

    /// `x` from `const { x, y } = r` (`const x = $t.x`) tested for `value`, where `x` is a boolean
    /// discriminant of `$t`'s union: `$t` narrowed so, and its other destructured locals with it.
    fn linked_narrowing(&mut self, x: Sym, value: bool) -> Option<Vec<Narrowing>> {
        let (id, _) = self.lookup(x)?;
        let (_, obj, field) = *self.fcx.last()?.field_aliases.iter().find(|a| a.0 == id)?;
        if !self.bool_discriminant(obj, field) {
            return None;
        }
        let mut out = self.bool_field_narrowing(obj, field, value);
        let Some(Narrowing::Local(tmp, tty)) = out.first().map(|n| match n {
            Narrowing::Local(l, t) => Narrowing::Local(*l, *t),
            _ => Narrowing::Local(usize::MAX, ERROR),
        }) else { return None };
        if tmp == usize::MAX {
            return None;
        }
        let siblings: Vec<(LocalId, Sym)> = self
            .fcx
            .last()?
            .field_aliases
            .iter()
            .filter(|a| a.0 != id && a.2 != field && matches!(self.ast().expr(a.1).kind, ExprKind::Ident(t) if self.lookup(t).map(|l| l.0) == Some(tmp)))
            .map(|a| (a.0, a.2))
            .collect();
        for (sib, g) in siblings {
            let tys: Option<Vec<TyId>> = self.flat_members(tty).into_iter().map(|m| self.field_of(m, g)).collect();
            if let Some(tys) = tys {
                let t = self.types.union(&tys);
                out.push(Narrowing::Local(sib, t));
            }
        }
        Some(out)
    }

    /// x narrowed to the members whose field `name` can be `value`.
    fn bool_field_narrowing(&mut self, obj: ExprId, name: Sym, value: bool) -> Vec<Narrowing> {
        let Some((target, ty)) = self.target_of(obj) else { return Vec::new() };
        let lit = self.types.bool_lit(!value);
        let keep: Vec<TyId> = self
            .flat_members(ty)
            .into_iter()
            .filter(|&m| match *self.types.get(m) {
                Ty::Record(fs) => !self.types.fields(fs).iter().any(|f| f.name == name && f.ty == lit),
                _ => true,
            })
            .collect();
        vec![target.with(self.types.union(&keep))]
    }

    /// Is this callee the built-in `Array.isArray`?
    fn is_array_is_array(&self, callee: ExprId) -> bool {
        let ExprKind::Member { obj, name, .. } = &self.ast().expr(callee).kind else { return false };
        let ExprKind::Ident(s) = self.ast().expr(*obj).kind else { return false };
        self.name(s) == "Array" && self.name(*name) == "isArray" && self.lookup(s).is_none() && !self.scopes[self.cur as usize].values.contains_key(&s)
    }

    /// A narrowable reference: a local, or a field path of a local (`t.left.value`), with its current type.
    fn target_of(&mut self, e: ExprId) -> Option<(Target, TyId)> {
        match &self.ast().expr(e).kind {
            ExprKind::Ident(s) => {
                if let Some((id, ty)) = self.lookup(*s) {
                    return Some((Target { local: id, path: Vec::new(), heap: false, global: None }, ty));
                }
                // a module constant (not a `let`: a function could reassign that)
                let Some(Decl::Const(m, i)) = self.scopes[self.cur as usize].values.get(s).map(|d| d.0) else { return None };
                if !matches!(self.modules[m as usize].ast.items[i as usize].kind, ItemKind::Const { mutable: false, .. }) {
                    return None;
                }
                let ty = match self.lookup_global((m, i)) {
                    Some(t) => t,
                    None => self.const_type(m, i),
                };
                Some((Target { local: 0, path: Vec::new(), heap: false, global: Some((m, i)) }, ty))
            }
            ExprKind::This => self.lookup(super::THIS_SYM).map(|(id, ty)| (Target { local: id, path: Vec::new(), heap: false, global: None }, ty)),
            ExprKind::Paren(x) => self.target_of(*x),
            // `(m = re.exec(s)) !== null`: the variable, as just assigned
            ExprKind::Assign(crate::ast::AssignOp::Assign, l, _) if matches!(self.ast().expr(*l).kind, ExprKind::Ident(_)) => self.target_of(*l),
            ExprKind::Member { obj, name, optional, .. } => {
                let (mut target, base) = self.target_of(*obj)?;
                // (a module constant's contents can change under it: only its own value narrows)
                if target.global.is_some() {
                    return None;
                }
                target.path.push(*name);
                // (`x?.f`: the field where `x` is there)
                let base = if *optional { self.types.without_nullish(base) } else { self.types.without_undefined(base) };
                let members = self.flat_members(base);
                // Through a class instance: other references can change it (calls forget it); getters aren't narrowable.
                for &m in &members {
                    if self.class_of(m).is_some() {
                        target.heap = true;
                        if !matches!(self.class_member(m, *name), Some(super::class::ClassMemberRef::Field(_))) {
                            return None;
                        }
                    }
                }
                if let Some(t) = self.lookup_path(target.local, &target.path) {
                    return Some((target, t));
                }
                let mut tys = Vec::new();
                for m in members {
                    tys.push(self.field_of(m, *name)?);
                }
                let t = self.types.union(&tys);
                Some((target, t))
            }
            _ => None,
        }
    }

    /// Forgets narrowings at or under an assigned place (`t.left = x` invalidates `t.left.value`).
    pub(super) fn invalidate_place(&mut self, e: ExprId) {
        let mut path = Vec::new();
        let mut cur = e;
        loop {
            match &self.ast().expr(cur).kind {
                ExprKind::Paren(x) | ExprKind::NonNull(x) => cur = *x,
                ExprKind::Member { obj, name, .. } => {
                    path.push(*name);
                    cur = *obj;
                }
                ExprKind::Index { obj, .. } => {
                    // An element changed: forget everything under the collection's path.
                    path.clear();
                    cur = *obj;
                }
                ExprKind::Ident(s) => {
                    if let Some((id, _)) = self.lookup(*s) {
                        path.reverse();
                        self.narrow_path(id, path, None);
                    }
                    return;
                }
                _ => return,
            }
        }
    }
}

/// A reference that can be narrowed.
#[derive(Clone)]
pub(super) struct Target {
    local: LocalId,
    path: Vec<Sym>,
    /// The path goes through a class instance.
    heap: bool,
    /// A module constant (module, item), not a local.
    global: Option<(u32, u32)>,
}

impl Target {
    fn with(self, ty: TyId) -> Narrowing {
        if let Some(key) = self.global {
            return Narrowing::Global(key, ty);
        }
        if self.path.is_empty() { Narrowing::Local(self.local, ty) } else { Narrowing::Path(self.local, self.path, ty, self.heap) }
    }
}

pub(super) enum Narrowing {
    Local(LocalId, TyId),
    Path(LocalId, Vec<Sym>, TyId, bool),
    Global((u32, u32), TyId),
}

fn typeof_name(types: &Types, t: TyId) -> Option<&'static str> {
    if types.is_numeric(t) {
        Some("number")
    } else if t == BIGINT {
        Some("bigint")
    } else if t == DECIMAL {
        Some("decimal")
    } else if types.is_string(t) {
        Some("string")
    } else if t == BOOL {
        Some("boolean")
    } else if t == UNDEFINED {
        Some("undefined")
    } else if matches!(types.get(t), Ty::Func(..)) {
        Some("function")
    } else {
        Some("object")
    }
}

fn int_range(t: TyId) -> Option<(i128, i128)> {
    Some(match t {
        INT => (i64::MIN as i128, i64::MAX as i128),
        I8 => (i8::MIN as i128, i8::MAX as i128),
        I16 => (i16::MIN as i128, i16::MAX as i128),
        I32 => (i32::MIN as i128, i32::MAX as i128),
        U8 | U8C => (0, u8::MAX as i128),
        U16 => (0, u16::MAX as i128),
        U32 => (0, u32::MAX as i128),
        U64 => (0, u64::MAX as i128),
        _ => return None,
    })
}

/// A sized integer as arithmetic sees it: `int` (all but `u64` fit).
pub(super) fn promoted(t: TyId) -> TyId {
    if matches!(t, I8 | I16 | I32 | U8 | U8C | U16 | U32) { INT } else { t }
}

impl<'a> Checker<'a> {
    /// `xs[i]` (through parentheses and `!`): an element of an array.
    fn is_elem_place(&self, e: ExprId) -> bool {
        match &self.ast().expr(e).kind {
            ExprKind::Paren(x) | ExprKind::NonNull(x) => self.is_elem_place(*x),
            ExprKind::Index { .. } => true,
            _ => false,
        }
    }

    /// Element types `new Array(n)` can fill with an empty value (numbers, booleans, strings).
    /// `x.method()` is called on `x` (with no arguments).
    fn receiver_of(&self, x: ExprId, method: &str) -> bool {
        let ast = self.ast();
        let sp = ast.expr(x).span;
        ast.exprs.iter().any(|c| {
            matches!(&c.kind, ExprKind::Call { callee, args, .. } if args.is_empty()
                && c.span.file == sp.file && c.span.start == sp.start
                && matches!(&ast.expr(*callee).kind, ExprKind::Member { obj, name, .. } if *obj == x && self.name(*name) == method))
        })
    }

    /// `new Array(n)` that `.fill(v)` fills at once.
    fn filled_at_once(&self, ne: ExprId) -> bool {
        let ast = self.ast();
        let sp = ast.expr(ne).span;
        ast.exprs.iter().any(|x| {
            matches!(&x.kind, ExprKind::Call { callee, args, .. } if args.len() == 1
                && x.span.file == sp.file && x.span.start == sp.start
                && matches!(&ast.expr(*callee).kind, ExprKind::Member { obj, name, .. } if *obj == ne && self.name(*name) == "fill"))
        })
    }

    fn has_default(&self, t: TyId) -> bool {
        self.types.is_numeric(t) || matches!(*self.types.get(t), Ty::Bool | Ty::Str)
    }

    /// `type Tree = (Tree | undefined)[]` (a recursive alias, not generic, for an array): the
    /// array, which is what it's held as.
    pub(super) fn array_alias(&mut self, t: TyId) -> TyId {
        if let Ty::Rec(_, args) = *self.types.get(t)
            && self.types.tys(args).is_empty()
        {
            let body = self.unfold(t);
            if matches!(self.types.get(body), Ty::Array(_)) {
                return body;
            }
        }
        t
    }

    /// A tuple type's elements (`[A, B]`: a record with fields `0`, `1`), in order.
    pub(super) fn tuple_of(&self, t: TyId) -> Option<Vec<Field>> {
        let Ty::Record(fs) = *self.types.get(t) else { return None };
        crate::types::tuple_fields(self.types.fields(fs), self.interner)
    }
}

/// The capturing groups of a regular expression's source: `(`, `(?<name>`, not `(?:` or a
/// lookaround, not escaped or in a class.
fn capture_groups(src: &str) -> usize {
    let b = src.as_bytes();
    let (mut i, mut n, mut class) = (0, 0, false);
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'[' => class = true,
            b']' => class = false,
            b'(' if !class => {
                if b.get(i + 1) != Some(&b'?') || (b.get(i + 2) == Some(&b'<') && !matches!(b.get(i + 3), Some(b'=') | Some(b'!'))) {
                    n += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    n
}
