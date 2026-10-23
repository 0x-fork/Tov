use crate::ast::*;
use crate::diag::Diagnostic;
use crate::intern::{Interner, Sym};
use crate::lexer::{self, Tok, Token};
use crate::source::{FileId, Span};

pub fn parse(src: &str, file: FileId, interner: &mut Interner) -> (Ast, Vec<Diagnostic>) {
    // (the name of a spread's field, which checking may also make: see `check::rewrite`; and
    // the fields of a generator's `next()` result)
    interner.intern("...");
    interner.intern("value");
    interner.intern("done");
    let mut diags = Vec::new();
    let mut lexed = lexer::lex(src, file, &mut diags);
    // `type` is a keyword only where a declaration starts (`type T = ...`, `import type { ... }`,
    // `import { type T }`: a name or `{` follows), and elsewhere a name, as in TypeScript, so
    // `function f(type: string)` and `const { type } = x` work
    for i in 0..lexed.tokens.len() {
        if lexed.tokens[i].kind == Tok::Type && !matches!(lexed.tokens.get(i + 1).map(|t| t.kind), Some(Tok::Ident | Tok::LBrace)) {
            lexed.tokens[i].kind = Tok::Ident;
        }
    }
    let mut p = Parser { src, toks: lexed.tokens, strings: lexed.strings, pos: 0, file, interner, ast: Ast::default(), diags, script_stmts: Vec::new(), defaults: Vec::new(), more_stmts: Vec::new(), hidden: 0, prologue: Vec::new(), index_sig: None, call_sig: None, local_fns: Vec::new(), prop_defaults: false, extra_members: Vec::new(), generic_arrows: Vec::new(), local_types: Vec::new(), local_type_items: Vec::new(), runtime_tests: false, chai: false, lifted: Vec::new() };
    p.ast.iterator_sym = Some(p.interner.intern("__iterator"));
    // (`// @ts-expect-error`: the next line's start)
    let mut at = 0usize;
    for line in src.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("//") && (t.contains("@ts-expect-error") || t.contains("@ts-ignore")) {
            p.ast.ts_expect.push((at + line.len()) as u32);
        }
        at += line.len();
    }
    p.module();
    p.lift_functions();
    p.runtime_tests();
    if p.chai {
        p.chai_assertions();
    }
    p.unhide_local_types();
    slot_records(&mut p.ast, p.interner);
    (p.ast, p.diags)
}

/// A local `const row: Record<string, V> = {}` used only as a bag of named values — written
/// with `row[k] = v`, read only with names (`row.region`, `row["units"]`) — is a record with an
/// optional field per name read: `row[k] = v` becomes `k === "region" ? (row.region = v) : ...
/// : v` (keys nothing reads are dropped: nothing can see them), and each read the field, `!`
/// where a missing key would have trapped. No map is made, filled, searched or freed.
fn slot_records(ast: &mut Ast, interner: &mut Interner) {
    let (Some(record), Some(string)) = (interner.lookup("Record"), interner.lookup("string")) else { return };
    let cands: Vec<(StmtId, Sym, Span, TypeId)> = ast
        .stmts
        .iter()
        .enumerate()
        .filter_map(|(i, st)| match &st.kind {
            StmtKind::Let { name, name_span, ty: Some(t), init: Some(e), .. } => {
                let TypeExprKind::Named { ns: None, name: r, args, .. } = &ast.types[*t as usize].kind else { return None };
                let key_is_string = args.len() == 2 && matches!(ast.types[args[0] as usize].kind, TypeExprKind::Named { ns: None, name: k, ref args, .. } if k == string && args.is_empty());
                (*r == record && key_is_string && matches!(&ast.exprs[*e as usize].kind, ExprKind::Object(fs) if fs.is_empty())).then(|| (i as StmtId, *name, *name_span, args[1]))
            }
            _ => None,
        })
        .collect();
    if cands.is_empty() {
        return;
    }
    // each expression's parent, and the conditions statements test
    let mut parent = vec![u32::MAX; ast.exprs.len()];
    for (i, e) in ast.exprs.iter().enumerate() {
        let mut kid = |c: ExprId| parent[c as usize] = i as u32;
        match &e.kind {
            ExprKind::Unary(_, x) | ExprKind::Paren(x) | ExprKind::NonNull(x) | ExprKind::Typeof(x) | ExprKind::Spread(x) | ExprKind::As(x, _) | ExprKind::Try(x) | ExprKind::Await(x) | ExprKind::Yield(Some(x), _) => kid(*x),
            ExprKind::Binary(_, a, b) | ExprKind::Assign(_, a, b) => {
                kid(*a);
                kid(*b);
            }
            ExprKind::Update { target, .. } => kid(*target),
            ExprKind::Call { callee, args, .. } | ExprKind::New { callee, args, .. } => {
                kid(*callee);
                args.iter().for_each(|a| kid(a.expr));
            }
            ExprKind::Member { obj, .. } => kid(*obj),
            ExprKind::Index { obj, index, .. } => {
                kid(*obj);
                kid(*index);
            }
            ExprKind::Cond(a, b, c) => {
                kid(*a);
                kid(*b);
                kid(*c);
            }
            ExprKind::Array(xs) | ExprKind::Template(_, xs) | ExprKind::Seq(xs) => xs.iter().for_each(|&x| kid(x)),
            ExprKind::Object(fs) => fs.iter().for_each(|f| kid(f.value)),
            ExprKind::Arrow(f) => {
                if let ArrowBody::Expr(x) = f.body {
                    kid(x)
                }
            }
            _ => {}
        }
    }
    let mut tested: crate::hash::FxSet<ExprId> = Default::default();
    for st in &ast.stmts {
        match &st.kind {
            StmtKind::If(c, ..) | StmtKind::While(c, _) | StmtKind::DoWhile(_, c) | StmtKind::For { cond: Some(c), .. } => {
                tested.insert(*c);
            }
            _ => {}
        }
    }
    let within = |a: Span, b: Span| a.file == b.file && a.start >= b.start && a.end <= b.end;
    // (the parent, through parentheses)
    fn up(ast: &Ast, parent: &[u32], mut e: ExprId) -> Option<ExprId> {
        loop {
            let p = parent[e as usize];
            if p == u32::MAX {
                return None;
            }
            if !matches!(ast.exprs[p as usize].kind, ExprKind::Paren(_)) {
                return Some(p);
            }
            e = p;
        }
    }
    // a read where a missing key is a value (`row.x ?? d`, `if (row.x)`, `row.x === undefined`)
    let optional_ctx = |ast: &Ast, parent: &[u32], tested: &crate::hash::FxSet<ExprId>, e: ExprId| -> bool {
        if tested.contains(&e) {
            return true;
        }
        let Some(p) = up(ast, parent, e) else { return false };
        match &ast.exprs[p as usize].kind {
            ExprKind::Binary(BinOp::Nullish, l, _) => *l == e || matches!(ast.exprs[*l as usize].kind, ExprKind::Paren(x) if x == e),
            ExprKind::Binary(BinOp::And | BinOp::Or, ..) | ExprKind::Unary(UnOp::Not, _) | ExprKind::Typeof(_) => true,
            ExprKind::Binary(BinOp::Eq | BinOp::Ne | BinOp::LooseEq | BinOp::LooseNe, a, b) => {
                let other = if *a == e { *b } else { *a };
                matches!(ast.exprs[other as usize].kind, ExprKind::Undefined | ExprKind::Null)
            }
            ExprKind::Cond(c, ..) => *c == e,
            ExprKind::Member { optional: true, .. } | ExprKind::Index { optional: true, .. } => true,
            _ => false,
        }
    };
    for (let_id, name, name_span, vty) in cands {
        // the block the binding lives in
        let Some(block) = ast.stmts.iter().find(|st| matches!(&st.kind, StmtKind::Block(ss) if ss.contains(&let_id))).map(|st| st.span) else { continue };
        if !within(ast.stmts[let_id as usize].span, block) {
            continue;
        }
        let after = Span::new(name_span.file, name_span.end, block.end);
        // (another binding of the name inside: give up rather than tell them apart)
        let shadowed = ast.stmts.iter().enumerate().any(|(i, st)| {
            i as StmtId != let_id
                && within(st.span, block)
                && matches!(&st.kind, StmtKind::Let { name: n, .. } | StmtKind::ForOf { name: n, .. } if *n == name)
        }) || ast.exprs.iter().any(|e| matches!(&e.kind, ExprKind::Arrow(f) if within(e.span, block) && f.params.iter().any(|p| p.name == name)));
        if shadowed {
            continue;
        }
        enum Use { Set(ExprId, ExprId, ExprId), Read(ExprId, Sym, bool), Write(ExprId) }
        let mut uses = Vec::new();
        let mut ok = true;
        for (i, e) in ast.exprs.iter().enumerate() {
            if !matches!(e.kind, ExprKind::Ident(n) if n == name) || !within(e.span, after) {
                continue;
            }
            let i = i as ExprId;
            let Some(p) = up(ast, &parent, i) else {
                ok = false;
                break;
            };
            let target_of = |x: ExprId| up(ast, &parent, x).filter(|&g| matches!(ast.exprs[g as usize].kind, ExprKind::Assign(_, t, _) if t == x));
            // (`row.x++`, `row.get(k)`: not a plain read)
            let plain_read = |x: ExprId| !matches!(up(ast, &parent, x).map(|g| &ast.exprs[g as usize].kind), Some(ExprKind::Update { .. })) && !matches!(up(ast, &parent, x).map(|g| &ast.exprs[g as usize].kind), Some(ExprKind::Call { callee, .. }) if *callee == x);
            match ast.exprs[p as usize].kind {
                ExprKind::Index { obj, index, optional: false } if obj == i => {
                    let lit = match ast.exprs[index as usize].kind {
                        ExprKind::Str(k) => Some(k),
                        _ => None,
                    };
                    match (target_of(p).map(|g| (g, &ast.exprs[g as usize].kind)), lit) {
                        (Some((g, ExprKind::Assign(AssignOp::Assign, _, v))), None) => {
                            let simple = matches!(ast.exprs[index as usize].kind, ExprKind::Ident(_));
                            if !simple || !crate::check::rewrite::copyable(ast, *v) {
                                ok = false;
                                break;
                            }
                            uses.push(Use::Set(g, index, *v));
                        }
                        (Some((_, ExprKind::Assign(AssignOp::Assign, ..))), Some(_)) => uses.push(Use::Write(p)),
                        (None, Some(k)) if plain_read(p) => {
                            uses.push(Use::Read(p, k, optional_ctx(ast, &parent, &tested, p)));
                        }
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
                ExprKind::Member { obj, name: k, optional: false, .. } if obj == i => match target_of(p).map(|g| &ast.exprs[g as usize].kind) {
                    Some(ExprKind::Assign(AssignOp::Assign, ..)) => uses.push(Use::Write(p)),
                    Some(_) => {
                        ok = false;
                        break;
                    }
                    None if !plain_read(p) => {
                        ok = false;
                        break;
                    }
                    None => uses.push(Use::Read(p, k, optional_ctx(ast, &parent, &tested, p))),
                },
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let mut keys: Vec<Sym> = Vec::new();
        for u in &uses {
            let k = match u {
                Use::Read(_, k, _) => *k,
                Use::Write(p) => match ast.exprs[*p as usize].kind {
                    ExprKind::Member { name: k, .. } => k,
                    ExprKind::Index { index, .. } => match ast.exprs[index as usize].kind {
                        ExprKind::Str(k) => k,
                        _ => continue,
                    },
                    _ => continue,
                },
                Use::Set(..) => continue,
            };
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        if keys.len() > 16 {
            continue;
        }
        // (the record type)
        let fields: Vec<FieldTy> = keys.iter().map(|&k| FieldTy { name: k, name_span, optional: true, ty: vty }).collect();
        let tspan = ast.types[vty as usize].span;
        ast.types.push(TypeExpr { kind: TypeExprKind::Record(fields), span: tspan });
        let rec_ty = ast.types.len() as TypeId - 1;
        if let StmtKind::Let { ty, .. } = &mut ast.stmts[let_id as usize].kind {
            *ty = Some(rec_ty);
        }
        let push = |ast: &mut Ast, kind: ExprKind, span: Span| -> ExprId {
            ast.exprs.push(Expr { kind, span });
            ast.exprs.len() as ExprId - 1
        };
        for u in uses {
            match u {
                Use::Read(p, k, opt) => {
                    let span = ast.exprs[p as usize].span;
                    let obj = match ast.exprs[p as usize].kind {
                        ExprKind::Member { obj, .. } | ExprKind::Index { obj, .. } => obj,
                        _ => continue,
                    };
                    let m = push(ast, ExprKind::Member { obj, name: k, name_span: span, optional: false }, span);
                    ast.exprs[p as usize].kind = if opt { ExprKind::Paren(m) } else { ExprKind::NonNull(m) };
                }
                Use::Write(p) => {
                    if let ExprKind::Index { obj, index, .. } = ast.exprs[p as usize].kind
                        && let ExprKind::Str(k) = ast.exprs[index as usize].kind
                    {
                        let span = ast.exprs[p as usize].span;
                        ast.exprs[p as usize].kind = ExprKind::Member { obj, name: k, name_span: span, optional: false };
                    }
                }
                Use::Set(g, key, value) => {
                    let span = ast.exprs[g as usize].span;
                    let ExprKind::Assign(_, target, _) = ast.exprs[g as usize].kind else { continue };
                    let ExprKind::Index { obj, .. } = ast.exprs[target as usize].kind else { continue };
                    // (no key matched: the value, evaluated all the same)
                    let mut chain = value;
                    for &k in keys.iter().rev() {
                        let Some(v) = crate::check::rewrite::deep_copy_pub(ast, value) else { continue };
                        let o = push(ast, ExprKind::Ident(name), ast.exprs[obj as usize].span);
                        let m = push(ast, ExprKind::Member { obj: o, name: k, name_span: span, optional: false }, span);
                        let set = push(ast, ExprKind::Assign(AssignOp::Assign, m, v), span);
                        let kc = push(ast, ast_copy_ident(&ast.exprs[key as usize].kind), ast.exprs[key as usize].span);
                        let lit = push(ast, ExprKind::Str(k), span);
                        let test = push(ast, ExprKind::Binary(BinOp::Eq, kc, lit), span);
                        ast.quiet_tests.push(test);
                        chain = push(ast, ExprKind::Cond(test, set, chain), span);
                    }
                    let kind = std::mem::replace(&mut ast.exprs[chain as usize].kind, ExprKind::Error);
                    ast.exprs[g as usize].kind = kind;
                }
            }
        }
    }
    fn ast_copy_ident(k: &ExprKind) -> ExprKind {
        match k {
            ExprKind::Ident(n) => ExprKind::Ident(*n),
            _ => ExprKind::Error,
        }
    }
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    strings: Vec<String>,
    pos: usize,
    file: FileId,
    interner: &'a mut Interner,
    ast: Ast,
    diags: Vec<Diagnostic>,
    /// Top-level statements: (index of the next item, statement).
    script_stmts: Vec<(usize, StmtId)>,
    /// Default parameter values of the parameter list just parsed, for its body: (name, its
    /// span, its type, the default).
    defaults: Vec<(Sym, Span, Option<TypeId>, ExprId)>,
    /// `let a = 1, b = 2`: the declarations after the first, for the statement list that the
    /// statement is in.
    more_stmts: Vec<StmtId>,
    /// Hidden variables made so far (`__d1`, ... for destructuring).
    hidden: u32,
    /// Statements destructuring the parameters of the list just parsed, for its body.
    prologue: Vec<StmtId>,
    /// The index signature `[k: K]: V` of the object type just parsed.
    index_sig: Option<(TypeId, TypeId, Span)>,
    /// `{ (x: number): string }`: a call signature seen in the record type being read, the
    /// function type it is.
    call_sig: Option<(TypeId, Span)>,
    /// Rewriting tests declared in a function a suite calls (see `runtime_tests`): named inside
    /// the suites running, not by the suites written around them.
    runtime_tests: bool,
    /// `import { expect } from "chai"`: its assertions (`expect(x).to.equal(y)`) are rewritten as
    /// the built-in matchers.
    chai: bool,
    /// Function declarations in bodies, as `const`s (to hoist).
    local_fns: Vec<StmtId>,
    /// Nested generators and generic functions, made module functions (see `lift_functions`): the
    /// name in its body (none for an expression), the module function's, where it's written.
    lifted: Vec<(Option<Sym>, Sym, Span)>,
    /// A parameter property in the list just parsed has a default value.
    prop_defaults: bool,
    /// Fields made from a constructor's parameter properties, for the class being parsed.
    extra_members: Vec<ClassMember>,
    /// Generic arrow functions (`<T>(x: T) => x`): the arrow, its type parameters, where it starts
    /// (a top-level `const` of one becomes a generic function; any other is an error).
    generic_arrows: Vec<(ExprId, Vec<TypeParam>, Span)>,
    /// `type T = ...` / `interface T { }` in a body: module types under hidden names, which
    /// `T` stands for in the rest of the block (innermost last).
    local_types: Vec<(Sym, Sym)>,
    local_type_items: Vec<(Sym, Sym)>,
}

/// A destructuring pattern (desugared into `let`s).
enum Pat {
    Name(Sym, Span),
    /// Elements (holes are `None`), each with its default, and `...rest`.
    Array(Vec<Option<(Pat, Option<ExprId>)>>, Option<Box<Pat>>, Span),
    /// (field, its span, pattern, default), and `...rest`.
    Object(Vec<(Sym, Span, Pat, Option<ExprId>)>, Option<Box<Pat>>, Span),
}

impl Pat {
    fn span(&self) -> Span {
        match self {
            Pat::Name(_, s) | Pat::Array(_, _, s) | Pat::Object(_, _, s) => *s,
        }
    }
}

impl<'a> Parser<'a> {
    // ---------------------------------------------------------------- tokens

    fn tok(&self) -> Token {
        self.toks[self.pos]
    }

    fn kind(&self) -> Tok {
        self.toks[self.pos].kind
    }

    fn nth(&self, n: usize) -> Tok {
        self.toks.get(self.pos + n).map(|t| t.kind).unwrap_or(Tok::Eof)
    }

    fn at(&self, k: Tok) -> bool {
        self.kind() == k
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos];
        if t.kind != Tok::Eof {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, k: Tok) -> bool {
        if self.at(k) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn span_of(&self, t: Token) -> Span {
        Span::new(self.file, t.start, t.end)
    }

    fn cur_span(&self) -> Span {
        self.span_of(self.tok())
    }

    fn prev_span(&self) -> Span {
        if self.pos == 0 {
            return Span::new(self.file, 0, 0);
        }
        self.span_of(self.toks[self.pos - 1])
    }

    fn text(&self, t: Token) -> &'a str {
        &self.src[t.start as usize..t.end as usize]
    }

    /// The source text of a span (of this file).
    fn src_text(&self, span: Span) -> String {
        self.src.get(span.start as usize..span.end as usize).unwrap_or("").to_string()
    }

    fn at_word(&self, w: &str) -> bool {
        self.at(Tok::Ident) && self.text(self.tok()) == w
    }

    /// True when the current token is glued to the previous one (no whitespace between).
    fn adjacent(&self) -> bool {
        self.pos > 0 && self.toks[self.pos - 1].end == self.tok().start
    }

    fn found(&self) -> String {
        let t = self.tok();
        match t.kind {
            Tok::Eof => "end of file".to_string(),
            _ => format!("`{}`", self.text(t)),
        }
    }

    // ---------------------------------------------------------------- diagnostics

    fn err(&mut self, code: &'static str, span: Span, msg: impl Into<String>) {
        self.diags.push(Diagnostic::new(code, span, msg));
    }

    fn push(&mut self, d: Diagnostic) {
        self.diags.push(d);
    }

    fn expect(&mut self, k: Tok, context: &str) -> bool {
        if self.eat(k) {
            return true;
        }
        let msg = format!("expected {} {}, found {}", k.describe(), context, self.found());
        let span = self.cur_span();
        self.err("P0001", span, msg);
        false
    }

    /// Statement terminator: `;`, a line break, `}` or end of file.
    fn terminator(&mut self) {
        if self.eat(Tok::Semi) || self.at(Tok::RBrace) || self.at(Tok::Eof) || self.tok().nl_before {
            return;
        }
        let msg = format!("expected `;` or a line break after the statement, found {}", self.found());
        let span = self.cur_span();
        self.err("P0002", span, msg);
        self.recover_line();
    }

    /// Skips to the start of the next statement.
    fn recover_line(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.kind() {
                Tok::Eof => return,
                Tok::Semi if depth == 0 => {
                    self.bump();
                    return;
                }
                Tok::RBrace if depth == 0 => return,
                Tok::LBrace | Tok::LParen | Tok::LBracket => depth += 1,
                Tok::RBrace | Tok::RParen | Tok::RBracket => depth -= 1,
                _ => {}
            }
            self.bump();
            if depth <= 0 && self.tok().nl_before {
                return;
            }
        }
    }

    /// Skips an unsupported declaration: up to and including its `{ ... }` body, or to the end of the line.
    fn skip_decl(&mut self) {
        loop {
            match self.kind() {
                Tok::Eof => return,
                Tok::LBrace => {
                    self.skip_balanced();
                    return;
                }
                Tok::Semi => {
                    self.bump();
                    return;
                }
                _ => {
                    self.bump();
                    if self.tok().nl_before && !matches!(self.kind(), Tok::LBrace) {
                        return;
                    }
                }
            }
        }
    }

    fn skip_balanced(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.kind() {
                Tok::Eof => return,
                Tok::LBrace | Tok::LParen | Tok::LBracket => depth += 1,
                Tok::RBrace | Tok::RParen | Tok::RBracket => {
                    depth -= 1;
                    if depth <= 0 {
                        self.bump();
                        return;
                    }
                }
                _ => {}
            }
            self.bump();
        }
    }

    /// Runs `f`; if it fails or reports errors, rewinds as if it never ran.
    fn speculate<T>(&mut self, f: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        let (pos, nd, ne, ns, nt, ndf, npr) = (self.pos, self.diags.len(), self.ast.exprs.len(), self.ast.stmts.len(), self.ast.types.len(), self.defaults.len(), self.prologue.len());
        let ng = self.generic_arrows.len();
        let r = f(self);
        if r.is_some() && self.diags.len() == nd {
            return r;
        }
        self.pos = pos;
        self.generic_arrows.truncate(ng);
        self.defaults.truncate(ndf);
        self.prologue.truncate(npr);
        self.diags.truncate(nd);
        self.ast.exprs.truncate(ne);
        self.ast.stmts.truncate(ns);
        self.ast.types.truncate(nt);
        None
    }

    /// Index of the token closing the paren at `self.pos`.
    fn matching_paren(&self) -> Option<usize> {
        let mut depth = 0i32;
        for i in self.pos..self.toks.len() {
            match self.toks[i].kind {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                Tok::Eof => return None,
                _ => {}
            }
        }
        None
    }

    // ---------------------------------------------------------------- nodes

    fn mk_expr(&mut self, kind: ExprKind, span: Span) -> ExprId {
        self.ast.exprs.push(Expr { kind, span });
        self.ast.exprs.len() as ExprId - 1
    }

    /// `x = d` as a default: `x !== undefined ? x : d` (only `undefined` takes the default,
    /// not `null`; `x` first, so its type guides `d`'s). `x` is a name or a read of one's
    /// field or element, built again for the test.
    fn defaulted(&mut self, x: ExprId, d: ExprId, span: Span) -> ExprId {
        let again = self.copy_read(x);
        let u = self.mk_expr(ExprKind::Undefined, span);
        let test = self.mk_expr(ExprKind::Binary(BinOp::Ne, again, u), span);
        self.mk_expr(ExprKind::Cond(test, x, d), span)
    }

    fn copy_read(&mut self, x: ExprId) -> ExprId {
        let span = self.ast.exprs[x as usize].span;
        let kind = match &self.ast.exprs[x as usize].kind {
            &ExprKind::Ident(n) => ExprKind::Ident(n),
            &ExprKind::Int(i) => ExprKind::Int(i),
            &ExprKind::Member { obj, name, name_span, optional } => ExprKind::Member { obj: self.copy_read(obj), name, name_span, optional },
            &ExprKind::Index { obj, index, optional } => ExprKind::Index { obj: self.copy_read(obj), index: self.copy_read(index), optional },
            _ => unreachable!("a default's target is a name or a read of one"),
        };
        self.mk_expr(kind, span)
    }

    fn mk_stmt(&mut self, kind: StmtKind, span: Span) -> StmtId {
        self.ast.stmts.push(Stmt { kind, span });
        self.ast.stmts.len() as StmtId - 1
    }

    fn mk_ty(&mut self, kind: TypeExprKind, span: Span) -> TypeId {
        self.ast.types.push(TypeExpr { kind, span });
        self.ast.types.len() as TypeId - 1
    }

    fn expr_span(&self, e: ExprId) -> Span {
        self.ast.exprs[e as usize].span
    }

    fn ident(&mut self, context: &str) -> Option<(Sym, Span)> {
        if self.at(Tok::Ident) {
            let t = self.bump();
            let sym = self.interner.intern(self.text(t));
            return Some((sym, self.span_of(t)));
        }
        let msg = if self.kind().is_keyword() {
            format!("expected {context}, found keyword {}", self.found())
        } else {
            format!("expected {context}, found {}", self.found())
        };
        let span = self.cur_span();
        self.err("P0001", span, msg);
        None
    }

    /// Property names may be any identifier or keyword (`s.type`, `{ default: 1 }`).
    fn prop_name(&mut self) -> Option<(Sym, Span)> {
        if self.at(Tok::Ident) || self.kind().is_keyword() {
            let t = self.bump();
            let sym = self.interner.intern(self.text(t));
            return Some((sym, self.span_of(t)));
        }
        None
    }

    // ---------------------------------------------------------------- items

    fn module(&mut self) {
        while !self.at(Tok::Eof) {
            let start = self.pos;
            self.item();
            if self.pos == start {
                self.bump();
            }
        }
        self.finish_script();
    }

    /// A module with top-level statements is a script: its statements, and the initializers of
    /// its module variables, run in source order in a synthesized function `<script>`.
    fn finish_script(&mut self) {
        for (_, _, at) in std::mem::take(&mut self.generic_arrows) {
            self.err("U0015", at, "generic arrow functions are supported as a top-level `const` only; declare a generic `function`");
        }
        let Some(first) = self.ast.script_span else { return };
        let local = self.script_locals();
        let mut body = Vec::new();
        let mut stmt_iter = std::mem::take(&mut self.script_stmts).into_iter().peekable();
        // (the items that stay, renumbered)
        let mut new_index = vec![u32::MAX; self.ast.items.len()];
        let mut kept = 0;
        for (ii, ni) in new_index.iter_mut().enumerate() {
            if !local[ii] {
                *ni = kept;
                kept += 1;
            }
        }
        for ii in 0..self.ast.items.len() {
            // statements that came before this item
            while let Some(&(at, s)) = stmt_iter.peek() {
                if at > ii {
                    break;
                }
                body.push(s);
                stmt_iter.next();
            }
            if let ItemKind::Const { name, name_span, ty, init, mutable } = self.ast.items[ii].kind {
                let span = self.ast.items[ii].span;
                let st = if local[ii] {
                    // (`let m: string;`, given no value: as a function's, read only once assigned)
                    let given = !(mutable && ty.is_some() && matches!(self.ast.exprs[init as usize].kind, ExprKind::Undefined) && self.ast.exprs[init as usize].span == name_span);
                    StmtKind::Let { mutable, name, name_span, ty, init: given.then_some(init) }
                } else {
                    StmtKind::InitGlobal(new_index[ii])
                };
                body.push(self.mk_stmt(st, span));
            }
        }
        body.extend(stmt_iter.map(|(_, s)| s));
        if local.iter().any(|&l| l) {
            let items = std::mem::take(&mut self.ast.items);
            self.ast.items = items.into_iter().zip(&local).filter(|(_, l)| !**l).map(|(it, _)| it).collect();
        }
        // (the whole script: from its first statement to its last)
        let end = body.iter().map(|&b| self.ast.stmts[b as usize].span).filter(|sp| sp.file == first.file).map(|sp| sp.end).max().unwrap_or(first.end);
        let span = Span::new(first.file, first.start, end.max(first.end));
        let block = self.mk_stmt(StmtKind::Block(body), span);
        let name = self.interner.intern("<script>");
        let is_async = top_level_await(&self.ast, block);
        let f = FnDecl { name, name_span: span, tparams: Vec::new(), params: Vec::new(), ret: None, throws: None, body: block, is_async, is_gen: false };
        self.ast.script = Some(self.ast.items.len() as u32);
        self.ast.items.push(Item { kind: ItemKind::Function(f), span, exported: false });
    }

    /// A script's module variables that are only its own: variables of the script function
    /// (`<script>`) rather than of the module, so they live in registers and nothing retains
    /// them around calls. A variable stays the module's when it's exported, or anything but the
    /// script's statements after it mentions it: a function, a class, a type (`typeof x`), the
    /// statements before it, or the initializer of another module variable that stays the
    /// module's (those are evaluated on their own).
    fn script_locals(&self) -> Vec<bool> {
        let items = &self.ast.items;
        // (not one whose value is an object of computed keys: those are checked against the
        // declared type as module constants)
        let mut local: Vec<bool> = items.iter().map(|it| matches!(it.kind, ItemKind::Const { init, .. } if !self.ast.keyed_objects.contains(&init)) && !it.exported).collect();
        if !local.iter().any(|&l| l) {
            return local;
        }
        let names: crate::hash::FxMap<Sym, Vec<usize>> = items.iter().enumerate().filter(|(i, _)| local[*i]).fold(Default::default(), |mut m, (i, it)| {
            if let ItemKind::Const { name, .. } = it.kind {
                m.entry(name).or_default().push(i);
            }
            m
        });
        let within = |a: Span, b: Span| a.file == b.file && a.start >= b.start && a.end <= b.end;
        // where each mention is: an item (its index), or a script statement (the item it precedes)
        enum Owner { Item(usize), Stmt(usize), Unknown }
        let mut uses: Vec<(usize, Owner)> = Vec::new();
        for e in &self.ast.exprs {
            let ExprKind::Ident(sym) = e.kind else { continue };
            let Some(xs) = names.get(&sym) else { continue };
            let owner = if let Some(i) = items.iter().position(|it| within(e.span, it.span)) {
                Owner::Item(i)
            } else if let Some(&(at, _)) = self.script_stmts.iter().find(|(_, st)| within(e.span, self.ast.stmts[*st as usize].span)) {
                Owner::Stmt(at)
            } else {
                Owner::Unknown
            };
            for &x in xs {
                uses.push((x, match owner { Owner::Item(i) => Owner::Item(i), Owner::Stmt(a) => Owner::Stmt(a), Owner::Unknown => Owner::Unknown }));
            }
        }
        loop {
            let mut changed = false;
            for (x, owner) in &uses {
                if !local[*x] {
                    continue;
                }
                // (its own initializer may mention it only as a recursive arrow: a closure
                // that reads it later needs the module's variable)
                let own_arrow = matches!(items[*x].kind, ItemKind::Const { init, .. } if matches!(self.ast.exprs[init as usize].kind, ExprKind::Arrow(_)));
                let ok = match owner {
                    Owner::Item(i) => (*i == *x && own_arrow) || (*i > *x && local[*i]),
                    Owner::Stmt(at) => *at > *x,
                    Owner::Unknown => false,
                };
                if !ok {
                    local[*x] = false;
                    changed = true;
                }
            }
            if !changed {
                return local;
            }
        }
    }

    fn item(&mut self) {
        let start = self.cur_span();
        let mut exported = false;
        if self.at(Tok::Export) {
            self.bump();
            exported = true;
            if self.at(Tok::Default) {
                let dspan = self.cur_span();
                self.bump();
                self.interner.intern("default"); // (the checker looks the export up by it)
                // `export default function f() {}` / `class C {}`: the declaration, which is the default
                let named = match self.kind() {
                    Tok::Function => self.nth(1) == Tok::Ident,
                    Tok::Async => self.nth(1) == Tok::Function && self.nth(2) == Tok::Ident,
                    Tok::Class => self.nth(1) == Tok::Ident,
                    Tok::Abstract => self.nth(1) == Tok::Class && self.nth(2) == Tok::Ident,
                    Tok::Interface => true,
                    _ => false,
                };
                if named {
                    let before = self.ast.items.len();
                    self.item_decl(start, true);
                    let name = self.ast.items.get(before).and_then(|it| match &it.kind {
                        ItemKind::Function(f) => Some((f.name, f.name_span)),
                        ItemKind::Class(c) => Some((c.name, c.name_span)),
                        ItemKind::Interface { name, name_span, .. } => Some((*name, *name_span)),
                        _ => None,
                    });
                    self.ast.default_export = name;
                    return;
                }
                // `export default value`: a name, or a hidden constant holding the value
                let e = self.assign();
                self.terminator();
                if let ExprKind::Ident(n) = self.ast.exprs[e as usize].kind {
                    self.ast.default_export = Some((n, self.expr_span(e)));
                } else {
                    let name = self.interner.intern("__default");
                    let span = start.to(self.expr_span(e));
                    self.ast.items.push(Item { kind: ItemKind::Const { name, name_span: dspan, ty: None, init: e, mutable: false }, span, exported: true });
                    self.ast.default_export = Some((name, dspan));
                }
                return;
            }
            // `export { a, b as c }`, `export type { T }`, either `from "m"`
            if self.at(Tok::LBrace) || (self.at(Tok::Type) && self.nth(1) == Tok::LBrace) {
                self.eat(Tok::Type);
                self.bump(); // {
                let mut names = Vec::new();
                while !self.at(Tok::RBrace) && !self.at(Tok::Eof) {
                    if self.at(Tok::Type) && !matches!(self.nth(1), Tok::Comma | Tok::RBrace) && !(self.nth(1) == Tok::Ident && self.text(self.toks[self.pos + 1]) == "as") {
                        self.bump(); // `{ type T }`
                    }
                    let Some((n, sp)) = self.prop_name() else {
                        let span = self.cur_span();
                        let msg = format!("expected a name in the export list, found {}", self.found());
                        self.err("P0001", span, msg);
                        self.recover_line();
                        return;
                    };
                    let mut out = n;
                    if self.at_word("as") {
                        self.bump();
                        if let Some((a, _)) = self.prop_name() {
                            out = a;
                        }
                    }
                    names.push((n, out, sp));
                    if !self.eat(Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::RBrace, "to close the export list");
                let from = if self.at_word("from") {
                    self.bump();
                    self.reexport_import()
                } else {
                    None
                };
                self.terminator();
                self.ast.exports.push(ExportSpec { from, names, star: false, namespace: None });
                return;
            }
            // `export * from "m"`, `export * as ns from "m"`
            if self.at(Tok::Star) {
                self.bump();
                let namespace = if self.at_word("as") {
                    self.bump();
                    self.prop_name()
                } else {
                    None
                };
                if !self.at_word("from") {
                    let span = self.cur_span();
                    let msg = format!("expected `from` after `export *`, found {}", self.found());
                    self.err("P0001", span, msg);
                    self.recover_line();
                    return;
                }
                self.bump();
                let from = self.reexport_import();
                self.terminator();
                if from.is_some() {
                    self.ast.exports.push(ExportSpec { from, names: Vec::new(), star: namespace.is_none(), namespace });
                }
                return;
            }
        }
        self.item_decl(start, exported);
    }

    /// The module path after an `export ... from`: a hidden import of it (its item index).
    fn reexport_import(&mut self) -> Option<u32> {
        if !self.at(Tok::Str) {
            let span = self.cur_span();
            let msg = format!("expected a module path string, found {}", self.found());
            self.err("P0001", span, msg);
            self.recover_line();
            return None;
        }
        let t = self.bump();
        let path = self.strings[t.val as usize].clone();
        let path_span = self.span_of(t);
        let idx = self.ast.items.len() as u32;
        self.ast.items.push(Item {
            kind: ItemKind::Import(Import { names: Vec::new(), locals: Vec::new(), reexport: true, namespace: None, default: None, path, path_span }),
            span: path_span,
            exported: false,
        });
        Some(idx)
    }

    fn item_decl(&mut self, start: Span, exported: bool) {
        match self.kind() {
            Tok::Import => {
                if exported {
                    self.err("X0013", start, "re-exports are not supported; import from the module that declares the name");
                }
                self.import(start);
            }
            Tok::Function => {
                if let Some(f) = self.function() {
                    let span = start.to(self.prev_span());
                    self.ast.items.push(Item { kind: ItemKind::Function(f), span, exported });
                }
            }
            Tok::Async if self.nth(1) == Tok::Function => {
                self.bump();
                if let Some(mut f) = self.function() {
                    f.is_async = true;
                    let span = start.to(self.prev_span());
                    self.ast.items.push(Item { kind: ItemKind::Function(f), span, exported });
                }
            }
            Tok::Type if self.nth(1) == Tok::Ident => self.type_alias(start, exported),
            Tok::Interface => self.interface(start, exported),
            Tok::Const if self.nth(1) == Tok::Enum => {
                self.bump(); // const
                self.enum_decl(start, exported)
            }
            Tok::Const => self.const_item(start, exported),
            Tok::Let => self.const_item(start, exported),
            // (`var`: as `let`)
            Tok::Var => self.const_item(start, exported),
            Tok::Class | Tok::Abstract => self.class_decl(start, exported, false),
            Tok::Ident if self.text(self.tok()) == "cyclic" && matches!(self.nth(1), Tok::Class | Tok::Abstract) => {
                self.bump();
                self.class_decl(start, exported, true);
            }
            Tok::Enum => self.enum_decl(start, exported),
            Tok::Namespace => {
                let span = self.cur_span();
                self.push(Diagnostic::new("X0006", span, "namespaces are not supported").note("instead", "use one module (file) per namespace"));
                self.skip_decl();
            }
            Tok::Declare => {
                let span = self.cur_span();
                self.err("X0021", span, "`declare` is not supported");
                self.skip_decl();
            }
            Tok::At => {
                let span = self.cur_span();
                self.err("X0015", span, "decorators are not supported");
                self.bump();
                self.ident("a decorator name");
                if self.at(Tok::LParen) {
                    self.skip_balanced();
                }
            }
            Tok::Ident if self.text(self.tok()) == "test" && self.nth(1) == Tok::LParen => self.test_item(start),
            Tok::Ident if self.at_jest() => self.jest_item(start),
            _ => {
                // A script statement (only the entry module may have them; the checker says so).
                let s = self.stmt();
                let span = self.ast.stmts[s as usize].span;
                if !matches!(self.ast.stmts[s as usize].kind, StmtKind::Error | StmtKind::Empty) {
                    if exported {
                        self.err("P0201", span, "only declarations can be exported");
                    }
                    self.ast.script_span.get_or_insert(span);
                    self.script_stmts.push((self.ast.items.len(), s));
                }
            }
        }
    }

    /// `[abstract] class Name<T> extends Base<U> implements I, J { members }`.
    fn class_decl(&mut self, start: Span, exported: bool, cyclic: bool) {
        let is_abstract = self.eat(Tok::Abstract);
        if !self.expect(Tok::Class, "(a class declaration)") {
            self.skip_decl();
            return;
        }
        let Some((name, name_span)) = self.ident("a class name") else {
            self.skip_decl();
            return;
        };
        let tparams = self.tparams();
        let extends = if self.eat(Tok::Extends) { Some(self.postfix_ty()) } else { None };
        let mut implements = Vec::new();
        if self.eat(Tok::Implements) {
            loop {
                implements.push(self.postfix_ty());
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
        }
        if !self.at(Tok::LBrace) {
            self.expect(Tok::LBrace, "to start the class body");
            self.skip_decl();
            return;
        }
        self.bump();
        let mut members = Vec::new();
        while !self.at(Tok::RBrace) && !self.at(Tok::Eof) {
            let before = self.pos;
            if self.eat(Tok::Semi) {
                continue;
            }
            if let Some(m) = self.class_member() {
                members.push(m);
            }
            if self.pos == before {
                self.bump();
            }
        }
        // (fields made from parameter properties come first, as the parameters' own would)
        let extra = std::mem::take(&mut self.extra_members);
        members.splice(0..0, extra);
        self.expect(Tok::RBrace, "to close the class body");
        let span = start.to(self.prev_span());
        let decl = ClassDecl { name, name_span, tparams, is_abstract, cyclic, extends, implements, members };
        self.ast.items.push(Item { kind: ItemKind::Class(decl), span, exported });
    }

    /// Is the current word a modifier (followed by a member name) rather than the member's own name?
    fn at_modifier(&self) -> bool {
        let next = self.nth(1);
        let followed_by_name = matches!(next, Tok::Ident | Tok::Hash | Tok::Str | Tok::LBracket) || next.is_keyword();
        followed_by_name && !self.toks.get(self.pos + 1).map(|t| t.nl_before).unwrap_or(false)
    }

    fn class_member(&mut self) -> Option<ClassMember> {
        let start = self.cur_span();
        let mut vis = Visibility::Public;
        let (mut is_static, mut readonly, mut weak, mut is_abstract, mut is_async) = (false, false, false, false, false);
        loop {
            if self.at(Tok::Abstract) && self.at_modifier() {
                self.bump();
                is_abstract = true;
                continue;
            }
            if self.at(Tok::Async) && self.at_modifier() {
                self.bump();
                is_async = true;
                continue;
            }
            if !self.at(Tok::Ident) || !self.at_modifier() {
                break;
            }
            match self.text(self.tok()) {
                "public" => vis = Visibility::Public,
                "private" => vis = Visibility::Private,
                "protected" => vis = Visibility::Protected,
                "static" => is_static = true,
                "readonly" => readonly = true,
                "weak" => weak = true,
                "override" => {}
                "async" => is_async = true,
                "declare" => {
                    let span = self.cur_span();
                    self.err("X0021", span, "`declare` is not supported");
                }
                "get" | "set" => break,
                _ => break,
            }
            self.bump();
        }
        // Accessors: `get name(): T { ... }`.
        let mut accessor = None;
        if (self.at_word("get") || self.at_word("set")) && self.at_modifier() {
            accessor = Some(self.text(self.tok()) == "get");
            self.bump();
        }
        // `*items() { yield ... }`: a generator method
        let is_gen = self.eat(Tok::Star);
        // `[Symbol.iterator]() { ... }`: what `for...of` over an instance iterates, the method
        // `__iterator`
        let iterator_name = if self.at(Tok::LBracket)
            && self.toks.get(self.pos + 1).is_some_and(|t| t.kind == Tok::Ident && self.text(*t) == "Symbol")
            && self.toks.get(self.pos + 3).is_some_and(|t| t.kind == Tok::Ident && self.text(*t) == "iterator")
            && self.toks.get(self.pos + 4).is_some_and(|t| t.kind == Tok::RBracket)
        {
            let span = self.cur_span();
            for _ in 0..5 {
                self.bump();
            }
            let sym = self.interner.intern("__iterator");
            self.ast.iterator_sym = Some(sym);
            Some((sym, span.to(self.prev_span())))
        } else {
            None
        };
        if iterator_name.is_none() && (self.at(Tok::LBracket) || self.at(Tok::Str)) {
            self.push(Diagnostic::new("X0017", start, "computed and string member names are not supported").note("instead", "use an identifier name"));
            self.recover_line();
            return None;
        }
        let (name, name_span) = if let Some(n) = iterator_name {
            n
        } else if self.at(Tok::Hash) {
            let h = self.bump();
            let Some((n, s)) = self.ident("a private member name") else {
                self.recover_line();
                return None;
            };
            let text = format!("#{}", self.interner.get(n));
            vis = Visibility::Private;
            (self.interner.intern(&text), self.span_of(h).to(s))
        } else {
            match self.prop_name() {
                Some(x) => x,
                None => {
                    let msg = format!("expected a class member, found {}", self.found());
                    let span = self.cur_span();
                    self.err("P0001", span, msg);
                    self.recover_line();
                    return None;
                }
            }
        };
        // `set name(v) { ... }`: the method `__set_name` (`x.name = v` calls it: see the checker)
        let name = if accessor == Some(false) {
            let text = format!("__set_{}", self.interner.get(name));
            self.interner.intern(&text)
        } else {
            name
        };
        let text = self.interner.get(name).to_string();
        if self.at(Tok::LParen) || self.at(Tok::Lt) {
            let tparams = self.tparams();
            self.prop_defaults = false;
            let mut params = self.params();
            let defaults = (std::mem::take(&mut self.defaults), std::mem::take(&mut self.prologue));
            let leading = defaults.0.len() + defaults.1.len();
            let defaulted: Vec<Sym> = defaults.0.iter().map(|d| d.0).collect();
            let prop_defaults = std::mem::take(&mut self.prop_defaults);
            let ret = if self.eat(Tok::Colon) { Some(self.ty()) } else { None };
            let throws = self.effects_clause();
            // (an overload's signature, `get(i: number): T;` before `get(i: number) { ... }`:
            // calls are checked against the implementation's)
            if !self.at(Tok::LBrace) && !is_abstract && self.member_named_next(&text) {
                self.eat(Tok::Semi);
                return None;
            }
            let body = if self.at(Tok::LBrace) {
                let b = self.block();
                self.with_defaults(defaults, b)
            } else {
                if !is_abstract {
                    let span = self.cur_span();
                    let msg = format!("expected `{{` to start the method body, found {}", self.found());
                    self.err("P0001", span, msg);
                }
                self.terminator();
                let span = self.prev_span();
                self.mk_stmt(StmtKind::Block(Vec::new()), span)
            };
            // `constructor(public limit = 2)`: as TypeScript defines it, a field `limit` and
            // `this.limit = limit` once the defaults are in (and after `super(...)`)
            if text == "constructor" && prop_defaults {
                let mut assigns = Vec::new();
                let bspan = self.ast.stmts[body as usize].span;
                let at_body = Span::new(bspan.file, bspan.start, bspan.start);
                for p in params.iter_mut() {
                    let Some((pvis, preadonly)) = p.prop.take() else { continue };
                    let fspan = at_body;
                    self.extra_members.push(ClassMember {
                        name: p.name,
                        name_span: p.span,
                        span: p.span,
                        vis: pvis,
                        is_static: false,
                        readonly: preadonly,
                        weak: false,
                        is_abstract: false,
                        kind: MemberKind::Field { ty: p.ty, init: None, optional: p.optional && !defaulted.contains(&p.name) },
                    });
                    let this = self.mk_expr(ExprKind::This, fspan);
                    let target = self.mk_expr(ExprKind::Member { obj: this, name: p.name, name_span: fspan, optional: false }, fspan);
                    let value = self.mk_expr(ExprKind::Ident(p.name), fspan);
                    let assign = self.mk_expr(ExprKind::Assign(AssignOp::Assign, target, value), fspan);
                    assigns.push(self.mk_stmt(StmtKind::Expr(assign), fspan));
                }
                let ss = match &self.ast.stmts[body as usize].kind {
                    StmtKind::Block(ss) => ss.clone(),
                    _ => Vec::new(),
                };
                let after_super = ss.iter().position(|&x| matches!(&self.ast.stmts[x as usize].kind, StmtKind::Expr(e) if matches!(&self.ast.exprs[*e as usize].kind, ExprKind::Call { callee, .. } if matches!(self.ast.exprs[*callee as usize].kind, ExprKind::Super))));
                let at = after_super.map(|i| i + 1).unwrap_or(leading.min(ss.len()));
                if let StmtKind::Block(ss) = &mut self.ast.stmts[body as usize].kind {
                    for (k, st) in assigns.into_iter().enumerate() {
                        ss.insert(at + k, st);
                    }
                }
            }
            let fd = FnDecl { name, name_span, tparams, params, ret, throws, body, is_async, is_gen };
            let kind = if text == "constructor" && accessor.is_none() {
                MemberKind::Constructor(fd)
            } else if accessor == Some(true) {
                MemberKind::Getter(fd)
            } else {
                MemberKind::Method(fd)
            };
            let span = start.to(self.prev_span());
            return Some(ClassMember { name, name_span, span, vis, is_static, readonly, weak, is_abstract, kind });
        }
        // Field.
        let optional = self.eat(Tok::Question);
        // (`x!: T`: assigned by a method the constructor calls, say)
        if self.eat(Tok::Bang) {
            self.ast.definite_fields.push(name_span.start);
        }
        let ty = if self.eat(Tok::Colon) { Some(self.ty()) } else { None };
        let init = if self.eat(Tok::Eq) { Some(self.assign()) } else { None };
        if !(self.eat(Tok::Semi) || self.eat(Tok::Comma) || self.at(Tok::RBrace) || self.tok().nl_before) {
            let msg = format!("expected `;` or a line break after the field, found {}", self.found());
            let span = self.cur_span();
            self.err("P0002", span, msg);
            self.recover_line();
        }
        let span = start.to(self.prev_span());
        Some(ClassMember { name, name_span, span, vis, is_static, readonly, weak, is_abstract, kind: MemberKind::Field { ty, init, optional } })
    }

    fn import(&mut self, start: Span) {
        self.bump(); // import
        if self.at(Tok::Type) {
            self.bump(); // `import type { ... }` is the same as `import { ... }`
        }
        let mut names = Vec::new();
        let mut locals = Vec::new();
        let mut namespace = None;
        let mut default = None;
        if self.at(Tok::Str) {
            let span = self.cur_span();
            self.err("X0031", span, "side-effect imports are not supported; modules have no initialization side effects");
            self.bump();
            self.terminator();
            return;
        } else if self.eat(Tok::Star) {
            if !self.at_word("as") {
                self.expect(Tok::Ident, "(`as`) after `import *`");
            } else {
                self.bump();
            }
            namespace = self.ident("a namespace name");
        } else if self.at(Tok::LBrace) {
            if !self.import_list(&mut names, &mut locals) {
                return;
            }
        } else if self.at(Tok::Ident) {
            default = self.ident("a name");
            self.interner.intern("default"); // (the checker looks the export up by it)
            // `import d, { a } from ...` / `import d, * as ns from ...`
            if self.eat(Tok::Comma) {
                if self.eat(Tok::Star) {
                    if self.at_word("as") {
                        self.bump();
                    }
                    namespace = self.ident("a namespace name");
                } else if self.at(Tok::LBrace) && !self.import_list(&mut names, &mut locals) {
                    return;
                }
            }
        } else {
            let span = self.cur_span();
            let msg = format!("expected `{{`, `*` or a string after `import`, found {}", self.found());
            self.err("P0001", span, msg);
            self.recover_line();
            return;
        }
        if !self.at_word("from") {
            let span = self.cur_span();
            let msg = format!("expected `from` after the import list, found {}", self.found());
            self.err("P0001", span, msg);
            self.recover_line();
            return;
        }
        self.bump();
        if !self.at(Tok::Str) {
            let span = self.cur_span();
            let msg = format!("expected a module path string, found {}", self.found());
            self.err("P0001", span, msg);
            self.recover_line();
            return;
        }
        let t = self.bump();
        let path = self.strings[t.val as usize].clone();
        let path_span = self.span_of(t);
        self.terminator();
        let span = start.to(path_span);
        // (`import { describe, expect, it } from "@jest/globals"`, or Vitest's, or Bun's: the
        // test API, which is built in)
        if matches!(path.as_str(), "@jest/globals" | "vitest" | "bun:test") && namespace.is_none() && default.is_none() {
            return;
        }
        // (Chai's `expect`: its assertions become the built-in matchers, see `chai_assertions`)
        if path == "chai" && namespace.is_none() && default.is_none() && names.iter().all(|(n, _)| self.interner.get(*n) == "expect") {
            self.chai = true;
            return;
        }
        self.ast.items.push(Item { kind: ItemKind::Import(Import { names, locals, reexport: false, namespace, default, path, path_span }), span, exported: false });
    }

    /// `{ a, b as c }` in an import (the exported names, and what they're called here); false if
    /// it had to give up on the line.
    fn import_list(&mut self, names: &mut Vec<(Sym, Span)>, locals: &mut Vec<Sym>) -> bool {
        self.bump();
        while !self.at(Tok::RBrace) && !self.at(Tok::Eof) {
            if self.at(Tok::Type) && !matches!(self.nth(1), Tok::Comma | Tok::RBrace) && !(self.nth(1) == Tok::Ident && self.text(self.toks[self.pos + 1]) == "as") {
                self.bump(); // `{ type T }`
            }
            let Some(name) = self.prop_name() else {
                let span = self.cur_span();
                let msg = format!("expected an imported name, found {}", self.found());
                self.err("P0001", span, msg);
                self.recover_line();
                return false;
            };
            let mut local = name.0;
            if self.at_word("as") {
                self.bump();
                match self.ident("a name") {
                    Some((a, _)) => local = a,
                    None => {
                        self.recover_line();
                        return false;
                    }
                }
            }
            names.push(name);
            locals.push(local);
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RBrace, "to close the import list");
        true
    }

    fn tparams(&mut self) -> Vec<TypeParam> {
        let mut out = Vec::new();
        if !self.eat(Tok::Lt) {
            return out;
        }
        while !self.at(Tok::Gt) && !self.at(Tok::Eof) {
            let Some((name, span)) = self.ident("a type parameter name") else { break };
            let bound = if self.eat(Tok::Extends) { Some(self.ty()) } else { None };
            let default = if self.eat(Tok::Eq) { Some(self.ty()) } else { None };
            out.push(TypeParam { name, span, bound, default });
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::Gt, "to close the type parameter list");
        out
    }

    fn params(&mut self) -> Vec<Param> {
        let mut out = Vec::new();
        self.expect(Tok::LParen, "to start the parameter list");
        while !self.at(Tok::RParen) && !self.at(Tok::Eof) {
            let rest = self.eat(Tok::DotDotDot);
            if self.at(Tok::LBrace) || self.at(Tok::LBracket) {
                // `({ x, y }: P) =>`: a hidden parameter, destructured as the body starts
                let pat = self.pattern();
                let optional = self.eat(Tok::Question);
                let ty = if self.eat(Tok::Colon) { Some(self.ty()) } else { None };
                let tmp = self.fresh_name();
                let pspan = pat.span();
                let mut init = self.mk_expr(ExprKind::Ident(tmp), pspan);
                let mut optional = optional;
                if self.eat(Tok::Eq) {
                    let d = self.assign();
                    init = self.defaulted(init, d, pspan);
                    optional = true;
                }
                let mut stmts = Vec::new();
                self.destructure(&pat, false, None, init, &mut stmts);
                self.prologue.extend(stmts);
                out.push(Param { name: tmp, span: pspan, ty, inout: false, optional, prop: None, rest });
                if !self.eat(Tok::Comma) {
                    break;
                }
                continue;
            }
            // Constructor parameter properties: `private readonly x: T`.
            let mut prop: Option<(Visibility, bool)> = None;
            loop {
                let next_is_name = matches!(self.nth(1), Tok::Ident) || self.nth(1).is_keyword();
                if !next_is_name {
                    break;
                }
                let vis = if self.at_word("public") {
                    Some(Visibility::Public)
                } else if self.at_word("private") {
                    Some(Visibility::Private)
                } else if self.at_word("protected") {
                    Some(Visibility::Protected)
                } else {
                    None
                };
                if let Some(v) = vis {
                    self.bump();
                    prop = Some((v, prop.map(|p| p.1).unwrap_or(false)));
                } else if self.at_word("readonly") {
                    self.bump();
                    prop = Some((prop.map(|p| p.0).unwrap_or(Visibility::Public), true));
                } else {
                    break;
                }
            }
            let inout = if self.at_word("inout") && self.nth(1) == Tok::Ident {
                self.bump();
                true
            } else {
                false
            };
            let Some((name, span)) = self.ident("a parameter name") else {
                self.skip_to_paren_end();
                break;
            };
            let mut optional = self.eat(Tok::Question);
            let mut ty = if self.eat(Tok::Colon) { Some(self.ty()) } else { None };
            if self.at(Tok::Eq) {
                let s = self.cur_span();
                self.bump();
                let e = self.assign();
                let s = s.to(self.expr_span(e));
                if inout || rest {
                    self.err("U0005", s, "a default value for this parameter is not supported yet; make it optional (`x?: T`) and use `x ?? default`");
                } else {
                    if prop.is_some() {
                        self.prop_defaults = true;
                    }
                    // `x: T = d`: an optional parameter, and `let x: T = x ?? d` (shadowing it)
                    // starting the body.
                    if ty.is_none() {
                        ty = self.literal_type(e);
                    }
                    optional = true;
                    self.defaults.push((name, span, ty, e));
                }
            }
            out.push(Param { name, span, ty, inout, optional, prop, rest });
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen, "to close the parameter list");
        out
    }

    /// The type a parameter takes from its default value when it has no annotation (`n = 10`).
    /// An expression that is a number whatever its names stand for (arithmetic other than `+`,
    /// `.length`, `Math.*`, and `+` of those).
    fn numeric_expr(&self, e: ExprId) -> bool {
        match &self.ast.exprs[e as usize].kind {
            ExprKind::Int(_) | ExprKind::Float(_) => true,
            ExprKind::Paren(x) => self.numeric_expr(*x),
            ExprKind::Unary(_, x) => self.numeric_expr(*x),
            ExprKind::Member { name, .. } => self.interner.get(*name) == "length",
            ExprKind::Call { callee, .. } => matches!(&self.ast.exprs[*callee as usize].kind, ExprKind::Member { obj, .. } if matches!(self.ast.exprs[*obj as usize].kind, ExprKind::Ident(n) if self.interner.get(n) == "Math")),
            ExprKind::Binary(BinOp::Add, a, b) => self.numeric_expr(*a) && self.numeric_expr(*b),
            // (a bigint's too, which a default rarely is)
            ExprKind::Binary(op, ..) => matches!(op, BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem | BinOp::Pow | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr | BinOp::UShr),
            _ => false,
        }
    }

    fn literal_type(&mut self, e: ExprId) -> Option<TypeId> {
        // `visiting = new Set<string>()`: the class, with its type arguments
        if let ExprKind::New { callee, type_args, .. } = &self.ast.exprs[e as usize].kind
            && let ExprKind::Ident(name) = self.ast.exprs[*callee as usize].kind
        {
            let (args, span) = (type_args.clone(), self.expr_span(*callee));
            return Some(self.mk_ty(TypeExprKind::Named { ns: None, name, name_span: span, args }, span));
        }
        let name = match &self.ast.exprs[e as usize].kind {
            ExprKind::Int(_) | ExprKind::Float(_) => "number",
            ExprKind::Str(_) | ExprKind::Template(..) => "string",
            ExprKind::Bool(_) => "boolean",
            ExprKind::Unary(_, x) if matches!(self.ast.exprs[*x as usize].kind, ExprKind::Int(_) | ExprKind::Float(_)) => "number",
            // `end = xs.length - 1`, `mid = Math.floor(n / 2)`
            _ if self.numeric_expr(e) => "number",
            ExprKind::Binary(BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::Eq | BinOp::Ne, ..) => "boolean",
            _ => return None,
        };
        let span = self.expr_span(e);
        let sym = self.interner.intern(name);
        Some(self.mk_ty(TypeExprKind::Named { ns: None, name: sym, name_span: span, args: Vec::new() }, span))
    }

    /// Starts `body` with `let x: T = x !== undefined ? x : default` for each parameter with a default (the
    /// `let` declared at the default, so it's a variable of its own), then the parameters'
    /// destructuring.
    fn with_defaults(&mut self, (defaults, prologue): (Vec<(Sym, Span, Option<TypeId>, ExprId)>, Vec<StmtId>), body: StmtId) -> StmtId {
        if defaults.is_empty() && prologue.is_empty() {
            return body;
        }
        let mut stmts = Vec::new();
        for (name, name_span, ty, d) in defaults {
            let a = self.mk_expr(ExprKind::Ident(name), name_span);
            let ds = self.expr_span(d);
            let init = self.defaulted(a, d, name_span.to(ds));
            stmts.push(self.mk_stmt(StmtKind::Let { mutable: true, name, name_span: ds, ty, init: Some(init) }, name_span.to(ds)));
        }
        stmts.extend(prologue);
        let span = self.ast.stmts[body as usize].span;
        if let StmtKind::Block(ss) = &self.ast.stmts[body as usize].kind {
            stmts.extend(ss.iter().copied());
        }
        self.mk_stmt(StmtKind::Block(stmts), span)
    }

    fn skip_to_paren_end(&mut self) {
        let mut depth = 0i32;
        while !self.at(Tok::Eof) {
            match self.kind() {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            self.bump();
        }
    }

    fn function(&mut self) -> Option<FnDecl> {
        self.bump(); // function
        let is_gen = self.eat(Tok::Star);
        let (name, name_span) = self.ident("a function name")?;
        let mut f = self.function_rest(name, name_span)?;
        f.is_gen = is_gen;
        Some(f)
    }

    /// A function after its name: type parameters, parameters, result, body.
    fn function_rest(&mut self, name: Sym, name_span: Span) -> Option<FnDecl> {
        let tparams = self.tparams();
        let params = self.params();
        let defaults = (std::mem::take(&mut self.defaults), std::mem::take(&mut self.prologue));
        let ret = if self.eat(Tok::Colon) { Some(self.ty()) } else { None };
        let throws = self.effects_clause();
        // An overload's signature (`function f(a: number[][], b: number): number[][]` before
        // `function f(...) { ... }`): calls are checked against the implementation's
        if !self.at(Tok::LBrace) && self.overload_follows(name) {
            self.eat(Tok::Semi);
            self.ast.overloads.push((name, tparams, params, ret));
            return None;
        }
        if !self.at(Tok::LBrace) {
            let span = self.cur_span();
            let msg = format!("expected `{{` to start the function body, found {}", self.found());
            self.err("P0001", span, msg);
            self.recover_line();
            return None;
        }
        let body = self.block();
        let body = self.with_defaults(defaults, body);
        Some(FnDecl { name, name_span, tparams, params, ret, throws, body, is_async: false, is_gen: false })
    }

    /// After a method's signature without a body: the next member is one of the same name (after
    /// a `;` and modifiers).
    fn member_named_next(&self, name: &str) -> bool {
        let mut i = self.pos + usize::from(self.at(Tok::Semi));
        if !(self.at(Tok::Semi) || self.tok().nl_before) {
            return false;
        }
        while let Some(t) = self.toks.get(i) {
            let w = self.text(*t);
            if (t.kind == Tok::Async || (t.kind == Tok::Ident && matches!(w, "public" | "private" | "protected" | "static" | "readonly" | "override" | "abstract"))) && self.toks.get(i + 1).is_some_and(|n| n.kind == Tok::Ident) {
                i += 1;
                continue;
            }
            return (t.kind == Tok::Ident || matches!(w, "constructor")) && w == name;
        }
        false
    }

    /// After a signature without a body: another declaration of `name` comes next (perhaps
    /// after a `;`, `export`, `async`).
    fn overload_follows(&self, name: Sym) -> bool {
        let mut i = self.pos + usize::from(self.at(Tok::Semi));
        let at = |i: usize| self.toks.get(i).map(|t| t.kind).unwrap_or(Tok::Eof);
        if !(self.at(Tok::Semi) || self.tok().nl_before) {
            return false;
        }
        while matches!(at(i), Tok::Export | Tok::Async) {
            i += 1;
        }
        // (`function* f`: a generator's)
        let star = usize::from(at(i + 1) == Tok::Star);
        at(i) == Tok::Function && self.toks.get(i + 1 + star).is_some_and(|t| t.kind == Tok::Ident && self.text(*t) == self.interner.get(name))
    }

    /// `throws E` (returned) / `uses fs | net` (not supported yet).
    fn effects_clause(&mut self) -> Option<TypeId> {
        let mut throws = None;
        loop {
            if self.at_word("throws") {
                self.bump();
                throws = Some(self.ty());
            } else if self.at_word("uses") {
                let span = self.cur_span();
                self.err("U0004", span, "`uses` effect clauses are not supported yet");
                self.bump();
                self.ident("an effect name");
                while self.eat(Tok::Pipe) {
                    self.ident("an effect name");
                }
            } else {
                return throws;
            }
        }
    }

    /// A type declared in a body keeps its name when no other type in the module has it.
    fn unhide_local_types(&mut self) {
        let taken = |ast: &Ast, n: Sym| {
            ast.items.iter().any(|it| match &it.kind {
                ItemKind::TypeAlias { name, .. } | ItemKind::Interface { name, .. } => *name == n,
                ItemKind::Class(c) => c.name == n,
                _ => false,
            })
        };
        for &(name, hidden) in &self.local_type_items {
            if self.local_type_items.iter().filter(|t| t.0 == name).count() > 1 || taken(&self.ast, name) {
                continue;
            }
            for it in &mut self.ast.items {
                if let ItemKind::TypeAlias { name: n, .. } | ItemKind::Interface { name: n, .. } = &mut it.kind
                    && *n == hidden
                {
                    *n = name;
                }
            }
            for t in &mut self.ast.types {
                if let TypeExprKind::Named { ns: None, name: n, .. } = &mut t.kind
                    && *n == hidden
                {
                    *n = name;
                }
            }
        }
    }

    fn type_alias(&mut self, start: Span, exported: bool) {
        self.bump(); // type
        let Some((name, name_span)) = self.ident("a type name") else {
            self.recover_line();
            return;
        };
        let tparams = self.tparams();
        if !self.expect(Tok::Eq, "after the type name") {
            self.recover_line();
            return;
        }
        let ty = self.ty();
        self.terminator();
        let span = start.to(self.prev_span());
        // `type P = { x: number }` is an interface with those members (as TypeScript's object
        // types are structural, a class instance with them is one), a record until one is given
        if let TypeExprKind::Record(fields) = &self.ast.ty(ty).kind {
            let members = fields.clone();
            self.ast.items.push(Item { kind: ItemKind::Interface { name, name_span, tparams, members, body: ty, extends: Vec::new() }, span, exported });
            return;
        }
        self.ast.items.push(Item { kind: ItemKind::TypeAlias { name, name_span, tparams, ty }, span, exported });
    }

    fn interface(&mut self, start: Span, exported: bool) {
        self.bump(); // interface
        let Some((name, name_span)) = self.ident("an interface name") else {
            self.skip_decl();
            return;
        };
        let tparams = self.tparams();
        // `extends A, B<T>`: their members too
        let mut extends = Vec::new();
        if self.eat(Tok::Extends) {
            loop {
                extends.push(self.ty());
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
        }
        if !self.at(Tok::LBrace) {
            self.expect(Tok::LBrace, "to start the interface body");
            self.skip_decl();
            return;
        }
        let body_start = self.cur_span();
        let saved_call = self.call_sig.take();
        let members = self.record_fields();
        if let Some((_, cspan)) = std::mem::replace(&mut self.call_sig, saved_call) {
            self.err("U0021", cspan, "an interface with a call signature is not supported yet; use a function type (`type F = (x: T) => R`)");
        }
        let body = self.mk_ty(TypeExprKind::Record(members.clone()), body_start.to(self.prev_span()));
        // (as a record: those it extends and its own members, `A & B<T> & { ... }`)
        let body = if extends.is_empty() {
            body
        } else {
            let mut parts = extends.clone();
            parts.push(body);
            self.mk_ty(TypeExprKind::Intersection(parts), body_start.to(self.prev_span()))
        };
        let span = start.to(self.prev_span());
        self.ast.items.push(Item { kind: ItemKind::Interface { name, name_span, tparams, members, body, extends }, span, exported });
    }

    fn const_item(&mut self, start: Span, exported: bool) {
        let mutable = self.at(Tok::Let);
        self.bump(); // const / let
        if self.at(Tok::LBrace) || self.at(Tok::LBracket) {
            // `const [lo, hi] = f()`: a module variable each (the value in a hidden one)
            let pat = self.pattern();
            let ty = if self.eat(Tok::Colon) { Some(self.ty()) } else { None };
            if !self.expect(Tok::Eq, "(module constants need a value)") {
                self.recover_line();
                return;
            }
            let init = self.assign();
            self.terminator();
            let mut binds = Vec::new();
            self.bindings(&pat, ty, init, &mut binds);
            for (name, name_span, ty, init, hidden) in binds {
                let span = start.to(self.expr_span(init));
                self.ast.items.push(Item { kind: ItemKind::Const { name, name_span, ty, init, mutable: mutable && !hidden }, span, exported: exported && !hidden });
            }
            return;
        }
        // (`let lo = 0, hi = n`: a module variable each)
        let mut start = start;
        loop {
            let Some((name, name_span)) = self.ident("a constant name") else {
                self.recover_line();
                return;
            };
            let ty = if self.eat(Tok::Colon) { Some(self.ty()) } else { None };
            // `let m: T | null`: `undefined` to start with (`null` if that's the type's absent value)
            let init = if mutable && ty.is_some() && !self.at(Tok::Eq) {
                let te = ty.unwrap();
                let null_only = match &self.ast.types[te as usize].kind {
                    TypeExprKind::Union(ms) => {
                        let has = |k: fn(&TypeExprKind) -> bool| ms.iter().any(|&m| k(&self.ast.types[m as usize].kind));
                        has(|k| matches!(k, TypeExprKind::Null)) && !has(|k| matches!(k, TypeExprKind::Undefined))
                    }
                    _ => false,
                };
                self.mk_expr(if null_only { ExprKind::Null } else { ExprKind::Undefined }, name_span)
            } else {
                if !self.expect(Tok::Eq, "(module constants need a value)") {
                    self.recover_line();
                    return;
                }
                self.assign()
            };
            let more = self.eat(Tok::Comma);
            if !more {
                self.terminator();
            }
            let span = start.to(self.prev_span());
            // `const f = (n: number): number => ...`, `const id = <T>(x: T): T => x`: the function
            // `f` (it may call itself; calls to it are direct), as JavaScript's differ only in
            // when the name can be used
            // (with its result type written: an inferred one stays the closure's, which speculation
            // widens with what it's given to; an async one returning a promise has its value)
            // (or calling itself, which a closure's constant can't)
            let generic = self.generic_arrows.iter().any(|g| g.0 == init);
            let iv = self.ast.exprs[init as usize].span;
            if ty.is_none()
                && !mutable
                && let ExprKind::Arrow(f) = &self.ast.exprs[init as usize].kind
                && (generic
                    || (!f.is_async
                        && (f.ret.is_some()
                            || self.ast.exprs.iter().any(|e| matches!(e.kind, ExprKind::Ident(n) if n == name) && e.span.file == iv.file && e.span.start >= iv.start && e.span.end <= iv.end))))
            {
                let tparams = match self.generic_arrows.iter().position(|g| g.0 == init) {
                    Some(gi) => self.generic_arrows.remove(gi).1,
                    None => Vec::new(),
                };
                let ExprKind::Arrow(f) = std::mem::replace(&mut self.ast.exprs[init as usize].kind, ExprKind::Error) else { unreachable!() };
                let f = *f;
                let body = match f.body {
                    ArrowBody::Block(b) => b,
                    ArrowBody::Expr(x) => {
                        let sp = self.expr_span(x);
                        let r = self.mk_stmt(StmtKind::Return(Some(x)), sp);
                        self.mk_stmt(StmtKind::Block(vec![r]), sp)
                    }
                };
                let decl = FnDecl { name, name_span, tparams, params: f.params, ret: f.ret, throws: f.throws, body, is_async: f.is_async, is_gen: false };
                self.ast.items.push(Item { kind: ItemKind::Function(decl), span, exported });
            } else {
                self.ast.items.push(Item { kind: ItemKind::Const { name, name_span, ty, init, mutable }, span, exported });
            }
            if !more {
                return;
            }
            start = self.cur_span();
        }
    }

    /// `enum Color { Red, Green = 5 }` / `enum Dir { Up = "UP" }`: the type `Color` (a number,
    /// or the union of the string values) and the constant `Color` holding the members.
    fn enum_decl(&mut self, start: Span, exported: bool) {
        self.bump(); // enum
        let Some((name, name_span)) = self.ident("an enum name") else {
            self.skip_decl();
            return;
        };
        if !self.expect(Tok::LBrace, "to start the enum's members") {
            self.skip_decl();
            return;
        }
        let mut members: Vec<(Sym, Span, ExprId)> = Vec::new();
        let mut next: i64 = 0;
        let mut strings = Vec::new();
        while !self.at(Tok::RBrace) && !self.at(Tok::Eof) {
            let Some((m, mspan)) = self.prop_name() else { break };
            let value = if self.eat(Tok::Eq) {
                let e = self.assign();
                match &self.ast.exprs[e as usize].kind {
                    ExprKind::Int(v) => next = *v as i64 + 1,
                    ExprKind::Unary(UnOp::Neg, x) if matches!(self.ast.exprs[*x as usize].kind, ExprKind::Int(_)) => {
                        let ExprKind::Int(v) = self.ast.exprs[*x as usize].kind else { unreachable!() };
                        next = -(v as i64) + 1;
                    }
                    ExprKind::Str(s) => strings.push(*s),
                    // `1 << 2`, `Read | Write`: worked out here, as TypeScript does
                    _ => match self.enum_const(e, &members, name) {
                        Some(v) => {
                            next = v + 1;
                            let es = self.expr_span(e);
                            let lit = self.mk_expr(ExprKind::Int(v.unsigned_abs()), es);
                            let lit = if v < 0 { self.mk_expr(ExprKind::Unary(UnOp::Neg, lit), es) } else { lit };
                            members.push((m, mspan, lit));
                            if !self.eat(Tok::Comma) {
                                break;
                            }
                            continue;
                        }
                        None => {
                            let es = self.expr_span(e);
                            self.err("X0005", es, "an enum member's value must be a number, a string or a constant expression of the members before it");
                        }
                    },
                }
                e
            } else {
                let v = next;
                next += 1;
                let e = self.mk_expr(ExprKind::Int(v.unsigned_abs()), mspan);
                if v < 0 { self.mk_expr(ExprKind::Unary(UnOp::Neg, e), mspan) } else { e }
            };
            members.push((m, mspan, value));
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RBrace, "to close the enum");
        let span = start.to(self.prev_span());
        // the type: a number, or (all strings) the union of the values
        let ty = if !strings.is_empty() && strings.len() == members.len() {
            let ms: Vec<TypeId> = strings.iter().map(|&s| self.mk_ty(TypeExprKind::StrLit(s), name_span)).collect();
            if ms.len() == 1 { ms[0] } else { self.mk_ty(TypeExprKind::Union(ms), name_span) }
        } else {
            let number = self.interner.intern("number");
            self.mk_ty(TypeExprKind::Named { ns: None, name: number, name_span, args: Vec::new() }, name_span)
        };
        self.ast.items.push(Item { kind: ItemKind::TypeAlias { name, name_span, tparams: Vec::new(), ty }, span, exported });
        // the members: `{ Red: Color, ... }`
        let fields: Vec<FieldTy> = members
            .iter()
            .map(|&(m, mspan, _)| {
                let t = self.mk_ty(TypeExprKind::Named { ns: None, name, name_span: mspan, args: Vec::new() }, mspan);
                FieldTy { name: m, name_span: mspan, optional: false, ty: t }
            })
            .collect();
        let rec = self.mk_ty(TypeExprKind::Record(fields), span);
        let obj = members.iter().map(|&(m, mspan, value)| ObjField { name: m, name_span: mspan, value, quoted: false }).collect();
        let init = self.mk_expr(ExprKind::Object(obj), span);
        self.ast.items.push(Item { kind: ItemKind::Const { name, name_span, ty: Some(rec), init, mutable: false }, span, exported });
    }

    /// An enum member's constant value: integers, `+ - * / % << >> >>> & | ^ ~`, and the
    /// enum's earlier members (`Read`, `Flags.Read`).
    fn enum_const(&self, e: ExprId, members: &[(Sym, Span, ExprId)], enum_name: Sym) -> Option<i64> {
        let ast = &self.ast;
        let member = |name: Sym| members.iter().find(|m| m.0 == name).and_then(|m| self.enum_const(m.2, &members[..members.iter().position(|x| x.0 == name).unwrap()], enum_name));
        Some(match &ast.exprs[e as usize].kind {
            ExprKind::Int(v) => *v as i64,
            ExprKind::Paren(x) => self.enum_const(*x, members, enum_name)?,
            ExprKind::Unary(UnOp::Neg, x) => -self.enum_const(*x, members, enum_name)?,
            ExprKind::Unary(UnOp::BitNot, x) => !(self.enum_const(*x, members, enum_name)? as i32) as i64,
            ExprKind::Ident(n) => member(*n)?,
            ExprKind::Member { obj, name, .. } if matches!(ast.exprs[*obj as usize].kind, ExprKind::Ident(o) if o == enum_name) => member(*name)?,
            ExprKind::Binary(op, a, b) => {
                let (a, b) = (self.enum_const(*a, members, enum_name)?, self.enum_const(*b, members, enum_name)?);
                let (x, y) = (a as i32, b as i32);
                match op {
                    BinOp::Add => a.checked_add(b)?,
                    BinOp::Sub => a.checked_sub(b)?,
                    BinOp::Mul => a.checked_mul(b)?,
                    BinOp::Div if b != 0 && a % b == 0 => a / b,
                    BinOp::Rem if b != 0 => a % b,
                    BinOp::Shl => x.wrapping_shl(y as u32 & 31) as i64,
                    BinOp::Shr => x.wrapping_shr(y as u32 & 31) as i64,
                    BinOp::UShr => ((x as u32) >> (y as u32 & 31)) as i64,
                    BinOp::BitAnd => (x & y) as i64,
                    BinOp::BitOr => (x | y) as i64,
                    BinOp::BitXor => (x ^ y) as i64,
                    _ => return None,
                }
            }
            _ => return None,
        })
    }

    fn test_item(&mut self, start: Span) {
        self.bump(); // test
        self.bump(); // (
        if !self.at(Tok::Str) {
            let span = self.cur_span();
            self.err("P0001", span, "expected a test name string: `test(\"name\", () => { ... })`");
            self.recover_line();
            return;
        }
        let t = self.bump();
        let name = self.strings[t.val as usize].clone();
        let name_span = self.span_of(t);
        self.expect(Tok::Comma, "after the test name");
        let body = self.assign();
        self.eat(Tok::Comma);
        self.expect(Tok::RParen, "to close `test(...)`");
        self.terminator();
        let span = start.to(self.prev_span());
        self.ast.items.push(Item { kind: ItemKind::Test { name, name_span, body, suite: false }, span, exported: false });
    }

    /// At the top level: `describe(...)`, `it(...)`, `test.skip(...)` and the like (Jest's and
    /// Vitest's API)?
    fn at_jest(&self) -> bool {
        let w = self.text(self.tok());
        match self.nth(1) {
            Tok::LParen => matches!(w, "describe" | "it"),
            Tok::Dot => {
                matches!(w, "describe" | "it" | "test")
                    && self.toks.get(self.pos + 2).is_some_and(|t| t.kind == Tok::Ident && matches!(self.text(*t), "skip" | "only" | "todo" | "each"))
            }
            _ => false,
        }
    }

    /// `describe("A", () => { ... })`, `it("does", () => ...)`: a suite runs its body as a test
    /// item, in which each `it`/`test` call is a test of its own (`A > does`, see `suite_body`).
    fn jest_item(&mut self, start: Span) {
        let e = self.expr();
        self.terminator();
        let span = start.to(self.prev_span());
        // `test.each(table)(name, f)` on its own: a suite of its rows
        if let Some((table, n, f)) = self.each_call(e) {
            let name_span = self.expr_span(n);
            let name = match &self.ast.exprs[n as usize].kind {
                ExprKind::Str(s) => self.interner.get(*s).to_string(),
                _ => self.src_text(name_span),
            };
            let Some(st) = self.each_stmt(table, n, f, "", &[], &[], span) else { return };
            let b = self.mk_stmt(StmtKind::Block(vec![st]), span);
            let body = self.mk_expr(ExprKind::Arrow(Box::new(ArrowFn { params: Vec::new(), ret: None, throws: None, body: ArrowBody::Block(b), is_async: false })), span);
            self.ast.items.push(Item { kind: ItemKind::Test { name, name_span, body, suite: true }, span, exported: false });
            return;
        }
        let Some((kind, modifier, args)) = self.jest_call(e) else { return };
        if matches!(modifier.as_deref(), Some("skip" | "todo")) {
            return;
        }
        let (Some(&n), Some(&f)) = (args.first(), args.get(1)) else {
            self.err("P0001", span, format!("`{kind}` takes a name and a function"));
            return;
        };
        let name_span = self.expr_span(n);
        let name = match &self.ast.exprs[n as usize].kind {
            ExprKind::Str(s) => self.interner.get(*s).to_string(),
            _ => self.src_text(name_span),
        };
        let suite = kind == "describe";
        if suite {
            self.suite_body(f, &name, &[], &[], None);
        }
        self.ast.items.push(Item { kind: ItemKind::Test { name, name_span, body: f, suite }, span, exported: false });
    }

    /// `name(args)` or `name.modifier(args)` for Jest's functions: (name, modifier, args).
    fn jest_call(&self, e: ExprId) -> Option<(String, Option<String>, Vec<ExprId>)> {
        let ExprKind::Call { callee, args, .. } = &self.ast.exprs[e as usize].kind else { return None };
        let args = args.iter().map(|a| a.expr).collect();
        match &self.ast.exprs[*callee as usize].kind {
            ExprKind::Ident(s) => {
                let n = self.interner.get(*s);
                matches!(n, "describe" | "it" | "test" | "beforeEach" | "afterEach" | "beforeAll" | "afterAll").then(|| (n.to_string(), None, args))
            }
            ExprKind::Member { obj, name, .. } => {
                let ExprKind::Ident(s) = self.ast.exprs[*obj as usize].kind else { return None };
                let (n, m) = (self.interner.get(s), self.interner.get(*name));
                (matches!(n, "describe" | "it" | "test") && matches!(m, "skip" | "only" | "todo")).then(|| (n.to_string(), Some(m.to_string()), args))
            }
            _ => None,
        }
    }

    /// A suite's body, in place: `beforeEach(f)` becomes a hidden constant called first by each
    /// later test of this suite and those in it (`afterEach(f)` last), `beforeAll(f)` a call
    /// there, `afterAll(f)` a call at the end, a nested `describe` its own body there, and
    /// `it("does", g)` a test `__test("Suite > does", g)`.
    fn suite_body(&mut self, arrow: ExprId, prefix: &str, before: &[Sym], after: &[Sym], own: Option<ExprId>) {
        let body = match &self.ast.exprs[arrow as usize].kind {
            ExprKind::Arrow(f) => match f.body {
                ArrowBody::Block(b) => b,
                // (`describe("A", () => check(f))`: a body of that statement)
                ArrowBody::Expr(x) => {
                    let span = self.expr_span(x);
                    let st = self.mk_stmt(StmtKind::Expr(x), span);
                    let b = self.mk_stmt(StmtKind::Block(vec![st]), span);
                    if let ExprKind::Arrow(f) = &mut self.ast.exprs[arrow as usize].kind {
                        f.body = ArrowBody::Block(b);
                    }
                    b
                }
            },
            _ => return,
        };
        self.suite_block(body, prefix, before, after);
        // (its name, for tests declared in functions it calls: `__suiteEnter(name)` first,
        // `__suiteLeave()` last)
        let span = self.ast.stmts[body as usize].span;
        let full = match own {
            Some(n) => {
                let f = self.interner.intern("__suiteName");
                let callee = self.mk_expr(ExprKind::Ident(f), span);
                self.mk_expr(ExprKind::Call { callee, type_args: Vec::new(), args: vec![Arg { expr: n, by_ref: None }], optional: false }, span)
            }
            None => {
                let t = self.interner.intern(prefix);
                self.mk_expr(ExprKind::Str(t), span)
            }
        };
        let enter = self.helper_call("__suiteEnter", vec![full], span);
        let leave = self.helper_call("__suiteLeave", Vec::new(), span);
        if let StmtKind::Block(ss) = &mut self.ast.stmts[body as usize].kind {
            ss.insert(0, enter);
            ss.push(leave);
        }
    }

    /// `name(args);`
    fn helper_call(&mut self, name: &str, args: Vec<ExprId>, span: Span) -> StmtId {
        let f = self.interner.intern(name);
        let callee = self.mk_expr(ExprKind::Ident(f), span);
        let args = args.into_iter().map(|expr| Arg { expr, by_ref: None }).collect();
        let c = self.mk_expr(ExprKind::Call { callee, type_args: Vec::new(), args, optional: false }, span);
        self.mk_stmt(StmtKind::Expr(c), span)
    }

    /// Tests declared in functions (`const check = (f) => it.each(rows)(name, ...)`, a helper
    /// with `describe(...)` and `it(...)` in it): named inside the suites running when it's called.
    fn runtime_tests(&mut self) {
        // (a module's own `it` or `test` isn't Jest's)
        let own = |p: &Self, n: &str| {
            let Some(s) = p.interner.lookup(n) else { return false };
            p.ast.stmts.iter().any(|st| matches!(st.kind, StmtKind::Let { name, .. } if name == s))
                || p.ast.items.iter().any(|it| matches!(&it.kind, ItemKind::Const { name, .. } if *name == s) || matches!(&it.kind, ItemKind::Function(f) if f.name == s))
        };
        let shadowed: Vec<&str> = ["it", "test", "describe"].into_iter().filter(|n| own(self, n)).collect();
        let is_test = |p: &Self, e: ExprId| {
            if let Some((table, _, _)) = p.each_call(e) {
                let _ = table;
                let ExprKind::Call { callee, .. } = &p.ast.exprs[e as usize].kind else { return false };
                let ExprKind::Call { callee: inner, .. } = &p.ast.exprs[*callee as usize].kind else { return false };
                let ExprKind::Member { obj, .. } = &p.ast.exprs[*inner as usize].kind else { return false };
                let ExprKind::Ident(s) = p.ast.exprs[*obj as usize].kind else { return false };
                return !shadowed.contains(&p.interner.get(s));
            }
            matches!(p.jest_call(e), Some((k, _, args)) if matches!(k.as_str(), "it" | "test" | "describe") && args.len() >= 2 && !shadowed.contains(&k.as_str()))
        };
        // (an arrow that is one, `(f) => it.each(rows)(...)`: a body of that statement)
        for i in 0..self.ast.exprs.len() {
            let ExprKind::Arrow(f) = &self.ast.exprs[i].kind else { continue };
            let ArrowBody::Expr(x) = f.body else { continue };
            if !is_test(self, x) {
                continue;
            }
            let span = self.expr_span(x);
            let st = self.mk_stmt(StmtKind::Expr(x), span);
            let b = self.mk_stmt(StmtKind::Block(vec![st]), span);
            if let ExprKind::Arrow(f) = &mut self.ast.exprs[i].kind {
                f.body = ArrowBody::Block(b);
            }
        }
        let n = self.ast.stmts.len();
        for b in 0..n {
            let StmtKind::Block(ss) = &self.ast.stmts[b].kind else { continue };
            if !ss.iter().any(|&s| matches!(self.ast.stmts[s as usize].kind, StmtKind::Expr(e) if is_test(self, e))) {
                continue;
            }
            self.runtime_tests = true;
            self.suite_block(b as StmtId, "", &[], &[]);
            self.runtime_tests = false;
        }
        self.top_level_test_calls();
    }

    /// Chai's assertions, `expect(x).to.equal(y)`, `expect(f).to.throw()`, `expect(v).to.be.true`:
    /// the built-in matchers (`expect(x).toBe(y)`, ...).
    fn chai_assertions(&mut self) {
        for i in 0..self.ast.exprs.len() {
            if let Some(kind) = self.chai_chain(i as ExprId) {
                self.ast.exprs[i].kind = kind;
            }
        }
        // (Mocha's `this.timeout(ms)`, `this.slow(ms)`, `this.retries(n)` in a test: tests run to
        // the end here)
        for i in 0..self.ast.stmts.len() {
            let StmtKind::Expr(e) = self.ast.stmts[i].kind else { continue };
            let ExprKind::Call { callee, .. } = &self.ast.exprs[e as usize].kind else { continue };
            if let ExprKind::Member { obj, name, .. } = &self.ast.exprs[*callee as usize].kind
                && matches!(self.ast.exprs[*obj as usize].kind, ExprKind::This)
                && matches!(self.interner.get(*name), "timeout" | "slow" | "retries")
            {
                self.ast.stmts[i].kind = StmtKind::Empty;
            }
        }
    }

    /// `e` as a Chai assertion, rewritten: the matcher call.
    fn chai_chain(&mut self, e: ExprId) -> Option<ExprKind> {
        // (the assertion's last word and its arguments, then the words before it)
        let (last, args, mut cur) = match &self.ast.exprs[e as usize].kind {
            ExprKind::Call { callee, args, .. } => match &self.ast.exprs[*callee as usize].kind {
                ExprKind::Member { obj, name, .. } => (*name, Some(args.iter().map(|a| a.expr).collect::<Vec<_>>()), *obj),
                _ => return None,
            },
            ExprKind::Member { obj, name, .. } => (*name, None, *obj),
            _ => return None,
        };
        let (mut not, mut deep) = (false, false);
        let subject = loop {
            match &self.ast.exprs[cur as usize].kind {
                ExprKind::Member { obj, name, .. } => {
                    match self.interner.get(*name) {
                        "not" => not = !not,
                        "deep" => deep = true,
                        "to" | "be" | "been" | "is" | "that" | "which" | "and" | "has" | "have" | "with" | "at" | "of" | "same" | "but" | "does" | "still" | "also" | "own" | "nested" => {}
                        _ => return None,
                    }
                    cur = *obj;
                }
                ExprKind::Call { callee, args, .. } if args.len() == 1 && matches!(self.ast.exprs[*callee as usize].kind, ExprKind::Ident(s) if self.interner.get(s) == "expect") => break args[0].expr,
                _ => return None,
            }
        };
        let span = self.ast.exprs[e as usize].span;
        let word = self.interner.get(last).to_string();
        let lit = |p: &mut Self, b: bool| p.mk_expr(ExprKind::Bool(b), span);
        let (matcher, margs, subject) = match (word.as_str(), args) {
            ("equal" | "equals" | "eq" | "strictEqual", Some(a)) if a.len() == 1 => (if deep { "toEqual" } else { "toBe" }, a, subject),
            ("eql" | "eqls", Some(a)) if a.len() == 1 => ("toEqual", a, subject),
            ("include" | "includes" | "contain" | "contains", Some(a)) if a.len() == 1 => ("toContain", a, subject),
            ("lengthOf" | "length", Some(a)) if a.len() == 1 => ("toHaveLength", a, subject),
            ("above" | "gt" | "greaterThan", Some(a)) if a.len() == 1 => ("toBeGreaterThan", a, subject),
            ("least" | "gte" | "greaterThanOrEqual", Some(a)) if a.len() == 1 => ("toBeGreaterThanOrEqual", a, subject),
            ("below" | "lt" | "lessThan", Some(a)) if a.len() == 1 => ("toBeLessThan", a, subject),
            ("most" | "lte" | "lessThanOrEqual", Some(a)) if a.len() == 1 => ("toBeLessThanOrEqual", a, subject),
            ("throw" | "throws" | "Throw", Some(_)) => ("toThrow", Vec::new(), subject),
            // (`.closeTo(v, delta)`: `Math.abs(x - v) <= delta`)
            ("closeTo" | "approximately", Some(a)) if a.len() == 2 => {
                let d = self.mk_expr(ExprKind::Binary(BinOp::Sub, subject, a[0]), span);
                let math = self.interner.intern("Math");
                let abs = self.interner.intern("abs");
                let m = self.mk_expr(ExprKind::Ident(math), span);
                let f = self.mk_expr(ExprKind::Member { obj: m, name: abs, name_span: span, optional: false }, span);
                let c = self.mk_expr(ExprKind::Call { callee: f, type_args: Vec::new(), args: vec![Arg { expr: d, by_ref: None }], optional: false }, span);
                let le = self.mk_expr(ExprKind::Binary(BinOp::Le, c, a[1]), span);
                let t = lit(self, true);
                ("toBe", vec![t], le)
            }
            // (`.instanceOf(C)`: `x instanceof C`)
            ("instanceOf" | "instanceof", Some(a)) if a.len() == 1 => {
                let x = self.mk_expr(ExprKind::Binary(BinOp::Instanceof, subject, a[0]), span);
                let t = lit(self, true);
                ("toBe", vec![t], x)
            }
            // (`.a("string")`: `typeof x`, an array's `Array.isArray(x)`)
            ("a" | "an", Some(a)) if a.len() == 1 => {
                let what = match &self.ast.exprs[a[0] as usize].kind {
                    ExprKind::Str(s) => self.interner.get(*s).to_string(),
                    _ => return None,
                };
                if what == "array" {
                    let arr = self.interner.intern("Array");
                    let is = self.interner.intern("isArray");
                    let m = self.mk_expr(ExprKind::Ident(arr), span);
                    let f = self.mk_expr(ExprKind::Member { obj: m, name: is, name_span: span, optional: false }, span);
                    let c = self.mk_expr(ExprKind::Call { callee: f, type_args: Vec::new(), args: vec![Arg { expr: subject, by_ref: None }], optional: false }, span);
                    let t = lit(self, true);
                    ("toBe", vec![t], c)
                } else {
                    let t = self.mk_expr(ExprKind::Typeof(subject), span);
                    ("toBe", vec![a[0]], t)
                }
            }
            ("true", None) => {
                let t = lit(self, true);
                ("toBe", vec![t], subject)
            }
            ("false", None) => {
                let f = lit(self, false);
                ("toBe", vec![f], subject)
            }
            ("null", None) => ("toBeNull", Vec::new(), subject),
            ("undefined", None) => ("toBeUndefined", Vec::new(), subject),
            ("ok", None) => ("toBeTruthy", Vec::new(), subject),
            ("NaN", None) => ("toBeNaN", Vec::new(), subject),
            ("exist", None) => ("toBeDefined", Vec::new(), subject),
            ("empty", None) => {
                let z = self.mk_expr(ExprKind::Int(0), span);
                ("toHaveLength", vec![z], subject)
            }
            _ => return None,
        };
        let expect = self.interner.intern("expect");
        let ex = self.mk_expr(ExprKind::Ident(expect), span);
        let mut obj = self.mk_expr(ExprKind::Call { callee: ex, type_args: Vec::new(), args: vec![Arg { expr: subject, by_ref: None }], optional: false }, span);
        if not {
            let n = self.interner.intern("not");
            obj = self.mk_expr(ExprKind::Member { obj, name: n, name_span: span, optional: false }, span);
        }
        let m = self.interner.intern(matcher);
        let callee = self.mk_expr(ExprKind::Member { obj, name: m, name_span: span, optional: false }, span);
        Some(ExprKind::Call { callee, type_args: Vec::new(), args: margs.into_iter().map(|expr| Arg { expr, by_ref: None }).collect(), optional: false })
    }

    /// A script's statement calling a function that declares tests (`checkStack("top")`, at the
    /// top level): a test item of its own, so `tov test` runs it (in its place among the others).
    fn top_level_test_calls(&mut self) {
        let Some(si) = self.ast.script else { return };
        let Some(here) = self.interner.lookup("__testHere") else { return };
        // (functions declaring tests, and those calling them: by name, with where their code is)
        let mut fns: Vec<(Sym, Span)> = Vec::new();
        for it in &self.ast.items {
            match &it.kind {
                ItemKind::Function(f) => fns.push((f.name, self.ast.stmts[f.body as usize].span)),
                ItemKind::Const { name, init, .. } if matches!(self.ast.exprs[*init as usize].kind, ExprKind::Arrow(_)) => fns.push((*name, self.ast.exprs[*init as usize].span)),
                _ => {}
            }
        }
        for st in &self.ast.stmts {
            if let StmtKind::Let { name, init: Some(i), .. } = st.kind
                && matches!(self.ast.exprs[i as usize].kind, ExprKind::Arrow(_))
            {
                fns.push((name, self.ast.exprs[i as usize].span));
            }
        }
        let mentions = |ast: &Ast, sp: Span, n: Sym| ast.exprs.iter().any(|e| matches!(e.kind, ExprKind::Ident(x) if x == n) && e.span.file == sp.file && e.span.start >= sp.start && e.span.end <= sp.end);
        let mut declaring: Vec<Sym> = Vec::new();
        loop {
            let before = declaring.len();
            for &(n, sp) in &fns {
                if !declaring.contains(&n) && (mentions(&self.ast, sp, here) || declaring.iter().any(|&d| mentions(&self.ast, sp, d))) {
                    declaring.push(n);
                }
            }
            if declaring.len() == before {
                break;
            }
        }
        if declaring.is_empty() {
            return;
        }
        let ItemKind::Function(f) = &self.ast.items[si as usize].kind else { return };
        let body = f.body;
        let StmtKind::Block(ss) = &self.ast.stmts[body as usize].kind else { return };
        let ss = ss.clone();
        let mut kept = Vec::new();
        for s in ss {
            let calls = matches!(self.ast.stmts[s as usize].kind, StmtKind::Expr(e) if matches!(&self.ast.exprs[e as usize].kind, ExprKind::Call { callee, .. } if matches!(self.ast.exprs[*callee as usize].kind, ExprKind::Ident(n) if declaring.contains(&n))));
            if !calls {
                kept.push(s);
                continue;
            }
            let span = self.ast.stmts[s as usize].span;
            let b = self.mk_stmt(StmtKind::Block(vec![s]), span);
            let arrow = self.mk_expr(ExprKind::Arrow(Box::new(ArrowFn { params: Vec::new(), ret: None, throws: None, body: ArrowBody::Block(b), is_async: false })), span);
            self.ast.items.push(Item { kind: ItemKind::Test { name: String::new(), name_span: span, body: arrow, suite: true }, span, exported: false });
        }
        self.ast.stmts[body as usize].kind = StmtKind::Block(kept);
    }

    /// Tests declared inside a suite's loops, `if`s and blocks (`for (const f of fns) it(...)`)
    /// are its tests too, declared as the statement runs.
    fn suite_nested(&mut self, s: StmtId, prefix: &str, before: &[Sym], after: &[Sym]) {
        let inner: Vec<StmtId> = match &self.ast.stmts[s as usize].kind {
            StmtKind::For { body, .. } | StmtKind::ForOf { body, .. } | StmtKind::While(_, body) | StmtKind::DoWhile(body, _) => vec![*body],
            StmtKind::If(_, t, e) => std::iter::once(*t).chain(*e).collect(),
            StmtKind::Block(_) => return self.suite_block(s, prefix, before, after),
            _ => return,
        };
        for b in inner {
            if !matches!(self.ast.stmts[b as usize].kind, StmtKind::Block(_)) {
                let span = self.ast.stmts[b as usize].span;
                let k = std::mem::replace(&mut self.ast.stmts[b as usize].kind, StmtKind::Empty);
                let one = self.mk_stmt(k, span);
                self.ast.stmts[b as usize].kind = StmtKind::Block(vec![one]);
            }
            self.suite_block(b, prefix, before, after);
        }
    }

    fn suite_block(&mut self, body: StmtId, prefix: &str, before: &[Sym], after: &[Sym]) {
        let StmtKind::Block(ss) = &self.ast.stmts[body as usize].kind else { return };
        let ss = ss.clone();
        let (mut before, mut after) = (before.to_vec(), after.to_vec());
        let mut out = Vec::new();
        let mut tail = Vec::new();
        for s in ss {
            if let StmtKind::Expr(e) = self.ast.stmts[s as usize].kind
                && let Some((table, n, f)) = self.each_call(e)
            {
                let span = self.ast.stmts[s as usize].span;
                match self.each_stmt(table, n, f, prefix, &before, &after, span) {
                    Some(st) => out.push(st),
                    None => out.push(s),
                }
                continue;
            }
            let call = match self.ast.stmts[s as usize].kind {
                StmtKind::Expr(e) => self.jest_call(e).map(|c| (e, c)),
                _ => None,
            };
            let Some((e, (kind, modifier, args))) = call else {
                self.suite_nested(s, prefix, &before, &after);
                out.push(s);
                continue;
            };
            let span = self.ast.stmts[s as usize].span;
            if matches!(modifier.as_deref(), Some("skip" | "todo")) {
                continue;
            }
            match (kind.as_str(), &args[..]) {
                ("beforeEach" | "afterEach", &[f]) => {
                    let h = self.fresh_name();
                    let fs = self.expr_span(f);
                    out.push(self.mk_stmt(StmtKind::Let { mutable: false, name: h, name_span: fs, ty: None, init: Some(f) }, span));
                    if kind == "beforeEach" { before.push(h) } else { after.push(h) }
                }
                ("beforeAll" | "afterAll", &[f]) => {
                    let fs = self.expr_span(f);
                    let c = self.mk_expr(ExprKind::Call { callee: f, type_args: Vec::new(), args: Vec::new(), optional: false }, fs);
                    let st = self.mk_stmt(StmtKind::Expr(c), span);
                    if kind == "beforeAll" { out.push(st) } else { tail.push(st) }
                }
                ("describe", &[n, f, ..]) => {
                    let (name, own) = if self.runtime_tests { (String::new(), Some(n)) } else { (self.jest_name(prefix, n), None) };
                    self.suite_body(f, &name, &before, &after, own);
                    match &self.ast.exprs[f as usize].kind {
                        ExprKind::Arrow(a) => match a.body {
                            ArrowBody::Block(b) => out.push(b),
                            ArrowBody::Expr(x) => out.push(self.mk_stmt(StmtKind::Expr(x), span)),
                        },
                        _ => out.push(s),
                    }
                }
                ("it" | "test", &[n, f, ..]) => {
                    let ns = self.expr_span(n);
                    let full = match &self.ast.exprs[n as usize].kind {
                        _ if self.runtime_tests => n,
                        ExprKind::Str(t) => {
                            let t = format!("{prefix} > {}", self.interner.get(*t));
                            let t = self.interner.intern(&t);
                            self.mk_expr(ExprKind::Str(t), ns)
                        }
                        _ => {
                            let p = self.interner.intern(&format!("{prefix} > "));
                            let p = self.mk_expr(ExprKind::Str(p), ns);
                            self.mk_expr(ExprKind::Binary(BinOp::Add, p, n), ns)
                        }
                    };
                    self.with_hooks(f, &before);
                    let test = self.interner.intern(if self.runtime_tests { "__testHere" } else { "__test" });
                    let cs = self.ast.exprs[e as usize].span;
                    let callee = self.mk_expr(ExprKind::Ident(test), cs);
                    let mut args = vec![Arg { expr: full, by_ref: None }, Arg { expr: f, by_ref: None }];
                    // (the `afterEach` callbacks, run however the test ends)
                    if !after.is_empty() {
                        let ss: Vec<StmtId> = after.iter().map(|&h| self.hook_call(h, cs)).collect();
                        let b = self.mk_stmt(StmtKind::Block(ss), cs);
                        let a = self.mk_expr(ExprKind::Arrow(Box::new(ArrowFn { params: Vec::new(), ret: None, throws: None, body: ArrowBody::Block(b), is_async: false })), cs);
                        args.push(Arg { expr: a, by_ref: None });
                    }
                    let c = self.mk_expr(ExprKind::Call { callee, type_args: Vec::new(), args, optional: false }, cs);
                    out.push(self.mk_stmt(StmtKind::Expr(c), span));
                }
                _ => out.push(s),
            }
        }
        out.extend(tail);
        self.ast.stmts[body as usize].kind = StmtKind::Block(out);
    }

    /// `it.each(table)(name, f)` / `test.each(...)(...)`: (table, name, f).
    fn each_call(&self, e: ExprId) -> Option<(ExprId, ExprId, ExprId)> {
        let ExprKind::Call { callee, args, .. } = &self.ast.exprs[e as usize].kind else { return None };
        let ExprKind::Call { callee: inner, args: targs, .. } = &self.ast.exprs[*callee as usize].kind else { return None };
        let ExprKind::Member { obj, name, .. } = &self.ast.exprs[*inner as usize].kind else { return None };
        let ExprKind::Ident(s) = self.ast.exprs[*obj as usize].kind else { return None };
        if !matches!(self.interner.get(s), "it" | "test") || self.interner.get(*name) != "each" {
            return None;
        }
        match (&targs[..], &args[..]) {
            ([t], [n, f, ..]) => Some((t.expr, n.expr, f.expr)),
            _ => None,
        }
    }

    /// `it.each(table)(name, (a, b) => body)`: a test per row,
    /// `for (const row of table) __test(name of row, () => { const a = row[0]; const b = row[1]; body })`.
    /// A row is spread over the parameters when it's an array (two parameters or more, a rest
    /// one, or a table of array literals), else it's the one parameter (`({ input }) => ...`).
    /// A table of array literals is `as const` (its rows tuples, as Jest's types infer). The
    /// name's `%s`/`%p`/`%d`... show the row's values, `$field` an object row's field, `%#` the
    /// row number.
    #[allow(clippy::too_many_arguments)]
    fn each_stmt(&mut self, table: ExprId, n: ExprId, f: ExprId, prefix: &str, before: &[Sym], after: &[Sym], span: Span) -> Option<StmtId> {
        let ExprKind::Arrow(a) = &self.ast.exprs[f as usize].kind else { return None };
        if a.is_async {
            return None;
        }
        let params: Vec<(Sym, Span, Option<TypeId>, bool)> = a.params.iter().map(|p| (p.name, p.span, p.ty, p.rest)).collect();
        let fbody = match a.body {
            ArrowBody::Block(b) => b,
            ArrowBody::Expr(x) => {
                let xs = self.expr_span(x);
                self.mk_stmt(StmtKind::Expr(x), xs)
            }
        };
        let literal_rows = matches!(&self.ast.exprs[table as usize].kind, ExprKind::Array(rows) if !rows.is_empty() && rows.iter().all(|&r| matches!(self.ast.exprs[r as usize].kind, ExprKind::Array(_))));
        let spread = params.len() >= 2 || params.first().is_some_and(|p| p.3) || (params.len() == 1 && literal_rows);
        let ts = self.expr_span(table);
        let table = if literal_rows {
            let c = self.interner.intern("const");
            let te = self.mk_ty(TypeExprKind::Named { ns: None, name: c, name_span: ts, args: Vec::new() }, ts);
            self.mk_expr(ExprKind::As(table, te), ts)
        } else {
            table
        };
        let (row, idx) = (self.fresh_name(), self.fresh_name());
        let row_ref = |p: &mut Self| p.mk_expr(ExprKind::Ident(row), span);
        // the parameters, from the row
        let mut ss = Vec::new();
        for (j, &(name, pspan, ty, rest)) in params.iter().enumerate() {
            let r = row_ref(self);
            let init = if !spread && !literal_rows {
                // (a table made otherwise, `xs.map((x) => [x])`: an array row is spread, as Jest
                // spreads it, which its type tells)
                let f = self.interner.intern("__eachArg");
                let callee = self.mk_expr(ExprKind::Ident(f), pspan);
                self.mk_expr(ExprKind::Call { callee, type_args: Vec::new(), args: vec![Arg { expr: r, by_ref: None }], optional: false }, pspan)
            } else if !spread {
                r
            } else if rest {
                if j == 0 {
                    r
                } else {
                    let slice = self.interner.intern("slice");
                    let callee = self.mk_expr(ExprKind::Member { obj: r, name: slice, name_span: pspan, optional: false }, pspan);
                    let k = self.mk_expr(ExprKind::Int(j as u64), pspan);
                    self.mk_expr(ExprKind::Call { callee, type_args: Vec::new(), args: vec![Arg { expr: k, by_ref: None }], optional: false }, pspan)
                }
            } else {
                let k = self.mk_expr(ExprKind::Int(j as u64), pspan);
                self.mk_expr(ExprKind::Index { obj: r, index: k, optional: false }, pspan)
            };
            ss.push(self.mk_stmt(StmtKind::Let { mutable: false, name, name_span: pspan, ty, init: Some(init) }, pspan));
            if !spread {
                break;
            }
        }
        ss.push(fbody);
        // (the function's own span: the row and its number are declared outside it)
        let fspan = self.expr_span(f);
        let b = self.mk_stmt(StmtKind::Block(ss), fspan);
        let test_fn = self.mk_expr(ExprKind::Arrow(Box::new(ArrowFn { params: Vec::new(), ret: None, throws: None, body: ArrowBody::Block(b), is_async: false })), fspan);
        self.with_hooks(test_fn, before);
        // the name
        let ns = self.expr_span(n);
        let name = match &self.ast.exprs[n as usize].kind {
            ExprKind::Str(t) => {
                let text = self.interner.get(*t).to_string();
                let (mut parts, mut vals) = (vec![String::new()], Vec::new());
                let mut next = 0u64;
                let mut cs = text.chars().peekable();
                while let Some(c) = cs.next() {
                    let v = match (c, cs.peek().copied()) {
                        ('%', Some('%')) => {
                            cs.next();
                            parts.last_mut().unwrap().push('%');
                            continue;
                        }
                        ('%', Some('#')) => {
                            cs.next();
                            self.mk_expr(ExprKind::Ident(idx), ns)
                        }
                        ('%', Some('p' | 's' | 'd' | 'i' | 'f' | 'j' | 'o')) => {
                            cs.next();
                            let r = row_ref(self);
                            if spread {
                                let k = self.mk_expr(ExprKind::Int(next), ns);
                                next += 1;
                                self.mk_expr(ExprKind::Index { obj: r, index: k, optional: false }, ns)
                            } else {
                                r
                            }
                        }
                        ('$', Some(d)) if !spread && (d.is_alphabetic() || d == '_') => {
                            let mut field = String::new();
                            while let Some(&d) = cs.peek() {
                                if !(d.is_alphanumeric() || d == '_') {
                                    break;
                                }
                                field.push(d);
                                cs.next();
                            }
                            let r = row_ref(self);
                            let fname = self.interner.intern(&field);
                            self.mk_expr(ExprKind::Member { obj: r, name: fname, name_span: ns, optional: false }, ns)
                        }
                        _ => {
                            parts.last_mut().unwrap().push(c);
                            continue;
                        }
                    };
                    vals.push(v);
                    parts.push(String::new());
                }
                if !prefix.is_empty() {
                    parts[0].insert_str(0, &format!("{prefix} > "));
                }
                self.mk_expr(ExprKind::Template(parts, vals), ns)
            }
            _ if prefix.is_empty() => n,
            _ => {
                let p = self.interner.intern(&format!("{prefix} > "));
                let p = self.mk_expr(ExprKind::Str(p), ns);
                self.mk_expr(ExprKind::Binary(BinOp::Add, p, n), ns)
            }
        };
        let callee = {
            let t = self.interner.intern(if self.runtime_tests { "__testHere" } else { "__test" });
            self.mk_expr(ExprKind::Ident(t), span)
        };
        let mut args = vec![Arg { expr: name, by_ref: None }, Arg { expr: test_fn, by_ref: None }];
        if !after.is_empty() {
            let ss: Vec<StmtId> = after.iter().map(|&h| self.hook_call(h, span)).collect();
            let b = self.mk_stmt(StmtKind::Block(ss), span);
            let a = self.mk_expr(ExprKind::Arrow(Box::new(ArrowFn { params: Vec::new(), ret: None, throws: None, body: ArrowBody::Block(b), is_async: false })), span);
            args.push(Arg { expr: a, by_ref: None });
        }
        let call = self.mk_expr(ExprKind::Call { callee, type_args: Vec::new(), args, optional: false }, span);
        let call = self.mk_stmt(StmtKind::Expr(call), span);
        let inc = {
            let t = self.mk_expr(ExprKind::Ident(idx), span);
            let u = self.mk_expr(ExprKind::Update { inc: true, prefix: false, target: t }, span);
            self.mk_stmt(StmtKind::Expr(u), span)
        };
        let lb = self.mk_stmt(StmtKind::Block(vec![call, inc]), span);
        let lp = self.mk_stmt(StmtKind::ForOf { mutable: false, name: row, name_span: ts, iter: table, body: lb }, span);
        let zero = self.mk_expr(ExprKind::Int(0), span);
        let init = self.mk_stmt(StmtKind::Let { mutable: true, name: idx, name_span: span, ty: None, init: Some(zero) }, span);
        Some(self.mk_stmt(StmtKind::Block(vec![init, lp]), span))
    }

    /// A nested suite's name: `Outer > inner`.
    fn jest_name(&self, prefix: &str, n: ExprId) -> String {
        let inner = match &self.ast.exprs[n as usize].kind {
            ExprKind::Str(s) => self.interner.get(*s).to_string(),
            _ => self.src_text(self.expr_span(n)),
        };
        format!("{prefix} > {inner}")
    }

    /// `h();`
    fn hook_call(&mut self, h: Sym, span: Span) -> StmtId {
        let callee = self.mk_expr(ExprKind::Ident(h), span);
        let c = self.mk_expr(ExprKind::Call { callee, type_args: Vec::new(), args: Vec::new(), optional: false }, span);
        self.mk_stmt(StmtKind::Expr(c), span)
    }

    /// A test function `() => body` that calls `before` first.
    fn with_hooks(&mut self, f: ExprId, before: &[Sym]) {
        if before.is_empty() {
            return;
        }
        let span = self.expr_span(f);
        let ExprKind::Arrow(a) = &self.ast.exprs[f as usize].kind else { return };
        if !a.params.is_empty() || a.is_async {
            return;
        }
        let inner = match a.body {
            ArrowBody::Block(b) => b,
            ArrowBody::Expr(x) => self.mk_stmt(StmtKind::Expr(x), span),
        };
        let mut ss: Vec<StmtId> = before.iter().map(|&h| self.hook_call(h, span)).collect();
        ss.push(inner);
        let b = self.mk_stmt(StmtKind::Block(ss), span);
        if let ExprKind::Arrow(a) = &mut self.ast.exprs[f as usize].kind {
            a.body = ArrowBody::Block(b);
        }
    }

    // ---------------------------------------------------------------- types

    /// `A & B & ...` (binding tighter than `|`).
    fn intersection_ty(&mut self) -> TypeId {
        let start = self.cur_span();
        let first = self.postfix_ty();
        if !self.at(Tok::Amp) {
            return first;
        }
        let mut members = vec![first];
        while self.eat(Tok::Amp) {
            members.push(self.postfix_ty());
        }
        let span = start.to(self.prev_span());
        self.mk_ty(TypeExprKind::Intersection(members), span)
    }

    fn ty(&mut self) -> TypeId {
        let start = self.cur_span();
        // `x is T` (a type predicate, as a function's result)
        if matches!(self.kind(), Tok::Ident) && self.nth(1) == Tok::Ident && self.text(self.toks[self.pos + 1]) == "is" && !self.toks[self.pos + 1].nl_before {
            let (name, _) = self.ident("a parameter name").unwrap();
            self.bump(); // is
            let t = self.ty();
            let span = start.to(self.prev_span());
            return self.mk_ty(TypeExprKind::Predicate(name, t), span);
        }
        self.eat(Tok::Pipe);
        let first = self.intersection_ty();
        if !self.at(Tok::Pipe) {
            return first;
        }
        let mut members = vec![first];
        while self.eat(Tok::Pipe) {
            members.push(self.intersection_ty());
        }
        let span = start.to(self.prev_span());
        self.mk_ty(TypeExprKind::Union(members), span)
    }

    fn postfix_ty(&mut self) -> TypeId {
        let start = self.cur_span();
        // `readonly T[]`, `readonly [A, B]`: the array type (Tov checks no writes through it)
        if self.at_word("readonly") && matches!(self.nth(1), Tok::Ident | Tok::LParen | Tok::LBracket) {
            self.bump();
            return self.postfix_ty();
        }
        let mut t = self.primary_ty();
        while self.at(Tok::LBracket) && !self.tok().nl_before {
            self.bump();
            if self.eat(Tok::RBracket) {
                let span = start.to(self.prev_span());
                t = self.mk_ty(TypeExprKind::Array(t), span);
                continue;
            }
            // `T["k"]`
            let key = self.ty();
            self.expect(Tok::RBracket, "to close the indexed access type");
            let span = start.to(self.prev_span());
            t = self.mk_ty(TypeExprKind::Access(t, key), span);
        }
        t
    }

    fn primary_ty(&mut self) -> TypeId {
        let start = self.cur_span();
        match self.kind() {
            Tok::LParen => {
                if let Some(close) = self.matching_paren()
                    && self.toks.get(close + 1).map(|t| t.kind) == Some(Tok::Arrow) {
                        let params = self.params();
                        self.defaults.clear();
                    self.prologue.clear();
                        self.prologue.clear();
                        self.expect(Tok::Arrow, "in the function type");
                        let ret = self.ty();
                        let throws = if self.at_word("throws") {
                            self.bump();
                            Some(self.ty())
                        } else {
                            None
                        };
                        let span = start.to(self.prev_span());
                        return self.mk_ty(TypeExprKind::Func(params, ret, throws), span);
                    }
                self.bump();
                let t = self.ty();
                self.expect(Tok::RParen, "to close the parenthesized type");
                t
            }
            Tok::LBrace => {
                let saved = self.index_sig.take();
                let saved_call = self.call_sig.take();
                let fields = self.record_fields();
                let span = start.to(self.prev_span());
                let sig = std::mem::replace(&mut self.index_sig, saved);
                if let Some((f, cspan)) = std::mem::replace(&mut self.call_sig, saved_call) {
                    if !fields.is_empty() || sig.is_some() {
                        self.err("U0021", cspan, "a call signature with other members is not supported yet");
                    }
                    return f;
                }
                if let Some((k, v, sspan)) = sig {
                    if !fields.is_empty() {
                        self.err("X0017", sspan, "an index signature with other fields is not supported; use a `Map<K, V>` field for the dynamic keys");
                    }
                    let rec = self.interner.intern("Record");
                    return self.mk_ty(TypeExprKind::Named { ns: None, name: rec, name_span: span, args: vec![k, v] }, span);
                }
                self.mk_ty(TypeExprKind::Record(fields), span)
            }
            // `` `px` `` is that string; `` `${number}px` `` a template literal type: a string
            Tok::TemplateFull => {
                let t = self.bump();
                let s = self.interner.intern(&self.strings[t.val as usize]);
                self.mk_ty(TypeExprKind::StrLit(s), start)
            }
            Tok::TemplateHead => {
                self.bump();
                loop {
                    self.ty();
                    if self.eat(Tok::TemplateMiddle) {
                        continue;
                    }
                    if !self.eat(Tok::TemplateTail) {
                        self.expect(Tok::TemplateTail, "to close the template literal type");
                    }
                    break;
                }
                let span = start.to(self.prev_span());
                let string = self.interner.intern("string");
                self.mk_ty(TypeExprKind::Named { ns: None, name: string, name_span: span, args: Vec::new() }, span)
            }
            // `new () => Queue<number>`, `new <T>() => Queue<T>`: a class passed as a value
            Tok::New => {
                self.bump();
                self.interner.intern("new");
                let tparams = self.tparams();
                if !self.at(Tok::LParen) {
                    self.expect(Tok::LParen, "for the constructor type's parameters");
                    return self.mk_ty(TypeExprKind::Error, start);
                }
                let params = self.params();
                self.defaults.clear();
                self.prologue.clear();
                self.expect(Tok::Arrow, "in the constructor type");
                let ret = self.ty();
                let span = start.to(self.prev_span());
                self.mk_ty(TypeExprKind::Ctor(tparams, params, ret), span)
            }
            Tok::Str => {
                let t = self.bump();
                let s = self.interner.intern(&self.strings[t.val as usize]);
                self.mk_ty(TypeExprKind::StrLit(s), start)
            }
            Tok::True | Tok::False => {
                let b = self.at(Tok::True);
                self.bump();
                self.mk_ty(TypeExprKind::BoolLit(b), start)
            }
            // (`1 | 2 | 3`: a number; Tov has no numeric literal types)
            Tok::Int | Tok::Float => {
                self.bump();
                let number = self.interner.intern("number");
                self.mk_ty(TypeExprKind::Named { ns: None, name: number, name_span: start, args: Vec::new() }, start)
            }
            Tok::BigInt | Tok::Decimal => {
                self.bump();
                self.err("U0007", start, "bigint and decimal literal types are not supported yet; use `bigint` or `decimal`");
                self.mk_ty(TypeExprKind::Error, start)
            }
            Tok::Undefined => {
                self.bump();
                self.mk_ty(TypeExprKind::Undefined, start)
            }
            Tok::Null => {
                self.bump();
                self.mk_ty(TypeExprKind::Null, start)
            }
            Tok::Void => {
                self.bump();
                self.mk_ty(TypeExprKind::Void, start)
            }
            Tok::Typeof => {
                self.bump();
                let x = self.postfix_expr_no_call();
                let span = start.to(self.prev_span());
                self.mk_ty(TypeExprKind::Typeof(x), span)
            }
            Tok::LBracket => {
                // `[A, B]`: a tuple, a record whose fields are `0`, `1`, ...
                self.bump();
                let mut fields = Vec::new();
                while !self.at(Tok::RBracket) && !self.at(Tok::Eof) {
                    let fstart = self.cur_span();
                    if self.at(Tok::DotDotDot) {
                        self.err("U0008", fstart, "rest elements in tuple types are not supported yet");
                        self.bump();
                    }
                    // (a labelled element: `[x: number, y: number]`)
                    if matches!(self.kind(), Tok::Ident) && matches!(self.nth(1), Tok::Colon | Tok::Question) && !matches!(self.nth(2), Tok::Comma | Tok::RBracket) {
                        self.bump();
                    }
                    let optional = self.eat(Tok::Question);
                    self.eat(Tok::Colon);
                    let ty = self.ty();
                    let optional = optional || self.eat(Tok::Question);
                    let name = self.interner.intern(&fields.len().to_string());
                    fields.push(FieldTy { name, name_span: fstart, optional, ty });
                    if !self.eat(Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::RBracket, "to close the tuple type");
                let span = start.to(self.prev_span());
                self.mk_ty(TypeExprKind::Record(fields), span)
            }
            Tok::Ident => {
                let t = self.bump();
                let word = self.text(t);
                if word == "keyof" && matches!(self.kind(), Tok::Ident | Tok::Typeof | Tok::LParen) {
                    let inner = self.postfix_ty();
                    let span = start.to(self.prev_span());
                    return self.mk_ty(TypeExprKind::Keyof(inner), span);
                }
                let mut name = self.interner.intern(word);
                let mut name_span = start;
                let mut ns = None;
                if self.at(Tok::Dot) && self.adjacent() {
                    self.bump();
                    if let Some((n, s)) = self.ident("a type name") {
                        ns = Some((name, name_span));
                        name = n;
                        name_span = s;
                    }
                }
                let mut args = Vec::new();
                if self.at(Tok::Lt) && !self.tok().nl_before {
                    self.bump();
                    while !self.at(Tok::Gt) && !self.at(Tok::Eof) {
                        args.push(self.ty());
                        if !self.eat(Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::Gt, "to close the type argument list");
                }
                let span = start.to(self.prev_span());
                let name = match ns {
                    None => self.local_types.iter().rev().find(|t| t.0 == name).map_or(name, |t| t.1),
                    Some(_) => name,
                };
                self.mk_ty(TypeExprKind::Named { ns, name, name_span, args }, span)
            }
            // `this` as a type, in a class: the class (see `check::class`)
            Tok::This => {
                self.bump();
                let name = self.interner.intern("this");
                self.mk_ty(TypeExprKind::Named { ns: None, name, name_span: start, args: Vec::new() }, start)
            }
            _ => {
                let msg = format!("expected a type, found {}", self.found());
                self.err("P0003", start, msg);
                self.mk_ty(TypeExprKind::Error, start)
            }
        }
    }

    /// `{ a: T, b?: U, m(x: T): R }` for record types and interfaces.
    fn record_fields(&mut self) -> Vec<FieldTy> {
        self.bump(); // {
        let mut fields = Vec::new();
        while !self.at(Tok::RBrace) && !self.at(Tok::Eof) {
            let start = self.cur_span();
            if self.at_word("readonly") && self.nth(1) != Tok::Colon && self.nth(1) != Tok::Question {
                self.bump(); // values are copied, so `readonly` adds nothing; accept it for familiarity
            }
            if self.at(Tok::LBracket) {
                // `{ [k: string]: V }`: a `Record<string, V>` (a map)
                self.bump();
                self.ident("a key name");
                self.expect(Tok::Colon, "after the key name");
                let k = self.ty();
                self.expect(Tok::RBracket, "to close the index signature");
                self.expect(Tok::Colon, "before the value type");
                let v = self.ty();
                self.index_sig = Some((k, v, start));
            } else if let Some((name, name_span)) = self.prop_name() {
                let optional = self.eat(Tok::Question);
                if self.at(Tok::LParen) || self.at(Tok::Lt) {
                    // Method signature: `area(): f64` is a field of function type.
                    let tps = self.tparams();
                    if !tps.is_empty() {
                        self.err("U0015", start, "generic methods in interfaces are not supported yet");
                    }
                    let params = self.params();
                    self.defaults.clear();
                    self.prologue.clear();
                    self.expect(Tok::Colon, "before the method return type");
                    let ret = self.ty();
                    let throws = self.effects_clause();
                    let span = start.to(self.prev_span());
                    let ty = self.mk_ty(TypeExprKind::Func(params, ret, throws), span);
                    fields.push(FieldTy { name, name_span, optional, ty });
                } else {
                    self.expect(Tok::Colon, "after the field name");
                    let ty = self.ty();
                    fields.push(FieldTy { name, name_span, optional, ty });
                }
            } else if self.at(Tok::LParen) {
                // `{ (array: number[], target: number): number }`: a function type
                let params = self.params();
                self.defaults.clear();
                self.prologue.clear();
                self.expect(Tok::Colon, "before the call signature's return type");
                let ret = self.ty();
                let throws = self.effects_clause();
                let span = start.to(self.prev_span());
                let f = self.mk_ty(TypeExprKind::Func(params, ret, throws), span);
                if self.call_sig.is_some() {
                    self.err("U0021", span, "a type with more than one call signature (an overloaded function type) is not supported yet");
                }
                self.call_sig = Some((f, span));
            } else {
                let msg = format!("expected a field name, found {}", self.found());
                self.err("P0001", start, msg);
                self.recover_line();
                if self.at(Tok::RBrace) {
                    break;
                }
                continue;
            }
            if !(self.eat(Tok::Comma) || self.eat(Tok::Semi) || self.at(Tok::RBrace) || self.tok().nl_before) {
                let msg = format!("expected `,` between fields, found {}", self.found());
                let span = self.cur_span();
                self.err("P0001", span, msg);
                self.recover_line();
            }
        }
        self.expect(Tok::RBrace, "to close the record type");
        fields
    }

    // ---------------------------------------------------------------- statements

    fn block(&mut self) -> StmtId {
        let start = self.cur_span();
        let types = self.local_types.len();
        self.expect(Tok::LBrace, "to start a block");
        let mut stmts = Vec::new();
        while !self.at(Tok::RBrace) && !self.at(Tok::Eof) {
            let before = self.pos;
            stmts.push(self.stmt());
            stmts.append(&mut self.more_stmts);
            if self.pos == before {
                self.bump();
            }
        }
        self.expect(Tok::RBrace, "to close the block");
        self.local_types.truncate(types);
        self.hoist_functions(&mut stmts);
        let span = start.to(self.prev_span());
        self.mk_stmt(StmtKind::Block(stmts), span)
    }

    /// `f(args)`; `Array(n)` / `Array<T>(n)` is `new Array(n)`, as in JavaScript.
    fn call_or_new(&mut self, callee: ExprId, type_args: Vec<TypeId>, args: Vec<Arg>, span: Span) -> ExprId {
        if matches!(self.ast.exprs[callee as usize].kind, ExprKind::Ident(s) if self.interner.get(s) == "Array") {
            return self.mk_expr(ExprKind::New { callee, type_args, args }, span);
        }
        self.mk_expr(ExprKind::Call { callee, type_args, args, optional: false }, span)
    }

    fn local_function(&mut self, start: Span, is_async: bool) -> StmtId {
        let Some(mut f) = self.function() else { return self.mk_stmt(StmtKind::Error, start) };
        let span = start.to(self.prev_span());
        // (a generator, or a generic function: a module function of its own, when it uses nothing
        // of the function around it; see `lift_functions`)
        if f.is_gen || !f.tparams.is_empty() {
            f.is_async = is_async;
            self.lift(Some(f.name), f, span);
            return self.mk_stmt(StmtKind::Empty, span);
        }
        let (name, name_span) = (f.name, f.name_span);
        let init = self.arrow_of(f, is_async, span);
        let s = self.mk_stmt(StmtKind::Let { mutable: false, name, name_span, ty: None, init: Some(init) }, span);
        self.local_fns.push(s);
        s
    }

    /// `f` (written at `span`), as a module function of a hidden name: what it's called by.
    fn lift(&mut self, name: Option<Sym>, mut f: FnDecl, span: Span) -> Sym {
        let base = name.map(|n| self.interner.get(n).to_string()).unwrap_or_else(|| "gen".into());
        let hidden = self.interner.intern(&format!("__{base}_{}", self.lifted.len()));
        f.name = hidden;
        self.ast.items.push(Item { kind: ItemKind::Function(f), span, exported: false });
        self.lifted.push((name, hidden, span));
        hidden
    }

    /// The nested generators and generic functions `lift` made module functions: each is called
    /// by its module name in the function around it, which works when it uses nothing of that
    /// function's (its parameters and variables); otherwise it's an error, as before.
    fn lift_functions(&mut self) {
        let lifted = std::mem::take(&mut self.lifted);
        for (name, hidden, span) in lifted {
            let within = |s: Span, outer: Span| s.file == outer.file && s.start >= outer.start && s.end <= outer.end;
            // (the function around it: the smallest function body or arrow holding it)
            let mut around: Option<(Span, Vec<Sym>)> = None;
            let consider = |sp: Span, params: Vec<Sym>, around: &mut Option<(Span, Vec<Sym>)>| {
                if within(span, sp) && sp != span && around.as_ref().is_none_or(|(a, _)| sp.end - sp.start < a.end - a.start) {
                    *around = Some((sp, params));
                }
            };
            for it in &self.ast.items {
                match &it.kind {
                    ItemKind::Function(f) if f.name != hidden => consider(self.ast.stmts[f.body as usize].span, f.params.iter().map(|p| p.name).collect(), &mut around),
                    ItemKind::Class(c) => {
                        for m in &c.members {
                            if let crate::ast::MemberKind::Method(f) | crate::ast::MemberKind::Getter(f) | crate::ast::MemberKind::Constructor(f) = &m.kind {
                                consider(self.ast.stmts[f.body as usize].span, f.params.iter().map(|p| p.name).collect(), &mut around);
                            }
                        }
                    }
                    _ => {}
                }
            }
            for e in &self.ast.exprs {
                if let ExprKind::Arrow(a) = &e.kind {
                    consider(e.span, a.params.iter().map(|p| p.name).collect(), &mut around);
                }
            }
            let Some((outer, mut declared)) = around else { continue };
            // (what the function around declares outside it: its parameters, its variables, the
            // parameters of the closures in it)
            for st in &self.ast.stmts {
                match &st.kind {
                    StmtKind::Let { name: n, name_span, .. } | StmtKind::ForOf { name: n, name_span, .. } if within(*name_span, outer) && !within(*name_span, span) => declared.push(*n),
                    _ => {}
                }
            }
            for e in &self.ast.exprs {
                if let ExprKind::Arrow(a) = &e.kind
                    && within(e.span, outer)
                    && !within(e.span, span)
                {
                    declared.extend(a.params.iter().map(|p| p.name));
                }
            }
            let used: Vec<Sym> = self.ast.exprs.iter().filter(|e| within(e.span, span)).filter_map(|e| match e.kind {
                ExprKind::Ident(n) => Some(n),
                _ => None,
            }).collect();
            // (less what it declares itself: its parameters, variables, closures' parameters)
            let mut own: Vec<Sym> = Vec::new();
            for it in &self.ast.items {
                if let ItemKind::Function(f) = &it.kind
                    && f.name == hidden
                {
                    own.extend(f.params.iter().map(|p| p.name));
                }
            }
            for st in &self.ast.stmts {
                match &st.kind {
                    StmtKind::Let { name: n, name_span, .. } | StmtKind::ForOf { name: n, name_span, .. } if within(*name_span, span) => own.push(*n),
                    _ => {}
                }
            }
            for e in &self.ast.exprs {
                if let ExprKind::Arrow(a) = &e.kind
                    && within(e.span, span)
                {
                    own.extend(a.params.iter().map(|p| p.name));
                }
            }
            let captured: Vec<Sym> = used.iter().copied().filter(|n| Some(*n) != name && declared.contains(n) && !own.contains(n)).collect();
            if let Some(&c) = captured.first() {
                let what = self.interner.get(c).to_string();
                self.push(
                    Diagnostic::new("U0013", span, "generators and generic functions in a function are supported only when they use none of its variables")
                        .note("why", format!("this one uses `{what}` from the function around it"))
                        .note("instead", "declare it at the top level, passing what it uses"),
                );
                continue;
            }
            if let Some(n) = name {
                for e in self.ast.exprs.iter_mut() {
                    if matches!(e.kind, ExprKind::Ident(x) if x == n) && within(e.span, outer) {
                        e.kind = ExprKind::Ident(hidden);
                    }
                }
            }
        }
    }

    fn arrow_of(&mut self, f: FnDecl, is_async: bool, span: Span) -> ExprId {
        if f.is_gen {
            self.err("U0013", f.name_span, "generator functions are supported at the top level and as methods only yet");
        }
        let arrow = ArrowFn { params: f.params, ret: f.ret, throws: f.throws, body: ArrowBody::Block(f.body), is_async };
        self.mk_expr(ExprKind::Arrow(Box::new(arrow)), span)
    }

    /// Function declarations are hoisted in JavaScript: each of a block's moves up to before the
    /// first statement that uses it (an earlier one's body, say).
    fn hoist_functions(&mut self, stmts: &mut Vec<StmtId>) {
        let fns: Vec<StmtId> = stmts.iter().copied().filter(|s| self.local_fns.contains(s)).collect();
        if fns.is_empty() {
            return;
        }
        for _ in 0..fns.len() {
            let mut moved = false;
            for &f in &fns {
                let StmtKind::Let { name, name_span, .. } = self.ast.stmts[f as usize].kind else { continue };
                let uses: Vec<u32> = self
                    .ast
                    .exprs
                    .iter()
                    .filter(|e| matches!(e.kind, ExprKind::Ident(n) if n == name) && e.span.file == name_span.file)
                    .map(|e| e.span.start)
                    .collect();
                let at = stmts.iter().position(|&x| x == f).unwrap();
                let first = stmts[..at].iter().position(|&x| {
                    let sp = self.ast.stmts[x as usize].span;
                    uses.iter().any(|&u| u >= sp.start && u < sp.end)
                });
                if let Some(to) = first {
                    stmts.remove(at);
                    stmts.insert(to, f);
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
    }

    fn stmt(&mut self) -> StmtId {
        let start = self.cur_span();
        match self.kind() {
            Tok::LBrace => self.block(),
            Tok::Const | Tok::Let | Tok::Var => {
                let s = self.let_stmt();
                self.terminator();
                s
            }
            Tok::If => {
                self.bump();
                self.expect(Tok::LParen, "after `if`");
                let cond = self.expr();
                self.expect(Tok::RParen, "to close the `if` condition");
                let then = self.single_stmt();
                let els = if self.eat(Tok::Else) { Some(self.single_stmt()) } else { None };
                let span = start.to(self.prev_span());
                self.mk_stmt(StmtKind::If(cond, then, els), span)
            }
            Tok::While => {
                self.bump();
                self.expect(Tok::LParen, "after `while`");
                let cond = self.expr();
                self.expect(Tok::RParen, "to close the `while` condition");
                let body = self.single_stmt();
                let span = start.to(self.prev_span());
                self.mk_stmt(StmtKind::While(cond, body), span)
            }
            Tok::Do => {
                self.bump();
                let body = self.single_stmt();
                self.expect(Tok::While, "after the `do` body");
                self.expect(Tok::LParen, "after `while`");
                let cond = self.expr();
                self.expect(Tok::RParen, "to close the `while` condition");
                self.terminator();
                let span = start.to(self.prev_span());
                self.mk_stmt(StmtKind::DoWhile(body, cond), span)
            }
            Tok::For => self.for_stmt(),
            Tok::Switch => self.switch_stmt(),
            Tok::Return => {
                self.bump();
                let value = if self.at(Tok::Semi) || self.at(Tok::RBrace) || self.at(Tok::Eof) || self.tok().nl_before { None } else { Some(self.expr()) };
                self.terminator();
                let span = start.to(self.prev_span());
                self.mk_stmt(StmtKind::Return(value), span)
            }
            Tok::Break | Tok::Continue => {
                let brk = self.at(Tok::Break);
                self.bump();
                let label = if self.at(Tok::Ident) && !self.tok().nl_before { self.ident("a label").map(|l| l.0) } else { None };
                self.terminator();
                let span = start.to(self.prev_span());
                self.mk_stmt(if brk { StmtKind::Break(label) } else { StmtKind::Continue(label) }, span)
            }
            Tok::Semi => {
                self.bump();
                self.mk_stmt(StmtKind::Empty, start)
            }
            Tok::Throw => {
                self.bump();
                let e = self.expr();
                self.terminator();
                let span = start.to(self.prev_span());
                self.mk_stmt(StmtKind::Throw(e), span)
            }
            Tok::Try if self.nth(1) == Tok::LBrace => {
                self.bump();
                let body = self.block();
                let mut catch = None;
                if self.eat(Tok::Catch) {
                    let mut param = None;
                    if self.eat(Tok::LParen) {
                        if let Some((name, span)) = self.ident("an error variable name") {
                            let ty = if self.eat(Tok::Colon) { Some(self.ty()) } else { None };
                            param = Some((name, span, ty));
                        }
                        self.expect(Tok::RParen, "to close `catch (...)`");
                    }
                    let cbody = self.block();
                    catch = Some(Catch { param, body: cbody });
                }
                let finally = if self.eat(Tok::Finally) { Some(self.block()) } else { None };
                if catch.is_none() && finally.is_none() {
                    let span = self.cur_span();
                    self.err("P0001", span, "expected `catch` or `finally` after the `try` block");
                }
                let span = start.to(self.prev_span());
                self.mk_stmt(StmtKind::Try { body, catch, finally }, span)
            }
            // `function f(...) { }` in a body: `const f = (...) => { }`, moved up to before its
            // first use in the block (see `hoist_functions`)
            Tok::Function => self.local_function(start, false),
            Tok::Async if self.nth(1) == Tok::Function => {
                self.bump();
                self.local_function(start, true)
            }
            Tok::Type | Tok::Interface if self.nth(1) == Tok::Ident => {
                let name = self.interner.intern(self.text(self.toks[self.pos + 1]));
                let n = self.local_type_items.len();
                let hidden = self.interner.intern(&format!("{}#{n}", self.interner.get(name)));
                self.local_types.push((name, hidden));
                self.local_type_items.push((name, hidden));
                let items = self.ast.items.len();
                if self.at(Tok::Type) {
                    self.type_alias(start, false);
                } else {
                    self.interface(start, false);
                }
                if let Some(item) = self.ast.items.get_mut(items) {
                    match &mut item.kind {
                        ItemKind::TypeAlias { name, .. } | ItemKind::Interface { name, .. } => *name = hidden,
                        _ => {}
                    }
                }
                let span = start.to(self.prev_span());
                self.mk_stmt(StmtKind::Empty, span)
            }
            Tok::Class | Tok::Enum | Tok::Interface | Tok::Import | Tok::Export => {
                let msg = format!("{} declarations are only allowed at the top level", self.found());
                self.err("P0006", start, msg);
                self.skip_decl();
                self.mk_stmt(StmtKind::Error, start)
            }
            Tok::Ident if self.nth(1) == Tok::Colon => {
                let (label, _) = self.ident("a label").unwrap();
                self.bump(); // :
                let body = self.stmt();
                if matches!(self.ast.stmts[body as usize].kind, StmtKind::While(..) | StmtKind::DoWhile(..) | StmtKind::For { .. } | StmtKind::ForOf { .. }) {
                    self.ast.labels.push((body, label));
                } else {
                    self.err("X0018", start, "labels are only supported on loops");
                }
                body
            }
            _ => {
                let e = self.expr();
                // `a = 1, b = 2;`: each in turn (the comma operator, as a statement)
                if self.at(Tok::Comma) {
                    let mut es = vec![e];
                    while self.eat(Tok::Comma) {
                        es.push(self.expr());
                    }
                    self.terminator();
                    let span = start.to(self.prev_span());
                    let stmts: Vec<StmtId> = es.into_iter().map(|x| {
                        let xs = self.expr_span(x);
                        self.mk_stmt(StmtKind::Expr(x), xs)
                    }).collect();
                    return self.mk_stmt(StmtKind::Block(stmts), span);
                }
                self.terminator();
                let span = start.to(self.prev_span());
                // `[a, b] = [b, a]`: the value in a hidden variable, then each assigned
                if let ExprKind::Assign(AssignOp::Assign, target, value) = self.ast.exprs[e as usize].kind
                    && let ExprKind::Array(elems) = &self.ast.exprs[target as usize].kind
                {
                    let elems = elems.clone();
                    let tspan = self.expr_span(target);
                    // `[a, b] = [b, a + b]`: each value in a hidden variable of its own (all of
                    // them first, as JavaScript evaluates the array), with no array made
                    let plain = |p: &Self, x: ExprId| matches!(p.ast.exprs[x as usize].kind, ExprKind::Ident(_) | ExprKind::Member { optional: false, .. } | ExprKind::Index { optional: false, .. });
                    if let ExprKind::Array(vals) = &self.ast.exprs[value as usize].kind
                        && vals.len() == elems.len()
                        // (a value whose type comes from its target keeps the array: `[x, y] = [undefined, []]`)
                        && vals.iter().all(|&v| !matches!(&self.ast.exprs[v as usize].kind, ExprKind::Spread(_) | ExprKind::Undefined | ExprKind::Null | ExprKind::Arrow(_) | ExprKind::Object(_) | ExprKind::Array(_)))
                        && elems.iter().all(|&x| plain(self, x))
                    {
                        let vals = vals.clone();
                        let mut out = Vec::new();
                        let mut tmps = Vec::new();
                        for &v in &vals {
                            let tmp = self.fresh_name();
                            let vs = self.expr_span(v);
                            out.push(self.mk_stmt(StmtKind::Let { mutable: false, name: tmp, name_span: vs, ty: None, init: Some(v) }, vs));
                            tmps.push(tmp);
                        }
                        for (&x, tmp) in elems.iter().zip(tmps) {
                            let xs = self.expr_span(x);
                            let v = self.mk_expr(ExprKind::Ident(tmp), xs);
                            let a = self.mk_expr(ExprKind::Assign(AssignOp::Assign, x, v), xs);
                            out.push(self.mk_stmt(StmtKind::Expr(a), xs));
                        }
                        return self.mk_stmt(StmtKind::Block(out), span);
                    }
                    let tmp = self.fresh_name();
                    let mut out = vec![self.mk_stmt(StmtKind::Let { mutable: false, name: tmp, name_span: tspan, ty: None, init: Some(value) }, span)];
                    for (i, &x) in elems.iter().enumerate() {
                        let xs = self.expr_span(x);
                        let t = self.mk_expr(ExprKind::Ident(tmp), xs);
                        let idx = self.mk_expr(ExprKind::Int(i as u64), xs);
                        let at = self.mk_expr(ExprKind::Index { obj: t, index: idx, optional: false }, xs);
                        let v = self.mk_expr(ExprKind::NonNull(at), xs);
                        let a = self.mk_expr(ExprKind::Assign(AssignOp::Assign, x, v), xs);
                        out.push(self.mk_stmt(StmtKind::Expr(a), xs));
                    }
                    return self.mk_stmt(StmtKind::Block(out), span);
                }
                self.mk_stmt(StmtKind::Expr(e), span)
            }
        }
    }

    /// `const x: T = e` / `let x = e` (without the terminator).
    fn let_stmt(&mut self) -> StmtId {
        let start = self.cur_span();
        let mutable = match self.kind() {
            Tok::Var => true,
            Tok::Let => true,
            _ => false,
        };
        self.bump();
        if self.at(Tok::LBrace) || self.at(Tok::LBracket) {
            // `const [a, b] = e`, `let { x, y } = e`: a hidden variable, then one per name
            let pat = self.pattern();
            let ty = if self.eat(Tok::Colon) { Some(self.ty()) } else { None };
            if !self.at(Tok::Eq) {
                let span = self.cur_span();
                self.err("P0001", span, "a destructuring declaration needs a value: `const [a, b] = e`");
                return self.mk_stmt(StmtKind::Error, start);
            }
            self.bump();
            let init = self.assign();
            let mut out = Vec::new();
            self.destructure(&pat, mutable, ty, init, &mut out);
            let first = out.remove(0);
            self.more_stmts.extend(out);
            return first;
        }
        let Some((name, name_span)) = self.ident("a variable name") else {
            self.recover_line();
            return self.mk_stmt(StmtKind::Error, start);
        };
        let ty = if self.eat(Tok::Colon) { Some(self.ty()) } else { None };
        let init = if self.eat(Tok::Eq) { Some(self.assign()) } else { None };
        let span = start.to(self.prev_span());
        let first = self.mk_stmt(StmtKind::Let { mutable, name, name_span, ty, init }, span);
        // `let a = 1, b = 2`: one statement each
        while self.eat(Tok::Comma) {
            let s = self.cur_span();
            let Some((name, name_span)) = self.ident("a variable name") else {
                self.recover_line();
                break;
            };
            let ty = if self.eat(Tok::Colon) { Some(self.ty()) } else { None };
            let init = if self.eat(Tok::Eq) { Some(self.assign()) } else { None };
            let span = s.to(self.prev_span());
            let st = self.mk_stmt(StmtKind::Let { mutable, name, name_span, ty, init }, span);
            self.more_stmts.push(st);
        }
        first
    }

    /// A hidden variable for destructuring (`__d1`, ...).
    fn fresh_name(&mut self) -> Sym {
        self.hidden += 1;
        let n = format!("__d{}", self.hidden);
        self.interner.intern(&n)
    }

    /// A destructuring pattern: `[a, , b = 1, ...rest]`, `{ x, y: py, z = 0, inner: { w } }`.
    fn pattern(&mut self) -> Pat {
        let start = self.cur_span();
        if self.eat(Tok::LBracket) {
            let mut elems = Vec::new();
            let mut rest = None;
            while !self.at(Tok::RBracket) && !self.at(Tok::Eof) {
                if self.at(Tok::Comma) {
                    self.bump();
                    elems.push(None);
                    continue;
                }
                if self.eat(Tok::DotDotDot) {
                    rest = Some(Box::new(self.pattern()));
                    break;
                }
                let p = self.pattern();
                let d = if self.eat(Tok::Eq) { Some(self.assign()) } else { None };
                elems.push(Some((p, d)));
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
            self.expect(Tok::RBracket, "to close the pattern");
            return Pat::Array(elems, rest, start);
        }
        if self.eat(Tok::LBrace) {
            let mut props = Vec::new();
            let mut rest = None;
            while !self.at(Tok::RBrace) && !self.at(Tok::Eof) {
                if self.eat(Tok::DotDotDot) {
                    rest = Some(Box::new(self.pattern()));
                    break;
                }
                let Some((key, kspan)) = self.ident("a field name") else { break };
                let p = if self.eat(Tok::Colon) { self.pattern() } else { Pat::Name(key, kspan) };
                let d = if self.eat(Tok::Eq) { Some(self.assign()) } else { None };
                props.push((key, kspan, p, d));
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
            self.expect(Tok::RBrace, "to close the pattern");
            return Pat::Object(props, rest, start);
        }
        match self.ident("a variable name") {
            Some((n, s)) => Pat::Name(n, s),
            None => Pat::Name(self.fresh_name(), start),
        }
    }

    /// `let <pat>: ty = init` as one `let` per name (the value in a hidden variable first).
    fn destructure(&mut self, pat: &Pat, mutable: bool, ty: Option<TypeId>, init: ExprId, out: &mut Vec<StmtId>) {
        let mut binds = Vec::new();
        self.bindings(pat, ty, init, &mut binds);
        for (name, name_span, ty, init, hidden) in binds {
            let span = name_span.to(self.expr_span(init));
            out.push(self.mk_stmt(StmtKind::Let { mutable: mutable && !hidden, name, name_span, ty, init: Some(init) }, span));
        }
    }

    /// The variables a pattern binds: (name, span, type, value, a hidden one).
    fn bindings(&mut self, pat: &Pat, ty: Option<TypeId>, init: ExprId, out: &mut Vec<(Sym, Span, Option<TypeId>, ExprId, bool)>) {
        let (tmp, tspan) = match pat {
            Pat::Name(n, s) => {
                out.push((*n, *s, ty, init, false));
                return;
            }
            Pat::Array(_, _, s) | Pat::Object(_, _, s) => (self.fresh_name(), *s),
        };
        out.push((tmp, tspan, ty, init, true));
        let read = |p: &mut Self| p.mk_expr(ExprKind::Ident(tmp), tspan);
        match pat {
            Pat::Array(elems, rest, _) => {
                for (i, e) in elems.iter().enumerate() {
                    let Some((sub, d)) = e else { continue };
                    let span = sub.span();
                    let t = read(self);
                    let idx = self.mk_expr(ExprKind::Int(i as u64), span);
                    let at = self.mk_expr(ExprKind::Index { obj: t, index: idx, optional: false }, span);
                    let v = match d {
                        Some(d) => self.defaulted(at, *d, span),
                        None => self.mk_expr(ExprKind::NonNull(at), span),
                    };
                    self.bindings(sub, None, v, out);
                }
                if let Some(r) = rest {
                    let span = r.span();
                    let t = read(self);
                    let slice = self.interner.intern("slice");
                    let callee = self.mk_expr(ExprKind::Member { obj: t, name: slice, name_span: span, optional: false }, span);
                    let from = self.mk_expr(ExprKind::Int(elems.len() as u64), span);
                    let v = self.mk_expr(ExprKind::Call { callee, type_args: Vec::new(), args: vec![Arg { expr: from, by_ref: None }], optional: false }, span);
                    self.bindings(r, None, v, out);
                }
            }
            Pat::Object(props, rest, _) => {
                for (key, kspan, sub, d) in props {
                    let t = read(self);
                    let at = self.mk_expr(ExprKind::Member { obj: t, name: *key, name_span: *kspan, optional: false }, *kspan);
                    let v = match d {
                        Some(d) => self.defaulted(at, *d, *kspan),
                        None => at,
                    };
                    self.bindings(sub, None, v, out);
                }
                // `...others`: `__rest(tmp, "name", ...)`, the other fields (see the checker)
                if let Some(r) = rest {
                    let span = r.span();
                    let mut args = vec![Arg { expr: read(self), by_ref: None }];
                    for (key, kspan, _, _) in props {
                        args.push(Arg { expr: self.mk_expr(ExprKind::Str(*key), *kspan), by_ref: None });
                    }
                    let name = self.interner.intern("__rest");
                    let callee = self.mk_expr(ExprKind::Ident(name), span);
                    let v = self.mk_expr(ExprKind::Call { callee, type_args: Vec::new(), args, optional: false }, span);
                    self.bindings(r, None, v, out);
                }
            }
            Pat::Name(..) => {}
        }
    }

    /// A statement where one is expected (`if (c) stmt`): declarations of several variables
    /// become a block.
    fn single_stmt(&mut self) -> StmtId {
        let s = self.stmt();
        if self.more_stmts.is_empty() {
            return s;
        }
        let mut all = vec![s];
        all.append(&mut self.more_stmts);
        let span = self.ast.stmts[s as usize].span;
        self.mk_stmt(StmtKind::Block(all), span)
    }

    fn for_stmt(&mut self) -> StmtId {
        let start = self.cur_span();
        self.bump(); // for
        let is_await = self.eat(Tok::Await);
        self.expect(Tok::LParen, "after `for`");
        // `for (const [k, v] of xs)`: each element in a hidden variable, destructured in the body
        if matches!(self.kind(), Tok::Const | Tok::Let | Tok::Var) && matches!(self.nth(1), Tok::LBracket | Tok::LBrace) && !is_await {
            let save = self.pos;
            let mutable = !self.at(Tok::Const);
            self.bump();
            let close = {
                let p = self.pos;
                self.skip_balanced();
                let c = self.pos;
                self.pos = p;
                c
            };
            if self.toks.get(close).map(|t| t.kind == Tok::Ident && self.text(*t) == "of").unwrap_or(false) {
                let pat = self.pattern();
                self.bump(); // of
                let iter = self.expr();
                self.expect(Tok::RParen, "to close the `for` header");
                let body = self.single_stmt();
                let span = start.to(self.prev_span());
                let tmp = self.fresh_name();
                let pspan = pat.span();
                let elem = self.mk_expr(ExprKind::Ident(tmp), pspan);
                let mut stmts = Vec::new();
                self.destructure(&pat, mutable, None, elem, &mut stmts);
                stmts.push(body);
                let bspan = self.ast.stmts[body as usize].span;
                let body = self.mk_stmt(StmtKind::Block(stmts), bspan);
                return self.mk_stmt(StmtKind::ForOf { mutable: false, name: tmp, name_span: pspan, iter, body }, span);
            }
            self.pos = save;
        }
        if matches!(self.kind(), Tok::Const | Tok::Let | Tok::Var) && self.nth(1) == Tok::Ident {
            let is_of = self.toks.get(self.pos + 2).map(|t| t.kind == Tok::Ident && self.text(*t) == "of").unwrap_or(false);
            let is_in = self.nth(2) == Tok::In;
            if is_of || is_in {
                let mutable = !self.at(Tok::Const);
                self.bump();
                let (name, name_span) = self.ident("a loop variable").unwrap();
                let kw = self.cur_span();
                self.bump(); // of / in
                let iter = self.expr();
                // `for (const k in obj)`: its keys, `Object.keys(obj)`
                let iter = if is_in {
                    let (object, keys) = (self.interner.intern("Object"), self.interner.intern("keys"));
                    let isp = self.expr_span(iter);
                    let o = self.mk_expr(ExprKind::Ident(object), kw);
                    let callee = self.mk_expr(ExprKind::Member { obj: o, name: keys, name_span: kw, optional: false }, kw);
                    self.mk_expr(ExprKind::Call { callee, type_args: Vec::new(), args: vec![Arg { expr: iter, by_ref: None }], optional: false }, isp)
                } else {
                    iter
                };
                self.expect(Tok::RParen, "to close the `for` header");
                let body = self.single_stmt();
                let span = start.to(self.prev_span());
                if is_await {
                    return self.for_await(mutable, name, name_span, iter, body, span);
                }
                return self.mk_stmt(StmtKind::ForOf { mutable, name, name_span, iter, body }, span);
            }
        }
        if is_await {
            let span = self.cur_span();
            self.err("P0008", span, "`for await` needs `(const x of stream)`");
        }
        let init = if self.at(Tok::Semi) {
            None
        } else if matches!(self.kind(), Tok::Const | Tok::Let | Tok::Var) {
            Some(self.let_stmt())
        } else {
            let e = self.expr();
            let span = self.expr_span(e);
            Some(self.mk_stmt(StmtKind::Expr(e), span))
        };
        // `for (let i = 0, j = n - 1; ...)`: the declarations before the loop, in a block of its own
        let more = std::mem::take(&mut self.more_stmts);
        self.expect(Tok::Semi, "after the `for` initializer");
        let cond = if self.at(Tok::Semi) { None } else { Some(self.expr()) };
        self.expect(Tok::Semi, "after the `for` condition");
        let step = if self.at(Tok::RParen) {
            None
        } else {
            let first = self.expr();
            let mut steps = vec![first];
            while self.eat(Tok::Comma) {
                steps.push(self.expr());
            }
            if steps.len() == 1 {
                Some(first)
            } else {
                let span = self.expr_span(first).to(self.prev_span());
                Some(self.mk_expr(ExprKind::Seq(steps), span))
            }
        };
        self.expect(Tok::RParen, "to close the `for` header");
        let body = self.single_stmt();
        let span = start.to(self.prev_span());
        if more.is_empty() {
            return self.mk_stmt(StmtKind::For { init, cond, step, body }, span);
        }
        let mut all: Vec<StmtId> = init.into_iter().collect();
        all.extend(more);
        all.push(self.mk_stmt(StmtKind::For { init: None, cond, step, body }, span));
        self.mk_stmt(StmtKind::Block(all), span)
    }

    /// `for await (const x of stream) body` reads a ReadableStream (`res.body`) chunk by chunk:
    ///
    /// ```text
    /// { const it = try stream.getReader(); let done = false
    ///   while (true) { const r = try await it.read(); if (r.done) { done = true; break }
    ///                  const x = r.value!; body }
    ///   if (!done) await it.cancel()   // left early (`break`): the rest isn't wanted
    ///   it.releaseLock() }
    /// ```
    fn for_await(&mut self, mutable: bool, name: Sym, name_span: Span, iter: ExprId, body: StmtId, span: Span) -> StmtId {
        let sp = self.expr_span(iter);
        let at = span.start;
        // Locals are keyed by where their name starts: each hidden one gets its own offset inside
        // the `for` keyword, where no written declaration can start.
        let hidden = |k: u32| Span::new(span.file, span.start + k, span.start + k + 1);
        let it = self.interner.intern(&format!("for_await_reader_{at}"));
        let res = self.interner.intern(&format!("for_await_result_{at}"));
        let done = self.interner.intern(&format!("for_await_done_{at}"));
        let method = |p: &mut Self, obj: ExprId, m: &str| -> ExprId {
            let name = p.interner.intern(m);
            let member = p.mk_expr(ExprKind::Member { obj, name, name_span: sp, optional: false }, sp);
            p.mk_expr(ExprKind::Call { callee: member, type_args: Vec::new(), args: Vec::new(), optional: false }, sp)
        };
        let field = |p: &mut Self, obj: ExprId, f: &str| -> ExprId {
            let name = p.interner.intern(f);
            p.mk_expr(ExprKind::Member { obj, name, name_span: sp, optional: false }, sp)
        };
        let ident = |p: &mut Self, s: Sym| p.mk_expr(ExprKind::Ident(s), sp);
        // const it = try stream.getReader()
        let get = method(self, iter, "getReader");
        let get = self.mk_expr(ExprKind::Try(get), sp);
        let let_it = self.mk_stmt(StmtKind::Let { mutable: false, name: it, name_span: hidden(0), ty: None, init: Some(get) }, sp);
        // let done = false
        let f = self.mk_expr(ExprKind::Bool(false), sp);
        let let_done = self.mk_stmt(StmtKind::Let { mutable: true, name: done, name_span: hidden(1), ty: None, init: Some(f) }, sp);
        // const r = try await it.read()
        let it_e = ident(self, it);
        let read = method(self, it_e, "read");
        let read = self.mk_expr(ExprKind::Await(read), sp);
        let read = self.mk_expr(ExprKind::Try(read), sp);
        let let_r = self.mk_stmt(StmtKind::Let { mutable: false, name: res, name_span: hidden(2), ty: None, init: Some(read) }, sp);
        // if (r.done) { done = true; break }
        let r_e = ident(self, res);
        let cond = field(self, r_e, "done");
        let d_e = ident(self, done);
        let t = self.mk_expr(ExprKind::Bool(true), sp);
        let set = self.mk_expr(ExprKind::Assign(AssignOp::Assign, d_e, t), sp);
        let set = self.mk_stmt(StmtKind::Expr(set), sp);
        let brk = self.mk_stmt(StmtKind::Break(None), sp);
        let then = self.mk_stmt(StmtKind::Block(vec![set, brk]), sp);
        let if_done = self.mk_stmt(StmtKind::If(cond, then, None), sp);
        // const x = r.value!
        let r_e = ident(self, res);
        let value = field(self, r_e, "value");
        let value = self.mk_expr(ExprKind::NonNull(value), sp);
        let let_x = self.mk_stmt(StmtKind::Let { mutable, name, name_span, ty: None, init: Some(value) }, name_span);
        let loop_body = self.mk_stmt(StmtKind::Block(vec![let_r, if_done, let_x, body]), span);
        let t = self.mk_expr(ExprKind::Bool(true), sp);
        let wh = self.mk_stmt(StmtKind::While(t, loop_body), span);
        // if (!done) await it.cancel()
        let d_e = ident(self, done);
        let not_done = self.mk_expr(ExprKind::Unary(UnOp::Not, d_e), sp);
        let it_e = ident(self, it);
        let cancel = method(self, it_e, "cancel");
        let cancel = self.mk_expr(ExprKind::Await(cancel), sp);
        let cancel = self.mk_stmt(StmtKind::Expr(cancel), sp);
        let if_cancel = self.mk_stmt(StmtKind::If(not_done, cancel, None), sp);
        // it.releaseLock()
        let it_e = ident(self, it);
        let release = method(self, it_e, "releaseLock");
        let release = self.mk_stmt(StmtKind::Expr(release), sp);
        self.mk_stmt(StmtKind::Block(vec![let_it, let_done, wh, if_cancel, release]), span)
    }

    fn switch_stmt(&mut self) -> StmtId {
        let start = self.cur_span();
        self.bump(); // switch
        self.expect(Tok::LParen, "after `switch`");
        let disc = self.expr();
        self.expect(Tok::RParen, "to close the `switch` value");
        self.expect(Tok::LBrace, "to start the `switch` body");
        let mut cases = Vec::new();
        while !self.at(Tok::RBrace) && !self.at(Tok::Eof) {
            let case_start = self.cur_span();
            let test = if self.eat(Tok::Case) {
                Some(self.expr())
            } else if self.eat(Tok::Default) {
                None
            } else {
                let msg = format!("expected `case` or `default`, found {}", self.found());
                self.err("P0001", case_start, msg);
                self.recover_line();
                continue;
            };
            self.expect(Tok::Colon, "after the case label");
            let mut body = Vec::new();
            while !matches!(self.kind(), Tok::Case | Tok::Default | Tok::RBrace | Tok::Eof) {
                let before = self.pos;
                body.push(self.stmt());
                body.append(&mut self.more_stmts);
                if self.pos == before {
                    self.bump();
                }
            }
            let span = case_start.to(self.prev_span());
            cases.push(Case { test, body, span });
        }
        self.expect(Tok::RBrace, "to close the `switch` body");
        let span = start.to(self.prev_span());
        self.mk_stmt(StmtKind::Switch(disc, cases), span)
    }

    // ---------------------------------------------------------------- expressions

    fn expr(&mut self) -> ExprId {
        self.assign()
    }

    fn assign_op(&self) -> Option<(AssignOp, usize)> {
        use BinOp::*;
        let op = match self.kind() {
            Tok::Eq => AssignOp::Assign,
            Tok::PlusEq => AssignOp::Op(Add),
            Tok::MinusEq => AssignOp::Op(Sub),
            Tok::StarEq => AssignOp::Op(Mul),
            Tok::SlashEq => AssignOp::Op(Div),
            Tok::PercentEq => AssignOp::Op(Rem),
            Tok::StarStarEq => AssignOp::Op(Pow),
            Tok::AmpEq => AssignOp::Op(BitAnd),
            Tok::PipeEq => AssignOp::Op(BitOr),
            Tok::CaretEq => AssignOp::Op(BitXor),
            Tok::LtLtEq => AssignOp::Op(Shl),
            Tok::QuestionQuestionEq => AssignOp::Op(Nullish),
            Tok::AmpAmpEq => AssignOp::Op(And),
            Tok::PipePipeEq => AssignOp::Op(Or),
            Tok::Gt => {
                // `>>=` and `>>>=` arrive as separate adjacent tokens.
                let t = &self.toks[self.pos..];
                let glued = |i: usize| t.get(i).zip(t.get(i + 1)).map(|(a, b)| a.end == b.start).unwrap_or(false);
                if t.len() > 3 && t[1].kind == Tok::Gt && t[2].kind == Tok::Gt && t[3].kind == Tok::Eq && glued(0) && glued(1) && glued(2) {
                    return Some((AssignOp::Op(UShr), 4));
                }
                if t.len() > 2 && t[1].kind == Tok::Gt && t[2].kind == Tok::Eq && glued(0) && glued(1) {
                    return Some((AssignOp::Op(Shr), 3));
                }
                return None;
            }
            _ => return None,
        };
        Some((op, 1))
    }

    fn assign(&mut self) -> ExprId {
        // `yield x`, `yield* xs`, `yield` (where nothing that could start a value follows)
        if self.at(Tok::Yield) {
            let start = self.cur_span();
            let t = self.bump();
            let delegate = self.eat(Tok::Star);
            let ends = matches!(self.kind(), Tok::RParen | Tok::RBracket | Tok::RBrace | Tok::Semi | Tok::Comma | Tok::Colon | Tok::Eof) || (self.tok().nl_before && !delegate);
            let _ = t;
            let value = if ends { None } else { Some(self.assign()) };
            let span = start.to(self.prev_span());
            return self.mk_expr(ExprKind::Yield(value, delegate), span);
        }
        if let Some(e) = self.try_arrow() {
            return e;
        }
        let lhs = self.cond_expr();
        if let Some((op, n)) = self.assign_op() {
            for _ in 0..n {
                self.bump();
            }
            let rhs = self.assign();
            let span = self.expr_span(lhs).to(self.expr_span(rhs));
            return self.mk_expr(ExprKind::Assign(op, lhs, rhs), span);
        }
        lhs
    }

    fn cond_expr(&mut self) -> ExprId {
        let cond = self.binary(1);
        if !self.at(Tok::Question) {
            return cond;
        }
        self.bump();
        let a = self.assign();
        self.expect(Tok::Colon, "in the conditional expression");
        let b = self.assign();
        let span = self.expr_span(cond).to(self.expr_span(b));
        self.mk_expr(ExprKind::Cond(cond, a, b), span)
    }

    /// Returns (operator, tokens to consume, precedence). `as` is reported as `None` op.
    fn peek_binop(&self) -> Option<(Option<BinOp>, usize, u8)> {
        use BinOp::*;
        let t = &self.toks[self.pos..];
        let glued = |i: usize| t.get(i).zip(t.get(i + 1)).map(|(a, b)| a.end == b.start).unwrap_or(false);
        let (op, n) = match self.kind() {
            Tok::PipePipe => (Or, 1),
            Tok::QuestionQuestion => (Nullish, 1),
            Tok::AmpAmp => (And, 1),
            Tok::Pipe => (BitOr, 1),
            Tok::Caret => (BitXor, 1),
            Tok::Amp => (BitAnd, 1),
            Tok::EqEq => (LooseEq, 1),
            Tok::BangEq => (LooseNe, 1),
            Tok::EqEqEq => (Eq, 1),
            Tok::BangEqEq => (Ne, 1),
            Tok::Lt => (Lt, 1),
            Tok::LtEq => (Le, 1),
            Tok::Gt => {
                if t.len() > 2 && t[1].kind == Tok::Gt && t[2].kind == Tok::Gt && glued(0) && glued(1) {
                    if t.get(3).map(|x| x.kind) == Some(Tok::Eq) && glued(2) {
                        return None; // `>>>=`
                    }
                    (UShr, 3)
                } else if t.len() > 1 && t[1].kind == Tok::Gt && glued(0) {
                    if t.get(2).map(|x| x.kind) == Some(Tok::Eq) && glued(1) {
                        return None; // `>>=`
                    }
                    (Shr, 2)
                } else if t.len() > 1 && t[1].kind == Tok::Eq && glued(0) {
                    (Ge, 2)
                } else {
                    (Gt, 1)
                }
            }
            Tok::LtLt => (Shl, 1),
            Tok::Plus => (Add, 1),
            Tok::Minus => (Sub, 1),
            Tok::Star => (Mul, 1),
            Tok::Slash => (Div, 1),
            Tok::Percent => (Rem, 1),
            Tok::StarStar => (Pow, 1),
            Tok::In => (In, 1),
            Tok::Instanceof => (Instanceof, 1),
            Tok::Ident if self.text(self.tok()) == "as" => return Some((None, 1, 8)),
            Tok::Ident if self.text(self.tok()) == "satisfies" => return Some((None, 1, 8)),
            _ => return None,
        };
        let prec = match op {
            Or | Nullish => 1,
            And => 2,
            BitOr => 3,
            BitXor => 4,
            BitAnd => 5,
            LooseEq | LooseNe | Eq | Ne => 6,
            Lt | Gt | Le | Ge | In | Instanceof => 7,
            Shl | Shr | UShr => 8,
            Add | Sub => 9,
            Mul | Div | Rem => 10,
            Pow => 11,
        };
        // `as` is relational-level in TS; keep it just above relational ops.
        let prec = if prec >= 8 { prec + 1 } else { prec };
        Some((Some(op), n, prec))
    }

    fn binary(&mut self, min_prec: u8) -> ExprId {
        let mut lhs = self.unary();
        while let Some((op, n, prec)) = self.peek_binop() {
            if prec < min_prec {
                break;
            }
            let _op_span = self.cur_span();
            let is_satisfies = op.is_none() && self.text(self.tok()) == "satisfies";
            for _ in 0..n {
                self.bump();
            }
            let Some(op) = op else {
                // `x as const`: x (its literals keep their types)
                if !is_satisfies && self.at(Tok::Const) {
                    let const_span = self.cur_span();
                    self.bump();
                    let span = self.expr_span(lhs).to(self.prev_span());
                    // (of an array literal, a tuple: `[k, v] as const`; of an object literal,
                    // its strings keep their literal types)
                    if matches!(self.ast.expr(lhs).kind, ExprKind::Array(_) | ExprKind::Object(_)) {
                        let name = self.interner.intern("const");
                        let ty = self.mk_ty(TypeExprKind::Named { ns: None, name, name_span: const_span, args: Vec::new() }, const_span);
                        lhs = self.mk_expr(ExprKind::As(lhs, ty), span);
                        continue;
                    }
                    lhs = self.mk_expr(ExprKind::Paren(lhs), span);
                    continue;
                }
                let ty = self.ty();
                let span = self.expr_span(lhs).to(self.prev_span());
                // (`x satisfies T`: `x`, checked to be a `T` and typed with it in mind, its own
                // type kept)
                if is_satisfies {
                    lhs = self.mk_expr(ExprKind::Paren(lhs), span);
                    self.ast.satisfies.insert(lhs, ty);
                    continue;
                }
                lhs = self.mk_expr(ExprKind::As(lhs, ty), span);
                continue;
            };
            let rhs = if op == BinOp::Pow { self.binary(prec) } else { self.binary(prec + 1) };
            let span = self.expr_span(lhs).to(self.expr_span(rhs));
            lhs = self.mk_expr(ExprKind::Binary(op, lhs, rhs), span);
        }
        lhs
    }

    fn unary(&mut self) -> ExprId {
        let start = self.cur_span();
        let op = match self.kind() {
            Tok::Bang => UnOp::Not,
            Tok::Minus => UnOp::Neg,
            Tok::Plus => UnOp::Plus,
            Tok::Tilde => UnOp::BitNot,
            Tok::Void => UnOp::Void,
            Tok::Typeof => {
                self.bump();
                let e = self.unary();
                let span = start.to(self.expr_span(e));
                return self.mk_expr(ExprKind::Typeof(e), span);
            }
            Tok::Delete => {
                self.bump();
                self.push(
                    Diagnostic::new("X0008", start, "`delete` is not supported")
                        .note("instead", "records have fixed fields; use `Map` and `map.delete(key)` for dynamic keys"),
                );
                return self.unary();
            }
            Tok::Await => {
                self.bump();
                let e = self.unary();
                let span = start.to(self.expr_span(e));
                return self.mk_expr(ExprKind::Await(e), span);
            }
            Tok::Try => {
                self.bump();
                let e = self.unary();
                let span = start.to(self.expr_span(e));
                return self.mk_expr(ExprKind::Try(e), span);
            }
            Tok::PlusPlus | Tok::MinusMinus => {
                let inc = self.at(Tok::PlusPlus);
                self.bump();
                let target = self.unary();
                let span = start.to(self.expr_span(target));
                return self.mk_expr(ExprKind::Update { inc, prefix: true, target }, span);
            }
            Tok::Lt => {
                self.push(Diagnostic::new("X0019", start, "angle-bracket type assertions are not supported").note("instead", "use `value as T`"));
                self.bump();
                self.ty();
                self.expect(Tok::Gt, "to close the type assertion");
                return self.unary();
            }
            _ => return self.postfix(),
        };
        self.bump();
        let e = self.unary();
        let span = start.to(self.expr_span(e));
        self.mk_expr(ExprKind::Unary(op, e), span)
    }

    /// A name and its fields (`cfg.server`), for `typeof` in a type.
    fn postfix_expr_no_call(&mut self) -> ExprId {
        let start = self.cur_span();
        let mut e = self.primary();
        while self.at(Tok::Dot) {
            self.bump();
            let Some((name, name_span)) = self.prop_name() else { break };
            let span = start.to(self.prev_span());
            e = self.mk_expr(ExprKind::Member { obj: e, name, name_span, optional: false }, span);
        }
        e
    }

    fn postfix(&mut self) -> ExprId {
        let mut e = self.primary();
        // (after `a?.b`, the rest of the chain is skipped too when `a` is absent: `a?.b.c(x)`
        // reads as `a?.b?.c(x)`, which is the same wherever TypeScript allows the first)
        let mut chain = false;
        loop {
            let start = self.expr_span(e);
            match self.kind() {
                Tok::Dot => {
                    self.bump();
                    let hashed = if self.at(Tok::Hash) {
                        let h = self.bump();
                        if self.at(Tok::Ident) && self.adjacent() {
                            let t = self.bump();
                            let sym = self.interner.intern(&format!("#{}", self.text(t)));
                            Some((sym, self.span_of(h).to(self.span_of(t))))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let Some((name, name_span)) = hashed.or_else(|| self.prop_name()) else {
                        let msg = format!("expected a property name after `.`, found {}", self.found());
                        let span = self.cur_span();
                        self.err("P0001", span, msg);
                        return e;
                    };
                    let span = start.to(name_span);
                    e = self.mk_expr(ExprKind::Member { obj: e, name, name_span, optional: chain }, span);
                }
                Tok::QuestionDot => {
                    self.bump();
                    chain = true;
                    if self.at(Tok::LParen) {
                        let args = self.args();
                        let span = start.to(self.prev_span());
                        e = self.mk_expr(ExprKind::Call { callee: e, type_args: Vec::new(), args, optional: true }, span);
                    } else if self.eat(Tok::LBracket) {
                        let index = self.expr();
                        self.expect(Tok::RBracket, "to close the index");
                        let span = start.to(self.prev_span());
                        e = self.mk_expr(ExprKind::Index { obj: e, index, optional: true }, span);
                    } else if let Some((name, name_span)) = self.prop_name() {
                        let span = start.to(name_span);
                        e = self.mk_expr(ExprKind::Member { obj: e, name, name_span, optional: true }, span);
                    } else {
                        let msg = format!("expected a property name after `?.`, found {}", self.found());
                        let span = self.cur_span();
                        self.err("P0001", span, msg);
                        return e;
                    }
                }
                Tok::LParen if chain && !self.tok().nl_before && !matches!(self.ast.exprs[e as usize].kind, ExprKind::Member { .. }) => {
                    // (`f?.()()`: the result is called only if there is one)
                    let args = self.args();
                    let span = start.to(self.prev_span());
                    e = self.mk_expr(ExprKind::Call { callee: e, type_args: Vec::new(), args, optional: true }, span);
                }
                Tok::LParen if !self.tok().nl_before => {
                    let args = self.args();
                    let span = start.to(self.prev_span());
                    e = self.call_or_new(e, Vec::new(), args, span);
                }
                Tok::LBracket if !self.tok().nl_before => {
                    self.bump();
                    let index = self.expr();
                    self.expect(Tok::RBracket, "to close the index");
                    let span = start.to(self.prev_span());
                    e = self.mk_expr(ExprKind::Index { obj: e, index, optional: chain }, span);
                }
                Tok::Bang if !self.tok().nl_before => {
                    self.bump();
                    let span = start.to(self.prev_span());
                    e = self.mk_expr(ExprKind::NonNull(e), span);
                }
                Tok::PlusPlus | Tok::MinusMinus if !self.tok().nl_before => {
                    let inc = self.at(Tok::PlusPlus);
                    self.bump();
                    let span = start.to(self.prev_span());
                    e = self.mk_expr(ExprKind::Update { inc, prefix: false, target: e }, span);
                }
                Tok::TemplateFull | Tok::TemplateHead if !self.tok().nl_before => {
                    let span = self.cur_span();
                    self.err("X0022", span, "tagged template literals are not supported");
                    self.primary();
                }
                Tok::Lt if self.adjacent() && matches!(self.ast.exprs[e as usize].kind, ExprKind::Ident(_) | ExprKind::Member { .. }) => {
                    // `f<int>(x)`: explicit type arguments, only when followed by a call.
                    let targs = self.speculate(|p| {
                        p.bump();
                        let mut out = Vec::new();
                        while !p.at(Tok::Gt) && !p.at(Tok::Eof) {
                            out.push(p.ty());
                            if !p.eat(Tok::Comma) {
                                break;
                            }
                        }
                        if !p.eat(Tok::Gt) || !p.at(Tok::LParen) {
                            return None;
                        }
                        Some(out)
                    });
                    let Some(type_args) = targs else { return e };
                    let args = self.args();
                    let span = start.to(self.prev_span());
                    e = self.call_or_new(e, type_args, args, span);
                }
                _ => return e,
            }
        }
    }

    fn args(&mut self) -> Vec<Arg> {
        self.bump(); // (
        let mut out = Vec::new();
        while !self.at(Tok::RParen) && !self.at(Tok::Eof) {
            let spread = self.at(Tok::DotDotDot).then(|| self.bump()).map(|t| self.span_of(t));
            let by_ref = if self.at(Tok::Amp) {
                let span = self.cur_span();
                self.bump();
                Some(span)
            } else {
                None
            };
            let expr = self.assign();
            let expr = match spread {
                Some(s) => {
                    let span = s.to(self.expr_span(expr));
                    self.mk_expr(ExprKind::Spread(expr), span)
                }
                None => expr,
            };
            out.push(Arg { expr, by_ref });
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen, "to close the argument list");
        out
    }

    fn try_arrow(&mut self) -> Option<ExprId> {
        let start = self.cur_span();
        if self.at(Tok::Async) && matches!(self.nth(1), Tok::LParen | Tok::Ident) {
            let save = self.pos;
            self.bump();
            let r = self.try_arrow();
            match r {
                Some(e) => {
                    if let ExprKind::Arrow(f) = &mut self.ast.exprs[e as usize].kind {
                        f.is_async = true;
                    }
                }
                None => self.pos = save,
            }
            return r;
        }
        if self.at(Tok::Ident) && self.nth(1) == Tok::Arrow {
            let t = self.bump();
            let name = self.interner.intern(self.text(t));
            let params = vec![Param { name, span: self.span_of(t), ty: None, inout: false, optional: false, prop: None, rest: false }];
            self.bump(); // =>
            return Some(self.arrow_body(start, params, None, None));
        }
        // (`<T>(`: the type parameters, read once the `(` shows it's an arrow)
        let mut tps = Vec::new();
        if self.at(Tok::Lt)
            && let Some(t) = self.speculate(|p| {
                let t = p.tparams();
                if p.at(Tok::LParen) { Some(t) } else { None }
            })
        {
            tps = t;
        }
        if !self.at(Tok::LParen) {
            return None;
        }
        if !tps.is_empty() {
            let r = self.arrow_after_parens(start);
            if let Some(e) = r {
                self.generic_arrows.push((e, tps, start));
            }
            return r;
        }
        self.arrow_after_parens(start)
    }

    /// An arrow function from its `(`: `(params) => body`, `(params): T => body`.
    fn arrow_after_parens(&mut self, start: Span) -> Option<ExprId> {
        let close = self.matching_paren()?;
        let after = self.toks.get(close + 1).map(|t| t.kind);
        match after {
            Some(Tok::Arrow) => {
                let params = self.params();
                self.bump(); // =>
                Some(self.arrow_body(start, params, None, None))
            }
            Some(Tok::Colon) => self.speculate(|p| {
                let params = p.params();
                if !p.eat(Tok::Colon) {
                    return None;
                }
                let ret = p.ty();
                let throws = p.effects_clause();
                if !p.eat(Tok::Arrow) {
                    return None;
                }
                Some(p.arrow_body(start, params, Some(ret), throws))
            }),
            _ => None,
        }
    }

    fn arrow_body(&mut self, start: Span, params: Vec<Param>, ret: Option<TypeId>, throws: Option<TypeId>) -> ExprId {
        let defaults = (std::mem::take(&mut self.defaults), std::mem::take(&mut self.prologue));
        let body = if self.at(Tok::LBrace) { ArrowBody::Block(self.block()) } else { ArrowBody::Expr(self.assign()) };
        let body = if defaults.0.is_empty() && defaults.1.is_empty() {
            body
        } else {
            let block = match body {
                ArrowBody::Block(b) => b,
                ArrowBody::Expr(x) => {
                    let sp = self.expr_span(x);
                    let r = self.mk_stmt(StmtKind::Return(Some(x)), sp);
                    self.mk_stmt(StmtKind::Block(vec![r]), sp)
                }
            };
            ArrowBody::Block(self.with_defaults(defaults, block))
        };
        let span = start.to(self.prev_span());
        self.mk_expr(ExprKind::Arrow(Box::new(ArrowFn { params, ret, throws, body, is_async: false })), span)
    }

    fn int_value(&mut self, t: Token) -> u64 {
        let text: String = self.text(t).chars().filter(|&c| c != '_').collect();
        let (digits, radix) = match text.get(..2) {
            Some("0x") | Some("0X") => (&text[2..], 16),
            Some("0b") | Some("0B") => (&text[2..], 2),
            Some("0o") | Some("0O") => (&text[2..], 8),
            _ => (&text[..], 10),
        };
        match u64::from_str_radix(digits, radix) {
            Ok(v) => v,
            Err(_) => {
                let span = self.span_of(t);
                self.err("L0005", span, "integer literal is too large for 64 bits");
                0
            }
        }
    }

    fn primary(&mut self) -> ExprId {
        let start = self.cur_span();
        match self.kind() {
            Tok::Int => {
                let t = self.bump();
                let v = self.int_value(t);
                self.mk_expr(ExprKind::Int(v), start)
            }
            Tok::Decimal => {
                let t = self.bump();
                let text: String = self.text(t).chars().filter(|&c| c != '_' && c != 'm').collect();
                let s = self.interner.intern(&text);
                self.mk_expr(ExprKind::Decimal(s), start)
            }
            Tok::BigInt => {
                let t = self.bump();
                let text: String = self.text(t).chars().filter(|&c| c != '_' && c != 'n').collect();
                let s = self.interner.intern(&text);
                self.mk_expr(ExprKind::BigInt(s), start)
            }
            Tok::Float => {
                let t = self.bump();
                let text: String = self.text(t).chars().filter(|&c| c != '_').collect();
                let v = text.parse::<f64>().unwrap_or(0.0);
                self.mk_expr(ExprKind::Float(v), start)
            }
            Tok::Str => {
                let t = self.bump();
                let s = self.interner.intern(&self.strings[t.val as usize]);
                self.mk_expr(ExprKind::Str(s), start)
            }
            Tok::TemplateFull => {
                let t = self.bump();
                let s = self.strings[t.val as usize].clone();
                self.mk_expr(ExprKind::Template(vec![s], Vec::new()), start)
            }
            Tok::TemplateHead => {
                let t = self.bump();
                let mut parts = vec![self.strings[t.val as usize].clone()];
                let mut exprs = Vec::new();
                loop {
                    exprs.push(self.expr());
                    match self.kind() {
                        Tok::TemplateMiddle => {
                            let t = self.bump();
                            parts.push(self.strings[t.val as usize].clone());
                        }
                        Tok::TemplateTail => {
                            let t = self.bump();
                            parts.push(self.strings[t.val as usize].clone());
                            break;
                        }
                        _ => {
                            let msg = format!("expected `}}` to close the template substitution, found {}", self.found());
                            let span = self.cur_span();
                            self.err("P0001", span, msg);
                            parts.push(String::new());
                            break;
                        }
                    }
                }
                let span = start.to(self.prev_span());
                self.mk_expr(ExprKind::Template(parts, exprs), span)
            }
            Tok::True | Tok::False => {
                let v = self.at(Tok::True);
                self.bump();
                self.mk_expr(ExprKind::Bool(v), start)
            }
            Tok::Undefined => {
                self.bump();
                self.mk_expr(ExprKind::Undefined, start)
            }
            Tok::Null => {
                self.bump();
                self.mk_expr(ExprKind::Null, start)
            }
            Tok::Ident => {
                let t = self.bump();
                let sym = self.interner.intern(self.text(t));
                self.mk_expr(ExprKind::Ident(sym), start)
            }
            Tok::LParen => {
                self.bump();
                let e = self.expr();
                // `(a, b)`: the comma operator, b's value after a
                let e = if self.at(Tok::Comma) {
                    let mut es = vec![e];
                    while self.eat(Tok::Comma) {
                        es.push(self.expr());
                    }
                    let span = self.expr_span(es[0]).to(self.prev_span());
                    self.mk_expr(ExprKind::Seq(es), span)
                } else {
                    e
                };
                self.expect(Tok::RParen, "to close the parenthesized expression");
                let span = start.to(self.prev_span());
                self.mk_expr(ExprKind::Paren(e), span)
            }
            Tok::LBracket => {
                self.bump();
                let mut elems = Vec::new();
                while !self.at(Tok::RBracket) && !self.at(Tok::Eof) {
                    if self.at(Tok::Comma) {
                        let span = self.cur_span();
                        self.err("P0007", span, "array holes are not supported");
                        self.bump();
                        continue;
                    }
                    if self.at(Tok::DotDotDot) {
                        let s = self.cur_span();
                        self.bump();
                        let x = self.assign();
                        let span = s.to(self.expr_span(x));
                        elems.push(self.mk_expr(ExprKind::Spread(x), span));
                    } else {
                        elems.push(self.assign());
                    }
                    if !self.eat(Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::RBracket, "to close the array literal");
                let span = start.to(self.prev_span());
                self.mk_expr(ExprKind::Array(elems), span)
            }
            Tok::LBrace => self.object_literal(),
            // `async function (x) { }`: an async arrow function
            Tok::Async if self.nth(1) == Tok::Function && matches!(self.nth(2), Tok::LParen | Tok::Ident) => {
                self.bump();
                self.bump();
                let (name, name_span) = if self.at(Tok::Ident) { self.ident("a function name").unwrap() } else { (self.interner.intern(""), start) };
                let Some(f) = self.function_rest(name, name_span) else { return self.mk_expr(ExprKind::Error, start) };
                let span = start.to(self.prev_span());
                self.arrow_of(f, true, span)
            }
            // `function (x) { }`: an arrow function (`this` and `arguments` aren't supported)
            Tok::Function if matches!(self.nth(1), Tok::LParen | Tok::Ident) && !(self.nth(1) == Tok::Ident && self.nth(2) == Tok::Lt) => {
                self.bump();
                let (name, name_span) = if self.at(Tok::Ident) { self.ident("a function name").unwrap() } else { (self.interner.intern(""), start) };
                let Some(f) = self.function_rest(name, name_span) else { return self.mk_expr(ExprKind::Error, start) };
                let span = start.to(self.prev_span());
                self.arrow_of(f, false, span)
            }
            // `function* () { ... }`: a module function of its own (see `lift_functions`)
            Tok::Function if self.nth(1) == Tok::Star => {
                self.bump();
                self.bump();
                let (name, name_span) = if self.at(Tok::Ident) { self.ident("a function name").unwrap() } else { (self.interner.intern(""), start) };
                let Some(mut f) = self.function_rest(name, name_span) else { return self.mk_expr(ExprKind::Error, start) };
                f.is_gen = true;
                let span = start.to(self.prev_span());
                let hidden = self.lift(None, f, span);
                self.mk_expr(ExprKind::Ident(hidden), span)
            }
            Tok::Function => {
                self.push(
                    Diagnostic::new("U0010", start, "function expressions are not supported")
                        .note("instead", "use an arrow function: `(x: T) => { ... }`"),
                );
                self.bump();
                if self.at(Tok::Ident) {
                    self.bump();
                }
                if self.at(Tok::LParen) {
                    self.skip_balanced();
                }
                if self.eat(Tok::Colon) {
                    self.ty();
                }
                if self.at(Tok::LBrace) {
                    self.skip_balanced();
                }
                let span = start.to(self.prev_span());
                self.mk_expr(ExprKind::Error, span)
            }
            Tok::New => {
                self.bump();
                let callee = if let Some((name, span)) = self.ident("a type name after `new`") {
                    self.mk_expr(ExprKind::Ident(name), span)
                } else {
                    return self.mk_expr(ExprKind::Error, start);
                };
                let mut type_args = Vec::new();
                if self.at(Tok::Lt) {
                    self.bump();
                    while !self.at(Tok::Gt) && !self.at(Tok::Eof) {
                        type_args.push(self.ty());
                        if !self.eat(Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::Gt, "to close the type argument list");
                }
                let args = if self.at(Tok::LParen) { self.args() } else { Vec::new() };
                let span = start.to(self.prev_span());
                self.mk_expr(ExprKind::New { callee, type_args, args }, span)
            }
            Tok::This => {
                self.bump();
                self.mk_expr(ExprKind::This, start)
            }
            Tok::Super => {
                self.bump();
                self.mk_expr(ExprKind::Super, start)
            }
            Tok::Class => {
                self.err("U0001", start, "class expressions are not supported; declare the class at the top level");
                self.bump();
                if self.at(Tok::Ident) {
                    self.bump();
                }
                while !self.at(Tok::LBrace) && !self.at(Tok::Eof) {
                    self.bump();
                }
                self.skip_balanced();
                self.mk_expr(ExprKind::Error, start)
            }
            // `/re/flags`: `__reLiteral("re", "flags")`, a `new RegExp` that can't throw (an invalid
            // literal is a syntax error in JavaScript: the program doesn't run at all)
            Tok::Regex => {
                let t = self.bump();
                let (pattern, flags) = (self.strings[t.val as usize].clone(), self.strings[t.val as usize + 1].clone());
                let (p, f, r) = (self.interner.intern(&pattern), self.interner.intern(&flags), self.interner.intern("__reLiteral"));
                let callee = self.mk_expr(ExprKind::Ident(r), start);
                let pe = self.mk_expr(ExprKind::Str(p), start);
                let fe = self.mk_expr(ExprKind::Str(f), start);
                self.mk_expr(ExprKind::Call { callee, type_args: Vec::new(), args: vec![Arg { expr: pe, by_ref: None }, Arg { expr: fe, by_ref: None }], optional: false }, start)
            }
            _ => {
                let msg = format!("expected an expression, found {}", self.found());
                self.err("P0004", start, msg);
                if !matches!(self.kind(), Tok::RParen | Tok::RBrace | Tok::RBracket | Tok::Semi | Tok::Eof | Tok::Comma) {
                    self.bump();
                }
                self.mk_expr(ExprKind::Error, start)
            }
        }
    }

    /// `name(params): T { body }` in an object literal: a field holding an arrow function
    /// (there is no `this` for it to see).
    fn object_method(&mut self, start: Span, is_async: bool) -> Option<ObjField> {
        let (name, name_span) = self.prop_name()?;
        if self.at(Tok::Lt) {
            let span = self.cur_span();
            self.err("U0015", span, "generic methods in object literals are not supported");
            self.tparams();
        }
        let params = self.params();
        let defaults = (std::mem::take(&mut self.defaults), std::mem::take(&mut self.prologue));
        let ret = if self.eat(Tok::Colon) { Some(self.ty()) } else { None };
        if !self.at(Tok::LBrace) {
            let msg = format!("expected `{{` to start the method body, found {}", self.found());
            let span = self.cur_span();
            self.err("P0001", span, msg);
            return None;
        }
        let b = self.block();
        let body = ArrowBody::Block(self.with_defaults(defaults, b));
        let span = start.to(self.prev_span());
        let value = self.mk_expr(ExprKind::Arrow(Box::new(ArrowFn { params, ret, throws: None, body, is_async })), span);
        Some(ObjField { name, name_span, value, quoted: false })
    }

    fn object_literal(&mut self) -> ExprId {
        let start = self.cur_span();
        self.bump(); // {
        let mut fields = Vec::new();
        // keys written as expressions (`[Color.Red]: v`, `1: v`), by field index: the literal is a
        // `Record<K, V>`'s, a map (see the end)
        let mut keyed: Vec<(usize, ExprId)> = Vec::new();
        while !self.at(Tok::RBrace) && !self.at(Tok::Eof) {
            let fstart = self.cur_span();
            if self.at(Tok::DotDotDot) {
                // `{ ...a, y: 3 }`: a's fields (see `check::rewrite`), named "..."
                self.bump();
                let x = self.assign();
                let span = fstart.to(self.prev_span());
                let value = self.mk_expr(ExprKind::Spread(x), span);
                let name = self.interner.intern("...");
                fields.push(ObjField { name, name_span: span, value, quoted: false });
            } else if self.at(Tok::Int) || self.at(Tok::LBracket) {
                let key = if self.eat(Tok::LBracket) {
                    let k = self.assign();
                    self.expect(Tok::RBracket, "after the computed key");
                    k
                } else {
                    self.primary()
                };
                let name_span = fstart.to(self.prev_span());
                if self.expect(Tok::Colon, "after the key") {
                    let value = self.assign();
                    let name = self.interner.intern("[key]");
                    keyed.push((fields.len(), key));
                    fields.push(ObjField { name, name_span, value, quoted: false });
                }
            } else if self.at(Tok::Str) {
                // `"key": value` — valid for a `Record<string, V>` (or a record field of that name).
                let t = self.bump();
                let text = self.strings[t.val as usize].clone();
                let name = self.interner.intern(&text);
                // A route key ("/users/:id"): its params become field names of `req.params`.
                for seg in text.split('/') {
                    if let Some(p) = seg.strip_prefix(':') {
                        self.interner.intern(p);
                    }
                }
                self.interner.intern("params");
                let name_span = self.span_of(t);
                if self.expect(Tok::Colon, "after the key") {
                    let value = self.assign();
                    fields.push(ObjField { name, name_span, value, quoted: true });
                }
            } else if self.at(Tok::Async) && matches!(self.nth(1), Tok::Ident) && self.nth(2) == Tok::LParen {
                // `async name(params) { ... }`
                self.bump();
                if let Some(f) = self.object_method(fstart, true) {
                    fields.push(f);
                }
            } else if matches!(self.kind(), Tok::Ident) && self.nth(1) == Tok::LParen || self.kind().is_keyword() && self.nth(1) == Tok::LParen {
                if let Some(f) = self.object_method(fstart, false) {
                    fields.push(f);
                }
            } else if let Some((name, name_span)) = self.prop_name() {
                if self.eat(Tok::Colon) {
                    let value = self.assign();
                    fields.push(ObjField { name, name_span, value, quoted: false });
                } else {
                    // Shorthand `{ r }`.
                    let value = self.mk_expr(ExprKind::Ident(name), name_span);
                    fields.push(ObjField { name, name_span, value, quoted: false });
                }
            } else {
                let msg = format!("expected a field name, found {}", self.found());
                self.err("P0001", fstart, msg);
                self.recover_line();
                if self.at(Tok::RBrace) {
                    break;
                }
                continue;
            }
            if !self.eat(Tok::Comma) {
                if !self.at(Tok::RBrace) {
                    let msg = format!("expected `,` or `}}` in the object literal, found {}", self.found());
                    let span = self.cur_span();
                    self.err("P0001", span, msg);
                }
                break;
            }
        }
        self.expect(Tok::RBrace, "to close the object literal");
        let span = start.to(self.prev_span());
        if !keyed.is_empty() {
            return self.keyed_object(fields, keyed, span);
        }
        self.mk_expr(ExprKind::Object(fields), span)
    }

    /// `{ [k]: v, 1: w, name: x }`: `new Map([[k, v], [1, w], ["name", x]])` (a `Record<K, V>`
    /// is a map; records' field names are identifiers).
    fn keyed_object(&mut self, fields: Vec<ObjField>, keyed: Vec<(usize, ExprId)>, span: Span) -> ExprId {
        let mut entries = Vec::new();
        for (i, f) in fields.into_iter().enumerate() {
            if matches!(self.ast.expr(f.value).kind, ExprKind::Spread(_)) {
                self.push(Diagnostic::new("X0017", f.name_span, "a spread among computed keys is not supported").note("instead", "build the map with `new Map(...)` and `set`"));
                continue;
            }
            let key = match keyed.iter().find(|(at, _)| *at == i) {
                Some(&(_, k)) => k,
                None => self.mk_expr(ExprKind::Str(f.name), f.name_span),
            };
            let es = self.expr_span(key).to(self.expr_span(f.value));
            entries.push(self.mk_expr(ExprKind::Array(vec![key, f.value]), es));
        }
        let list = self.mk_expr(ExprKind::Array(entries), span);
        let map = self.interner.intern("Map");
        let callee = self.mk_expr(ExprKind::Ident(map), span);
        let e = self.mk_expr(ExprKind::New { callee, type_args: Vec::new(), args: vec![Arg { expr: list, by_ref: None }] }, span);
        self.ast.keyed_objects.push(e);
        e
    }
}

/// Does the statement use `await` outside nested functions (a script with top-level `await`)?
fn top_level_await(ast: &Ast, s: StmtId) -> bool {
    fn expr(ast: &Ast, e: ExprId) -> bool {
        match &ast.expr(e).kind {
            ExprKind::Await(_) => true,
            ExprKind::Arrow(_) => false,
            ExprKind::Yield(x, _) => x.is_some_and(|x| expr(ast, x)),
            ExprKind::Unary(_, x) | ExprKind::Paren(x) | ExprKind::NonNull(x) | ExprKind::Typeof(x) | ExprKind::Spread(x) | ExprKind::As(x, _) | ExprKind::Try(x) => expr(ast, *x),
            ExprKind::Update { target, .. } => expr(ast, *target),
            ExprKind::Binary(_, a, b) | ExprKind::Assign(_, a, b) | ExprKind::Index { obj: a, index: b, .. } => expr(ast, *a) || expr(ast, *b),
            ExprKind::Cond(a, b, c) => expr(ast, *a) || expr(ast, *b) || expr(ast, *c),
            ExprKind::Call { callee, args, .. } | ExprKind::New { callee, args, .. } => expr(ast, *callee) || args.iter().any(|a| expr(ast, a.expr)),
            ExprKind::Member { obj, .. } => expr(ast, *obj),
            ExprKind::Template(_, xs) | ExprKind::Array(xs) | ExprKind::Seq(xs) => xs.iter().any(|&x| expr(ast, x)),
            ExprKind::Object(fs) => fs.iter().any(|f| expr(ast, f.value)),
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::BigInt(_) | ExprKind::Decimal(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Undefined | ExprKind::Null | ExprKind::Ident(_) | ExprKind::This | ExprKind::Super | ExprKind::Error => false,
        }
    }
    let opt = |e: &Option<ExprId>| e.is_some_and(|e| expr(ast, e));
    match &ast.stmt(s).kind {
        StmtKind::Let { init, .. } => opt(init),
        StmtKind::Expr(e) | StmtKind::Throw(e) => expr(ast, *e),
        StmtKind::Return(e) => opt(e),
        StmtKind::If(c, t, e) => expr(ast, *c) || top_level_await(ast, *t) || e.is_some_and(|e| top_level_await(ast, e)),
        StmtKind::While(c, b) | StmtKind::DoWhile(b, c) => expr(ast, *c) || top_level_await(ast, *b),
        StmtKind::For { init, cond, step, body } => init.is_some_and(|i| top_level_await(ast, i)) || opt(cond) || opt(step) || top_level_await(ast, *body),
        StmtKind::ForOf { iter, body, .. } => expr(ast, *iter) || top_level_await(ast, *body),
        StmtKind::Switch(e, cases) => expr(ast, *e) || cases.iter().any(|c| opt(&c.test) || c.body.iter().any(|&b| top_level_await(ast, b))),
        StmtKind::Block(ss) => ss.iter().any(|&b| top_level_await(ast, b)),
        StmtKind::Try { body, catch, finally } => {
            top_level_await(ast, *body) || catch.as_ref().is_some_and(|c| top_level_await(ast, c.body)) || finally.is_some_and(|f| top_level_await(ast, f))
        }
        // `const x = await f()` at the top level of a script: the initializer is on the item
        StmtKind::InitGlobal(ii) => matches!(&ast.items[*ii as usize].kind, ItemKind::Const { init, .. } if expr(ast, *init)),
        StmtKind::Break(_) | StmtKind::Continue(_) | StmtKind::Empty | StmtKind::Error => false,
    }
}
