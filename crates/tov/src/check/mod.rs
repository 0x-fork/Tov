//! Name resolution and type checking.

mod builtins;
pub(crate) mod class;
mod expr;
pub mod facts;
pub mod rewrite;
mod spec;
mod stmt;

pub use facts::{CallFact, Callee, IdentFact, MemberFact, ModuleFacts, NpmName};

/// A JavaScript typed array's element type (`Float64Array` → `f64`), for code generation.
pub fn typed_array_elem(name: &str) -> Option<TyId> {
    builtins::typed_array_elem(name)
}

/// `__native.name`'s parameter types and result (for code generation).
pub fn native_sig_pub(types: &mut Types, name: &str) -> Option<(Vec<TyId>, TyId)> {
    builtins::native_sig(types, name)
}

use crate::ast::{self, Ast, ExprId, ItemKind, StmtId, TypeExprKind};
use crate::diag::{similar, Applicability, Diagnostic};
use crate::intern::{Interner, Sym};
use crate::source::{SourceMap, Span};
use crate::types::*;
use crate::hash::{FxMap as HashMap, FxSet as HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub struct Module {
    pub file: crate::source::FileId,
    pub path: PathBuf,
    pub name: String,
    pub ast: Ast,
    /// Import item index → imported module index (for successfully resolved imports).
    pub imports: HashMap<u32, u32>,
    /// The built-in prelude (`Error` and friends): its exports are visible in every module.
    pub builtin: bool,
    /// Part of the standard library (the prelude, `node:fs`, ...): may use `__native`.
    pub std: bool,
    /// Named on the command line (only entry modules may be scripts).
    pub entry: bool,
    /// An npm package (or a Node.js built-in Tov has no module for), as imported: its exports
    /// are JavaScript values (`Js`), bundled with the program (see crate::npm).
    pub npm: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Decl {
    Fn(u32, u32),
    Const(u32, u32),
    Alias(u32),
    Iface(u32),
    Ns(u32),
    Class(u32),
    /// Imported from an npm package (module index): a `Js` value, or `Js` as a type.
    Npm(u32, facts::NpmName),
}

#[derive(Default, Clone)]
pub(crate) struct ModScope {
    values: HashMap<Sym, (Decl, Span)>,
    pub(crate) types: HashMap<Sym, (Decl, Span)>,
    exported_values: HashSet<Sym>,
    exported_types: HashSet<Sym>,
}

#[derive(Clone)]
pub(crate) struct AliasInfo {
    pub(crate) module: u32,
    pub(crate) item: u32,
    pub(crate) params: Vec<u32>,
    recursive: bool,
    body: Option<TyId>,
    resolving: bool,
}

#[derive(Clone)]
struct IfaceInfo {
    module: u32,
    item: u32,
    params: Vec<u32>,
    fields: Option<Vec<Field>>,
    resolving: bool,
}

#[derive(Clone)]
pub(crate) struct GParam {
    pub(crate) name: Sym,
    pub(crate) bound: Option<TyId>,
    /// `<T = undefined>`: used when inference leaves it open.
    pub(crate) default: Option<TyId>,
}

#[derive(Clone)]
pub(crate) struct Sig {
    pub(crate) tparams: Vec<u32>,
    pub(crate) params: Vec<FnParam>,
    pub(crate) param_names: Vec<Sym>,
    pub(crate) ret: TyId,
    /// The errors it can throw (`NEVER`: none; `UNKNOWN` while being inferred).
    pub(crate) throws: TyId,
    /// The last parameter is `...xs: T[]` (callers pass the elements as separate arguments).
    pub(crate) rest: bool,
}

impl Sig {
    /// Call-site view: fixed parameters and, for a rest parameter, its element type.
    pub(crate) fn call_params(&self, types: &Types) -> (Vec<FnParam>, Option<TyId>) {
        if !self.rest {
            return (self.params.clone(), None);
        }
        let n = self.params.len() - 1;
        let elem = match types.get(self.params[n].ty) {
            Ty::Array(e) => *e,
            _ => ERROR,
        };
        (self.params[..n].to_vec(), Some(elem))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LocalKind {
    Let,
    Const,
    Param,
    Inout,
    /// `for (const x of xs)`: a copy of the element.
    LoopVar,
}

struct Local {
    name: Sym,
    ty: TyId,
    kind: LocalKind,
    span: Span,
    /// The module `(module, item)` this local shadows, when it stands in for a top-level
    /// `const`/`let` so flow narrowing can see it: reads and writes still resolve globally.
    global: Option<(u32, u32)>,
    /// Span of the `const` keyword, for "change to `let`" fixes.
    kw_span: Option<Span>,
    /// `let x = <int literal>` without annotation: may become `f64` from later uses.
    promotable: bool,
    frame: usize,
    /// The speculated `number`s its type came from (see `spec`).
    sites: Vec<spec::Site>,
}

type LocalId = usize;

struct Frame {
    ret: Option<TyId>,
    /// The return annotation's speculated `number`s.
    ret_sites: Vec<spec::Site>,
    returns: Vec<TyId>,
    loops: u32,
    switches: u32,
    /// Labels of the enclosing loops (`outer: for`), innermost last.
    labels: Vec<Sym>,
    name: Option<Sym>,
    /// Functions and methods can throw; closures can't (yet).
    can_throw: bool,
    /// The declared `throws` type (`None`: inferred from `thrown`).
    throws_decl: Option<TyId>,
    /// Errors that leave the function (thrown or passed on with `try`).
    thrown: Vec<TyId>,
    /// Enclosing `try { }` blocks: the errors each one catches.
    try_frames: Vec<Vec<TyId>>,
    /// Inside the operand of a `try` expression.
    try_expr: u32,
    /// A module variable's initializer (not a closure) — for the "can't throw" message.
    module_init: bool,
    /// An async function or arrow (or an async script's top level): `await` is allowed.
    is_async: bool,
}

impl Frame {
    fn new(ret: Option<TyId>, name: Option<Sym>, can_throw: bool, throws_decl: Option<TyId>) -> Frame {
        Frame { ret, ret_sites: Vec::new(), returns: Vec::new(), loops: 0, switches: 0, labels: Vec::new(), name, can_throw, throws_decl, thrown: Vec::new(), try_frames: Vec::new(), try_expr: 0, module_init: false, is_async: false }
    }
}

#[derive(Default)]
struct FnCtx {
    locals: Vec<Local>,
    /// Innermost last. Each entry: (name, local, narrowed type).
    pub(crate) scopes: Vec<Vec<(Sym, LocalId, Option<TyId>)>>,
    frames: Vec<Frame>,
    tscope: Vec<(Sym, TyId)>,
    promoted: HashSet<u32>,
    pending: HashSet<u32>,
    /// A non-exhaustive `switch` was reported; "missing return" would only repeat it.
    reported_nonexhaustive: bool,
    /// Narrowings of field paths (`t.left`): (local, path, narrowed type or `None` = invalidated).
    /// Scope entries named `PATH_SYM` point into this table, so they follow scope lifetimes.
    path_table: Vec<(LocalId, Vec<Sym>, Option<TyId>)>,
    /// Narrowings of module constants (`const port = process.env.PORT; if (port !== undefined)`);
    /// scope entries named `GLOBAL_SYM` point into it.
    global_table: Vec<((u32, u32), TyId)>,
    /// For the copy-then-mutate check: `let b = a` bindings (copy, source, position),
    /// in-place mutations (local, span, how many fields/elements deep) and reads (local,
    /// position) of locals. An alias records how deep in its source it was taken.
    aliases: Vec<(LocalId, LocalId, u32, u32, Span)>,
    /// Values put in a collection or record: (container, value, position). Changing the value
    /// afterwards changes the container in TypeScript, but not the copy Tov stored.
    stored: Vec<(LocalId, LocalId, u32)>,
    /// Locals copied from, or stored into, a class instance or module variable (position, the
    /// place's span): TypeScript would share them with it.
    heap_links: Vec<(LocalId, u32, Span)>,
    mutations: Vec<(LocalId, Span, u32)>,
    /// `let x = x ?? d` starting a body, for a parameter `x = d` (see `parser::with_defaults`):
    /// such a local and the parameter's name.
    default_shadows: Vec<LocalId>,
    reads: Vec<(LocalId, u32)>,
    /// Names of locals assigned or changed (through a field, an element or a method call) inside a
    /// closure: never narrowed (the closure may run between the check and the use)...
    closure_mutated: HashSet<Sym>,
    /// ...except that a local holding only references (class instances, promises, functions) keeps
    /// its narrowing unless a closure assigns it: changing an object doesn't change its class.
    closure_assigned: HashSet<Sym>,
    /// `routes` handlers' request parameters and their route (`"/users/:id"`): `req.params`
    /// is typed from the route.
    route_locals: Vec<(LocalId, Sym)>,
    /// Active narrowings of paths that go through a class instance (`this.left`): any call may
    /// change them through another reference, so calls forget them.
    heap_paths: Vec<(LocalId, Vec<Sym>)>,
    /// The class whose member is being checked (for visibility, `super` and constructors).
    class: Option<u32>,
    /// Declaration key of `this` (for code generation).
    this_key: Option<u32>,
    /// Checking a constructor (or field initializers): `readonly` fields can be assigned.
    ctor: bool,
    /// Expressions whose value is a class instance (mutations through them change the heap object).
    class_valued: HashSet<ExprId>,
    /// A `test(...)` body: its top-level closure may throw (an uncaught error fails the test).
    test_body: bool,
}

/// Marks a scope entry that refers to `FnCtx::path_table`.
pub(crate) const PATH_SYM: Sym = Sym(u32::MAX);
/// The name `this` is declared under inside class methods.
pub(crate) const THIS_SYM: Sym = Sym(u32::MAX - 1);
/// Marks a scope entry that refers to `FnCtx::global_table`.
pub(crate) const GLOBAL_SYM: Sym = Sym(u32::MAX - 2);

#[derive(Clone, Copy)]
struct Syms {
    kind: Sym,
    main: Sym,
    u: Sym,
    number: Sym,
    /// `0`, `1`, ...: the fields of tuples the checker makes (`map.entries()`'s `[K, V]`).
    digits: [Sym; 4],
}

fn intern_syms(interner: &mut Interner) -> Syms {
    let digits = [interner.intern("0"), interner.intern("1"), interner.intern("2"), interner.intern("3")];
    Syms { kind: interner.intern("kind"), main: interner.intern("main"), u: interner.intern("U"), number: interner.intern("number"), digits }
}

/// Checking runs in two phases. Phase A (one thread) resolves everything that forms a module's
/// interface: type aliases, interfaces, function signatures (checking bodies of functions whose
/// return type is inferred) and constants. Phase B checks the remaining function bodies and tests,
/// one module at a time on every core. The tables behind `Arc` are only written in phase A;
/// phase B workers share them read-only and put new types in a per-module overlay, so results
/// (and diagnostic text) don't depend on how modules are spread across threads.
pub struct Checker<'a> {
    pub(crate) modules: &'a [Module],
    pub(crate) interner: &'a Interner,
    pub(crate) sm: &'a SourceMap,
    pub(crate) types: Types,
    /// Diagnostics for the unit currently being checked (a function body may be checked twice).
    diags: Vec<Diagnostic>,
    done: Vec<Diagnostic>,
    scopes: Arc<Vec<ModScope>>,
    pub(crate) aliases: Arc<Vec<AliasInfo>>,
    alias_names: Arc<Vec<String>>,
    ifaces: Arc<Vec<IfaceInfo>>,
    iface_names: Arc<Vec<String>>,
    pub(crate) classes: Arc<Vec<class::ClassInfo>>,
    pub(crate) class_names: Arc<Vec<String>>,
    /// Class member bodies checked in phase A (read-only in phase B) and by this checker.
    checked_members_base: Arc<HashSet<(u32, u32)>>,
    checked_members: HashSet<(u32, u32)>,
    class_resolving: HashSet<u32>,
    /// Overrides whose (inferred) return types are compared after their class is resolved.
    pending_overrides: Vec<(class::CMethod, class::CMethod)>,
    /// Inferred `throws` of method bodies being checked, keyed by (class, member).
    inferred_throws: HashMap<(u32, u32), TyId>,
    /// Generic parameters: frozen phase-A base, then this checker's own (ids continue after the base).
    gparams_base: Arc<Vec<GParam>>,
    gparams: Vec<GParam>,
    /// Names of the generic parameters, kept separately so type display doesn't allocate.
    param_names_base: Arc<Vec<Sym>>,
    param_names: Vec<Sym>,
    module_names: Arc<Vec<String>>,
    pub(crate) sigs: Arc<HashMap<(u32, u32), Sig>>,
    sig_in_progress: HashSet<(u32, u32)>,
    /// Signatures (assumed to throw) for recursive calls made while a body is inferred.
    sig_provisional: HashMap<(u32, u32), Sig>,
    /// Bodies checked in phase A (read-only in phase B) and by this checker.
    checked_bodies: Arc<HashSet<(u32, u32)>>,
    checked_local: HashSet<(u32, u32)>,
    /// Checking the value of a `Record` literal entry whose key is a route with `:params`.
    route_key: Option<Sym>,
    pub(crate) consts: Arc<HashMap<(u32, u32), TyId>>,
    /// Module variables of a script whose initializer can throw (the script checks after it).
    pub(crate) const_throws: Arc<HashSet<(u32, u32)>>,
    const_in_progress: HashSet<(u32, u32)>,
    /// A constant's type while its initializer is checked, when it's known before (annotated, or
    /// a call whose signature fixes it): a closure in the initializer can use the constant.
    const_provisional: HashMap<(u32, u32), TyId>,
    /// Constants whose provisional type a closure in their initializer used.
    const_provisional_used: HashSet<(u32, u32)>,
    exhaustive: HashSet<(u32, StmtId)>,
    fcx: Vec<FnCtx>,
    cur: u32,
    /// Generic parameters being inferred by the call currently being checked.
    infer_free: Vec<u32>,
    syms: Syms,
    /// Per-module facts for code generation (`check_for_build` only).
    pub(crate) facts: Option<Vec<ModuleFacts>>,
    /// The call expression whose instantiated signature `call_sig` should record.
    pending_call: Option<ExprId>,
    /// `number` checked as `int` (see `spec`): the sites known to be floats, and what this
    /// checker found: sites a float reached, the destinations of the values being checked,
    /// where expressions' types came from, and record fields' sites (frozen phase-A base, then
    /// this checker's own).
    spec: Arc<spec::Spec>,
    spec_used: bool,
    spec_hits: Vec<spec::Site>,
    /// Bodies found to throw though their signatures were taken not to: (module, body).
    throw_hits: Vec<(u32, StmtId)>,
    /// Module variables `let x = 0` a float is stored into, and interfaces made records that a
    /// class instance is given to: (module, item).
    global_hits: Vec<(u32, u32)>,
    spec_flow: Vec<Vec<spec::Site>>,
    spec_srcs_base: Arc<HashMap<Span, Vec<spec::Site>>>,
    spec_srcs: HashMap<Span, Vec<spec::Site>>,
    spec_fields_base: Arc<HashMap<(TyId, Sym), Vec<spec::Site>>>,
    spec_fields: HashMap<(TyId, Sym), Vec<spec::Site>>,
    /// Parameter and return sites of the callee `call_sig` is about to check.
    call_spec: Option<(Vec<Vec<spec::Site>>, Vec<spec::Site>)>,
    /// Checking a value stored into an element of a sized-integer array (`bytes[i] = n`,
    /// `bytes.push(n)`): a number wraps into it, as into a JavaScript typed array.
    wrap_store: bool,
    /// What the driver rewrites before checking again (see `rewrite`).
    pub(crate) rewrites: rewrite::Rewrites,
    /// The next expression checked may be absent (see `expr_inner`).
    want_optional: bool,
    /// Checking a built-in method's arguments: an integer parameter takes a number (truncated).
    float_args: bool,
    /// What callbacks given to built-in methods throw (`xs.map((x) => parse(x))`), as their
    /// arrows are checked: the method call passes it on.
    cb_throws: Vec<TyId>,
}

/// `threads`: phase-B worker count (default: all cores). Results don't depend on it.
fn new_checker<'a>(modules: &'a [Module], interner: &'a Interner, sm: &'a SourceMap, syms: Syms, spec: Arc<spec::Spec>) -> Checker<'a> {
    Checker {
        modules,
        interner,
        sm,
        types: Types::default(),
        diags: Vec::new(),
        done: Vec::new(),
        scopes: Arc::default(),
        aliases: Arc::default(),
        alias_names: Arc::default(),
        ifaces: Arc::default(),
        iface_names: Arc::default(),
        classes: Arc::default(),
        class_names: Arc::default(),
        checked_members_base: Arc::default(),
        checked_members: HashSet::default(),
        class_resolving: HashSet::default(),
        pending_overrides: Vec::new(),
        inferred_throws: HashMap::default(),
        gparams_base: Arc::default(),
        gparams: Vec::new(),
        param_names_base: Arc::default(),
        param_names: Vec::new(),
        module_names: Arc::new(modules.iter().map(|m| m.name.clone()).collect()),
        sigs: Arc::default(),
        sig_in_progress: HashSet::default(),
        sig_provisional: HashMap::default(),
        checked_bodies: Arc::default(),
        checked_local: HashSet::default(),
        route_key: None,
        consts: Arc::default(),
        const_throws: Arc::default(),
        const_in_progress: HashSet::default(),
        const_provisional: HashMap::default(),
        const_provisional_used: HashSet::default(),
        exhaustive: HashSet::default(),
        fcx: Vec::new(),
        cur: 0,
        infer_free: Vec::new(),
        syms,
        facts: None,
        pending_call: None,
        spec,
        spec_used: false,
        spec_hits: Vec::new(),
        throw_hits: Vec::new(),
        global_hits: Vec::new(),
        spec_flow: Vec::new(),
        spec_srcs_base: Arc::default(),
        spec_srcs: HashMap::default(),
        spec_fields_base: Arc::default(),
        spec_fields: HashMap::default(),
        call_spec: None,
        wrap_store: false,
        rewrites: rewrite::Rewrites::default(),
        want_optional: false,
        float_args: false,
        cb_throws: Vec::new(),
    }
}

/// Checks the program and keeps everything code generation needs: one type table for the
/// whole program and per-module facts. Single-threaded.
pub fn check_for_build<'a>(modules: &'a [Module], interner: &'a mut Interner, sm: &'a SourceMap) -> (Vec<Diagnostic>, Checker<'a>) {
    let syms = intern_syms(interner);
    let interner: &'a Interner = interner;
    let mut floats: HashSet<spec::Site> = HashSet::default();
    let mut throwing: HashSet<(u32, StmtId)> = HashSet::default();
    let mut float_globals: HashSet<(u32, u32)> = HashSet::default();
    let (mut on, mut rounds) = (true, 0);
    loop {
        rounds += 1;
        let mut c = new_checker(modules, interner, sm, syms, Arc::new(spec::Spec { on, floats: floats.clone(), throwing: throwing.clone(), float_globals: float_globals.clone() }));
        c.facts = Some(modules.iter().map(|m| ModuleFacts::new(m.ast.exprs.len())).collect());
        c.collect();
        c.resolve_imports();
        c.bind_web_globals();
        c.find_recursive_aliases();
        for m in 0..modules.len() as u32 {
            c.cur = m;
            c.check_interface(m);
        }
        c.check_class_cycles();
        for m in 0..modules.len() as u32 {
            c.cur = m;
            c.check_bodies(m);
        }
        let mut out = std::mem::take(&mut c.done);
        out.extend(std::mem::take(&mut c.diags));
        if spec::more_throwing(&mut throwing, &c.throw_hits) | spec::more_throwing(&mut float_globals, &c.global_hits) {
            continue;
        }
        if on && c.spec_used && std::env::var("TOV_TRACE").is_ok() {
            eprintln!("check: `number` as `int`, round {rounds}: {} new float site(s), {} error(s)", c.spec_hits.iter().filter(|s| !floats.contains(s)).count(), out.len());
            for d in &out {
                eprintln!("  {}", crate::diag::render_text(std::slice::from_ref(d), sm).lines().next().unwrap_or(""));
            }
        }
        if on
            && c.spec_used
            && let Some(again) = spec::next_round(&mut floats, &c.spec_hits, !out.is_empty(), rounds)
        {
            on = again;
            continue;
        }
        if std::env::var("TOV_TRACE").is_ok() && c.spec_used {
            eprintln!("check: `number` as `int`: {}, {rounds} round(s), {} float site(s)", if on { "on" } else { "off" }, floats.len());
        }
        out.sort_by(|a, b| (a.span.file, a.span.start, a.code).cmp(&(b.span.file, b.span.start, b.code)));
        out.dedup_by(|a, b| a.span == b.span && a.code == b.code && a.message == b.message);
        return (out, c);
    }
}

pub fn check_program(modules: &[Module], interner: &mut Interner, sm: &SourceMap, threads: Option<usize>) -> Vec<Diagnostic> {
    check_program_rewrites(modules, interner, sm, threads).0
}

/// `check_program`, and what the driver may rewrite and check again (see `rewrite`).
pub fn check_program_rewrites(modules: &[Module], interner: &mut Interner, sm: &SourceMap, threads: Option<usize>) -> (Vec<Diagnostic>, rewrite::Rewrites) {
    let syms = intern_syms(interner);
    let interner = &*interner;
    let mut floats: HashSet<spec::Site> = HashSet::default();
    let mut throwing: HashSet<(u32, StmtId)> = HashSet::default();
    let mut float_globals: HashSet<(u32, u32)> = HashSet::default();
    let (mut on, mut rounds) = (true, 0);
    loop {
        rounds += 1;
        let (out, used, hits, (throws, globals), rw) = check_program_once(modules, interner, sm, threads, syms, Arc::new(spec::Spec { on, floats: floats.clone(), throwing: throwing.clone(), float_globals: float_globals.clone() }));
        if spec::more_throwing(&mut throwing, &throws) | spec::more_throwing(&mut float_globals, &globals) {
            continue;
        }
        if on
            && used
            && let Some(again) = spec::next_round(&mut floats, &hits, !out.is_empty(), rounds)
        {
            on = again;
            continue;
        }
        return (out, rw);
    }
}

/// One check of the whole program; also returns whether speculation was used and its hits.
#[allow(clippy::type_complexity)]
fn check_program_once(modules: &[Module], interner: &Interner, sm: &SourceMap, threads: Option<usize>, syms: Syms, spec: Arc<spec::Spec>) -> (Vec<Diagnostic>, bool, Vec<spec::Site>, (Vec<(u32, StmtId)>, Vec<(u32, u32)>), rewrite::Rewrites) {
    let mut c = new_checker(modules, interner, sm, syms, spec);
    c.collect();
    c.resolve_imports();
    c.bind_web_globals();
    c.find_recursive_aliases();
    let ta = std::time::Instant::now();
    // Phase A.
    for m in 0..modules.len() as u32 {
        c.cur = m;
        c.check_interface(m);
    }
    c.check_class_cycles();
    let mut out = std::mem::take(&mut c.done);
    out.extend(std::mem::take(&mut c.diags));
    let mut used = c.spec_used;
    let mut hits = std::mem::take(&mut c.spec_hits);
    let mut throws = std::mem::take(&mut c.throw_hits);
    let mut globals = std::mem::take(&mut c.global_hits);
    let mut rw = std::mem::take(&mut c.rewrites);
    // Freeze phase-A results.
    c.checked_bodies = Arc::new(std::mem::take(&mut c.checked_local));
    c.checked_members_base = Arc::new(std::mem::take(&mut c.checked_members));
    c.gparams_base = Arc::new(std::mem::take(&mut c.gparams));
    c.param_names_base = Arc::new(std::mem::take(&mut c.param_names));
    c.spec_fields_base = Arc::new(std::mem::take(&mut c.spec_fields));
    c.spec_srcs_base = Arc::new(std::mem::take(&mut c.spec_srcs));
    let base = Arc::new(std::mem::take(&mut c.types));
    let tb = std::time::Instant::now();
    // Phase B: modules are handed out dynamically; each is checked from a clean overlay.
    let next = AtomicUsize::new(0);
    // Default: all cores, but no more than one thread per ~8 modules (tiny projects stay on one).
    let threads = threads.unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(modules.len().div_ceil(8))).clamp(1, modules.len().max(1));
    let c = &c;
    let run = |base: Arc<Types>| {
        let mut w = c.worker(base);
        loop {
            let m = next.fetch_add(1, Ordering::Relaxed);
            if m >= modules.len() {
                break;
            }
            w.reset();
            w.cur = m as u32;
            w.check_bodies(m as u32);
        }
        let mut d = std::mem::take(&mut w.done);
        d.extend(w.diags);
        (d, w.spec_used, w.spec_hits, (w.throw_hits, w.global_hits), w.rewrites)
    };
    let results: Vec<(Vec<Diagnostic>, bool, Vec<spec::Site>, (Vec<(u32, StmtId)>, Vec<(u32, u32)>), rewrite::Rewrites)> = if threads == 1 {
        vec![run(base.clone())]
    } else {
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|_| {
                    let base = base.clone();
                    let run = &run;
                    scope.spawn(move || run(base))
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("checker thread panicked")).collect()
        })
    };
    for (r, u, h, (t, g), w) in results {
        out.extend(r);
        used |= u;
        hits.extend(h);
        throws.extend(t);
        globals.extend(g);
        rw.extend(w);
    }
    if std::env::var("TOV_TRACE").is_ok() {
        eprintln!("check: phase A {:?}, phase B {:?} ({threads} threads)", tb - ta, tb.elapsed());
    }
    out.sort_by(|a, b| (a.span.file, a.span.start, a.code).cmp(&(b.span.file, b.span.start, b.code)));
    out.dedup_by(|a, b| a.span == b.span && a.code == b.code && a.message == b.message);
    (out, used, hits, (throws, globals), rw)
}

impl<'a> Checker<'a> {
    // ---------------------------------------------------------------- helpers

    fn ast(&self) -> &'a Ast {
        &self.modules[self.cur as usize].ast
    }

    pub(crate) fn name(&self, s: Sym) -> &str {
        match s {
            THIS_SYM => "this",
            PATH_SYM => "<path>",
            _ => self.interner.get(s),
        }
    }

    /// A phase-B checker sharing this checker's frozen tables.
    fn worker(&self, base: Arc<Types>) -> Checker<'a> {
        Checker {
            modules: self.modules,
            interner: self.interner,
            sm: self.sm,
            types: Types::overlay(base),
            diags: Vec::new(),
            done: Vec::new(),
            scopes: self.scopes.clone(),
            aliases: self.aliases.clone(),
            alias_names: self.alias_names.clone(),
            ifaces: self.ifaces.clone(),
            iface_names: self.iface_names.clone(),
            classes: self.classes.clone(),
            class_names: self.class_names.clone(),
            checked_members_base: self.checked_members_base.clone(),
            checked_members: HashSet::default(),
            class_resolving: HashSet::default(),
            pending_overrides: Vec::new(),
            inferred_throws: HashMap::default(),
            gparams_base: self.gparams_base.clone(),
            gparams: Vec::new(),
            param_names_base: self.param_names_base.clone(),
            param_names: Vec::new(),
            module_names: self.module_names.clone(),
            sigs: self.sigs.clone(),
            sig_in_progress: HashSet::default(),
            sig_provisional: HashMap::default(),
            checked_bodies: self.checked_bodies.clone(),
            checked_local: HashSet::default(),
            route_key: None,
            consts: self.consts.clone(),
            const_throws: self.const_throws.clone(),
            const_in_progress: HashSet::default(),
            const_provisional: HashMap::default(),
            const_provisional_used: HashSet::default(),
            exhaustive: HashSet::default(),
            fcx: Vec::new(),
            cur: 0,
            infer_free: Vec::new(),
            syms: self.syms,
            facts: None,
            pending_call: None,
            spec: self.spec.clone(),
            spec_used: false,
            spec_hits: Vec::new(),
            throw_hits: Vec::new(),
            global_hits: Vec::new(),
            spec_flow: Vec::new(),
            spec_srcs_base: self.spec_srcs_base.clone(),
            spec_srcs: HashMap::default(),
            spec_fields_base: self.spec_fields_base.clone(),
            spec_fields: HashMap::default(),
            call_spec: None,
            wrap_store: false,
            rewrites: rewrite::Rewrites::default(),
            want_optional: false,
            float_args: false,
            cb_throws: Vec::new(),
        }
    }

    /// Forgets everything created while checking the previous module.
    fn reset(&mut self) {
        self.types.reset();
        self.gparams.clear();
        self.param_names.clear();
        self.exhaustive.clear();
        self.checked_local.clear();
        self.checked_members.clear();
        self.spec_fields.clear();
    }

    #[inline]
    fn facts_mut(&mut self) -> Option<&mut ModuleFacts> {
        let cur = self.cur as usize;
        self.facts.as_mut().map(|f| &mut f[cur])
    }

    fn rec_ident(&mut self, e: ExprId, fact: IdentFact) {
        if let Some(f) = self.facts_mut() {
            f.idents.insert(e, fact);
        }
    }

    fn rec_call(&mut self, e: ExprId, callee: Callee) {
        if let Some(f) = self.facts_mut() {
            f.calls.insert(e, CallFact { callee, targs: Vec::new(), params: Vec::new(), rest: None, ret: ERROR });
        }
    }

    fn rec_binding(&mut self, s: StmtId, ty: TyId) {
        if let Some(f) = self.facts_mut() {
            f.bindings.insert(s, ty);
        }
    }

    pub(crate) fn gparam(&self, p: u32) -> &GParam {
        let base = self.gparams_base.len();
        if (p as usize) < base { &self.gparams_base[p as usize] } else { &self.gparams[p as usize - base] }
    }

    fn src(&self, span: Span) -> &'a str {
        self.sm.slice(span)
    }

    fn report(&mut self, d: Diagnostic) {
        self.diags.push(d);
    }

    pub(crate) fn show(&self, ty: TyId) -> String {
        Display {
            types: &self.types,
            interner: self.interner,
            param_names: (&self.param_names_base, &self.param_names),
            rec_names: &self.alias_names,
            iface_names: &self.iface_names,
            class_names: &self.class_names,
            module_names: &self.module_names,
        }
        .show(ty)
    }

    /// Runs `f` with its diagnostics committed immediately (for cached results that won't be recomputed).
    fn committed<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = std::mem::take(&mut self.diags);
        let r = f(self);
        let produced = std::mem::replace(&mut self.diags, saved);
        self.done.extend(produced);
        r
    }

    fn with_module<R>(&mut self, m: u32, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = self.cur;
        self.cur = m;
        let r = f(self);
        self.cur = saved;
        r
    }

    fn new_gparam(&mut self, name: Sym, bound: Option<TyId>) -> u32 {
        self.gparams.push(GParam { name, bound, default: None });
        self.param_names.push(name);
        (self.gparams_base.len() + self.gparams.len() - 1) as u32
    }

    // ---------------------------------------------------------------- module scopes

    fn collect(&mut self) {
        let modules = self.modules;
        // An interface no class implements holds only object literals: it's a record type (an
        // alias of its members), as fast as one and changed in place like one.
        let mut implemented = HashSet::default();
        for module in modules.iter() {
            for item in &module.ast.items {
                if let ItemKind::Class(cd) = &item.kind {
                    for &te in &cd.implements {
                        if let TypeExprKind::Named { name, .. } = &module.ast.ty(te).kind {
                            implemented.insert(*name);
                        }
                    }
                }
            }
        }
        for (mi, module) in modules.iter().enumerate() {
            let mi = mi as u32;
            let mut scope = ModScope::default();
            for (ii, item) in module.ast.items.iter().enumerate() {
                let ii = ii as u32;
                let (name, span, decl, is_type) = match &item.kind {
                    ItemKind::Function(f) => (f.name, f.name_span, Decl::Fn(mi, ii), false),
                    ItemKind::Const { name, name_span, .. } => (*name, *name_span, Decl::Const(mi, ii), false),
                    ItemKind::TypeAlias { name, name_span, .. } => {
                        Arc::make_mut(&mut self.aliases).push(AliasInfo { module: mi, item: ii, params: Vec::new(), recursive: false, body: None, resolving: false });
                        Arc::make_mut(&mut self.alias_names).push(self.interner.get(*name).to_string());
                        (*name, *name_span, Decl::Alias(self.aliases.len() as u32 - 1), true)
                    }
                    ItemKind::Interface { name, name_span, .. } if !implemented.contains(name) && !self.spec.float_globals.contains(&(mi, ii)) => {
                        Arc::make_mut(&mut self.aliases).push(AliasInfo { module: mi, item: ii, params: Vec::new(), recursive: false, body: None, resolving: false });
                        Arc::make_mut(&mut self.alias_names).push(self.interner.get(*name).to_string());
                        (*name, *name_span, Decl::Alias(self.aliases.len() as u32 - 1), true)
                    }
                    ItemKind::Interface { name, name_span, .. } => {
                        Arc::make_mut(&mut self.ifaces).push(IfaceInfo { module: mi, item: ii, params: Vec::new(), fields: None, resolving: false });
                        Arc::make_mut(&mut self.iface_names).push(self.interner.get(*name).to_string());
                        (*name, *name_span, Decl::Iface(self.ifaces.len() as u32 - 1), true)
                    }
                    ItemKind::Class(cd) => {
                        Arc::make_mut(&mut self.classes).push(class::ClassInfo {
                            module: mi,
                            item: ii,
                            params: Vec::new(),
                            this_ty: ERROR,
                            base: None,
                            fields: Vec::new(),
                            methods: Vec::new(),
                            statics: Vec::new(),
                            static_fields: Vec::new(),
                            ctor: class::CCtor { member: None, throws: Some(NEVER), params: Vec::new(), param_names: Vec::new() },
                            is_abstract: cd.is_abstract,
                            cyclic: cd.cyclic,
                            resolved: false,
                        });
                        Arc::make_mut(&mut self.class_names).push(self.interner.get(cd.name).to_string());
                        let decl = Decl::Class(self.classes.len() as u32 - 1);
                        // A class is both a type and a value (`new C()`, `C.staticMember`).
                        if builtins::is_builtin_type(self.interner.get(cd.name)) {
                            let n = self.interner.get(cd.name).to_string();
                            self.done.push(Diagnostic::new("N0005", cd.name_span, format!("`{n}` is a built-in type and can't be redeclared")));
                            continue;
                        }
                        if let Some((_, prev)) = scope.types.get(&cd.name).or_else(|| scope.values.get(&cd.name)) {
                            let (line, _) = self.sm.get(prev.file).line_col(prev.start);
                            let n = self.interner.get(cd.name).to_string();
                            self.done.push(Diagnostic::new("N0003", cd.name_span, format!("`{n}` is already declared in this module")).note("previous", format!("line {line}")));
                            continue;
                        }
                        scope.values.insert(cd.name, (decl, cd.name_span));
                        scope.types.insert(cd.name, (decl, cd.name_span));
                        if item.exported {
                            scope.exported_values.insert(cd.name);
                            scope.exported_types.insert(cd.name);
                        }
                        continue;
                    }
                    _ => continue,
                };
                if is_type && builtins::is_builtin_type(self.interner.get(name)) {
                    let n = self.interner.get(name).to_string();
                    self.done.push(Diagnostic::new("N0005", span, format!("`{n}` is a built-in type and can't be redeclared")));
                    continue;
                }
                let table = if is_type { &mut scope.types } else { &mut scope.values };
                if let Some((_, prev)) = table.get(&name) {
                    let (line, _) = self.sm.get(prev.file).line_col(prev.start);
                    let n = self.interner.get(name).to_string();
                    self.done.push(Diagnostic::new("N0003", span, format!("`{n}` is already declared in this module")).note("previous", format!("line {line}")));
                    continue;
                }
                table.insert(name, (decl, span));
                if item.exported {
                    if is_type {
                        scope.exported_types.insert(name);
                    } else {
                        scope.exported_values.insert(name);
                    }
                }
            }
            Arc::make_mut(&mut self.scopes).push(scope);
        }
        // The prelude's exports are in scope everywhere, unless a module declares the name.
        if let Some(b) = modules.iter().position(|m| m.builtin) {
            let bs = self.scopes[b].clone();
            for (mi, module) in modules.iter().enumerate() {
                if module.builtin {
                    continue;
                }
                let scope = &mut Arc::make_mut(&mut self.scopes)[mi];
                for (&name, &(decl, span)) in bs.values.iter().filter(|(n, _)| bs.exported_values.contains(n)) {
                    scope.values.entry(name).or_insert((decl, span));
                }
                for (&name, &(decl, span)) in bs.types.iter().filter(|(n, _)| bs.exported_types.contains(n)) {
                    scope.types.entry(name).or_insert((decl, span));
                }
            }
        }
    }

    /// Binds std/http's web globals (`Bun`, `Response`, ...) in every program module that
    /// doesn't declare the name itself.
    fn bind_web_globals(&mut self) {
        let modules = self.modules;
        let Some(w) = modules.iter().position(|m| m.path.as_os_str() == "<std>/http") else { return };
        let ws = self.scopes[w].clone();
        for (mi, module) in modules.iter().enumerate() {
            if module.std {
                continue;
            }
            let scope = &mut Arc::make_mut(&mut self.scopes)[mi];
            for name in crate::driver::WEB_GLOBALS {
                let Some(sym) = self.interner.lookup(name) else { continue };
                if let Some(&(decl, span)) = ws.values.get(&sym) {
                    scope.values.entry(sym).or_insert((decl, span));
                }
                if let Some(&(decl, span)) = ws.types.get(&sym) {
                    scope.types.entry(sym).or_insert((decl, span));
                }
            }
        }
    }

    fn resolve_imports(&mut self) {
        let modules = self.modules;
        for (mi, module) in modules.iter().enumerate() {
            for (ii, item) in module.ast.items.iter().enumerate() {
                let ItemKind::Import(imp) = &item.kind else { continue };
                let Some(&target) = module.imports.get(&(ii as u32)) else { continue };
                if modules[target as usize].npm.is_some() {
                    // Anything a package exports is a JavaScript value; names are checked when
                    // the program runs (a missing export reads as `undefined`, as in JavaScript).
                    let binds = imp.namespace.map(|n| (n, facts::NpmName::Ns)).into_iter()
                        .chain(imp.default.map(|n| (n, facts::NpmName::Default)))
                        .chain(imp.names.iter().map(|&(s, sp)| ((s, sp), facts::NpmName::Named(s))));
                    for ((name, span), what) in binds {
                        self.bind_import(mi, name, span, Decl::Npm(target, what), false);
                        self.bind_import(mi, name, span, Decl::Npm(target, what), true);
                    }
                    continue;
                }
                if let Some((name, span)) = imp.default {
                    let n = self.interner.get(name).to_string();
                    self.done.push(
                        Diagnostic::new("X0011", span, "default imports are only for npm packages")
                            .note("instead", format!("import named exports: `import {{ {n} }} from ...` or `import * as {n} from ...`")),
                    );
                }
                if let Some((ns, span)) = imp.namespace {
                    self.bind_import(mi, ns, span, Decl::Ns(target), false);
                    self.bind_import(mi, ns, span, Decl::Ns(target), true);
                    continue;
                }
                for &(name, span) in &imp.names {
                    let ts = &self.scopes[target as usize];
                    let value = ts.values.get(&name).filter(|_| ts.exported_values.contains(&name)).map(|d| d.0);
                    let ty = ts.types.get(&name).filter(|_| ts.exported_types.contains(&name)).map(|d| d.0);
                    if value.is_none() && ty.is_none() {
                        let n = self.interner.get(name).to_string();
                        let mut exports: Vec<&str> = ts.exported_values.iter().chain(ts.exported_types.iter()).map(|s| self.interner.get(*s)).collect();
                        exports.sort();
                        let private = ts.values.contains_key(&name) || ts.types.contains_key(&name);
                        let target_name = &modules[target as usize].name;
                        let mut d = Diagnostic::new("N0102", span, format!("`{target_name}` has no export named `{n}`"));
                        if private {
                            d = d.note("note", format!("`{n}` is declared there but not exported; add `export` to its declaration"));
                        } else {
                            let sug = similar(&n, exports.iter().copied());
                            if let Some(first) = sug.first() {
                                d = d.note("did you mean", sug.join(", ")).fix(Applicability::Maybe, format!("import `{first}`"), span, first.to_string());
                            }
                        }
                        d = d.note("exports", if exports.is_empty() { "(none)".to_string() } else { exports.join(", ") });
                        self.done.push(d);
                        continue;
                    }
                    if let Some(v) = value {
                        self.bind_import(mi, name, span, v, false);
                    }
                    if let Some(t) = ty {
                        self.bind_import(mi, name, span, t, true);
                    }
                }
            }
        }
    }

    fn bind_import(&mut self, m: usize, name: Sym, span: Span, decl: Decl, is_type: bool) {
        let scope = &mut Arc::make_mut(&mut self.scopes)[m];
        let table = if is_type { &mut scope.types } else { &mut scope.values };
        if let Some((existing, prev)) = table.get(&name) {
            if *existing != decl {
                let n = self.interner.get(name).to_string();
                let (line, _) = self.sm.get(prev.file).line_col(prev.start);
                self.done.push(Diagnostic::new("N0003", span, format!("`{n}` is already declared in this module")).note("previous", format!("line {line}")));
            }
            return;
        }
        table.insert(name, (decl, span));
    }

    /// Aliases that refer to themselves (directly or through other aliases) stay nominal.
    fn find_recursive_aliases(&mut self) {
        let n = self.aliases.len();
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (ai, info) in self.aliases.iter().enumerate() {
            let ast = &self.modules[info.module as usize].ast;
            let Some((_, _, _, ty)) = alias_item(&ast.items[info.item as usize].kind) else { continue };
            let mut refs = Vec::new();
            collect_type_refs(ast, ty, &mut refs);
            for (ns, name) in refs {
                let decl = match ns {
                    None => self.scopes[info.module as usize].types.get(&name).map(|d| d.0),
                    Some(ns) => match self.scopes[info.module as usize].values.get(&ns).map(|d| d.0) {
                        Some(Decl::Ns(t)) => self.scopes[t as usize].types.get(&name).map(|d| d.0),
                        _ => None,
                    },
                };
                if let Some(Decl::Alias(target)) = decl {
                    edges[ai].push(target as usize);
                }
            }
        }
        // A node is recursive if it can reach itself.
        for start in 0..n {
            let mut seen = vec![false; n];
            let mut stack = edges[start].clone();
            while let Some(x) = stack.pop() {
                if x == start {
                    Arc::make_mut(&mut self.aliases)[start].recursive = true;
                    break;
                }
                if !seen[x] {
                    seen[x] = true;
                    stack.extend(edges[x].iter().copied());
                }
            }
        }
    }

    // ---------------------------------------------------------------- items

    /// Phase A: everything other modules can depend on.
    fn check_interface(&mut self, m: u32) {
        let ast = self.ast();
        if let Some(span) = ast.script_span
            && !self.modules[m as usize].entry
        {
            self.done.push(
                Diagnostic::new("P0201", span, "only the program's entry file can have top-level statements")
                    .note("why", "importing a module has no side effects; its top level holds only declarations")
                    .note("instead", "export a function and call it from the entry file"),
            );
        }
        let mut test_names: HashMap<&str, Span> = HashMap::default();
        for (ii, item) in ast.items.iter().enumerate() {
            let ii = ii as u32;
            match &item.kind {
                ItemKind::TypeAlias { name, .. } | ItemKind::Interface { name, .. } if matches!(item.kind, ItemKind::TypeAlias { .. }) || matches!(self.scopes[m as usize].types.get(name).map(|d| d.0), Some(Decl::Alias(_))) => {
                    if let Some(Decl::Alias(a)) = self.scopes[m as usize].types.get(name).map(|d| d.0)
                        && self.aliases[a as usize].item == ii
                        && self.aliases[a as usize].module == m
                    {
                        self.alias_body(a);
                    }
                }
                ItemKind::TypeAlias { .. } => {}
                ItemKind::Interface { name, .. } => {
                    if let Some(Decl::Iface(i)) = self.scopes[m as usize].types.get(name).map(|d| d.0)
                        && self.ifaces[i as usize].item == ii
                    {
                        self.iface_fields(i);
                    }
                }
                ItemKind::Function(f) => {
                    self.fn_sig(m, ii);
                    if f.name == self.syms.main {
                        let sig = self.sigs[&(m, ii)].clone();
                        // `async function main()` returns `Promise<void>` or `Promise<int>`.
                        let ret = self.body_sig(f, &sig).ret;
                        if !sig.params.is_empty() || !(ret == VOID || ret == INT || ret == ERROR) {
                            self.done.push(
                                Diagnostic::new("T0701", f.name_span, "`main` must take no parameters and return `void` or `int`")
                                    .note("expected", "function main() { ... }   or   function main(): int { ... }"),
                            );
                        }
                    }
                }
                ItemKind::Const { .. } => {
                    self.const_type(m, ii);
                }
                ItemKind::Test { name, name_span, .. } => {
                    if let Some(prev) = test_names.insert(name.as_str(), *name_span) {
                        let (line, _) = self.sm.get(prev.file).line_col(prev.start);
                        self.done.push(Diagnostic::new("N0004", *name_span, format!("duplicate test name \"{name}\"")).note("previous", format!("line {line}")));
                    }
                }
                ItemKind::Import(_) => {}
                ItemKind::Class(cd) => {
                    if let Some(Decl::Class(c)) = self.scopes[m as usize].types.get(&cd.name).map(|d| d.0)
                        && self.classes[c as usize].item == ii
                        && self.classes[c as usize].module == m
                    {
                        self.check_class_interface(c);
                    }
                }
            }
        }
    }

    /// Phase B: function bodies not already checked in phase A, and tests.
    fn check_bodies(&mut self, m: u32) {
        let ast = self.ast();
        for (ii, item) in ast.items.iter().enumerate() {
            match &item.kind {
                ItemKind::Function(_) => self.check_fn_body(m, ii as u32),
                ItemKind::Test { body, .. } => self.check_test(*body),
                ItemKind::Class(cd) => {
                    if let Some(Decl::Class(c)) = self.scopes[m as usize].types.get(&cd.name).map(|d| d.0)
                        && self.classes[c as usize].item == ii as u32
                        && self.classes[c as usize].module == m
                    {
                        self.check_class_bodies(c);
                    }
                }
                _ => {}
            }
        }
    }

    fn check_test(&mut self, body: ExprId) {
        let expected = self.types.func(Vec::new(), VOID);
        let (closure_mutated, closure_assigned) = stmt::closure_mutated_expr(self.ast(), body);
        self.fcx.push(FnCtx { closure_mutated, closure_assigned, test_body: true, ..Default::default() });
        self.fcx.last_mut().unwrap().scopes.push(Vec::new());
        let t = self.expr(body, Some(expected));
        self.check_shared_copies();
        if !self.assignable(t, expected) {
            let span = self.ast().expr(body).span;
            let msg = format!("a test body must be `() => {{ ... }}`, found `{}`", self.show(t));
            self.report(Diagnostic::new("T0001", span, msg));
        }
        self.fcx.pop();
        let produced = std::mem::take(&mut self.diags);
        self.done.extend(produced);
    }

    fn const_type(&mut self, m: u32, ii: u32) -> TyId {
        if let Some(&t) = self.consts.get(&(m, ii)) {
            return t;
        }
        let ItemKind::Const { name, name_span, ty, init, mutable } = &self.modules[m as usize].ast.items[ii as usize].kind else { return ERROR };
        let mutable = *mutable;
        // In a script, initializers run in the script body, so they can throw like its statements.
        let script = self.modules[m as usize].ast.script.is_some() && self.modules[m as usize].entry;
        if !self.const_in_progress.insert((m, ii)) {
            // (a closure in the initializer: `const server = Bun.serve({ ... server.publish(...) })`)
            if let Some(&t) = self.const_provisional.get(&(m, ii)) {
                self.const_provisional_used.insert((m, ii));
                return t;
            }
            let n = self.name(*name).to_string();
            self.done.push(
                Diagnostic::new("T0308", *name_span, format!("constant `{n}` depends on itself"))
                    .note("instead", "annotate its type, or call with explicit type arguments, so a closure in its initializer can use it"),
            );
            return ERROR;
        }
        let (ty, init) = (*ty, *init);
        let provisional = match ty {
            Some(te) => Some(self.with_module(m, |c| c.resolve_type(te, &[]))),
            None => self.with_module(m, |c| c.peek_call_type(init)),
        };
        if let Some(p) = provisional {
            self.const_provisional.insert((m, ii), p);
        }
        let (t, throws) = self.committed(|c| {
            c.with_module(m, |c| {
                c.fcx.push(FnCtx::default());
                c.fcx.last_mut().unwrap().scopes.push(Vec::new());
                let mut frame = Frame::new(None, None, script, None);
                frame.module_init = true;
                frame.is_async = script && c.ast().script.is_some_and(|si| matches!(&c.ast().items[si as usize].kind, ItemKind::Function(f) if f.is_async));
                c.fcx.last_mut().unwrap().frames.push(frame);
                let t = match ty {
                    Some(te) => {
                        let declared = c.resolve_type(te, &[]);
                        let sites = c.type_sites(m, te);
                        c.flow(sites, |c| {
                            let found = c.expr(init, Some(declared));
                            c.expect_assignable(found, declared, c.ast().expr(init).span, None);
                        });
                        declared
                    }
                    None => {
                        let found = c.expr(init, None);
                        let t = c.check_inferred_binding(found, c.ast().expr(init).span);
                        // `let s = ""` holds any string later, as in TypeScript; `let x = 0`
                        // that's given a float is an `f64`
                        let t = if mutable { c.types.widen(t) } else { t };
                        if t == INT && c.spec.float_globals.contains(&(m, ii)) { F64 } else { t }
                    }
                };
                let throws = !c.fcx.last().unwrap().frames[0].thrown.is_empty();
                c.fcx.pop();
                (t, throws)
            })
        });
        if throws {
            Arc::make_mut(&mut self.const_throws).insert((m, ii));
        }
        self.const_in_progress.remove(&(m, ii));
        let used = self.const_provisional_used.remove(&(m, ii));
        if let Some(p) = self.const_provisional.remove(&(m, ii))
            && used
            && p != t
            && t != ERROR
            && !self.assignable(t, p)
        {
            let n = self.name(*name).to_string();
            let (ps, ts) = (self.show(p), self.show(t));
            self.done.push(
                Diagnostic::new("T0308", *name_span, format!("constant `{n}` is used in its own initializer as `{ps}`, but it's a `{ts}`"))
                    .note("instead", "annotate its type"),
            );
        }
        Arc::make_mut(&mut self.consts).insert((m, ii), t);
        t
    }

    /// The type a call or `new` produces as its callee's signature says, when its type parameters
    /// are fixed without looking at the arguments (given explicitly, or defaulted).
    fn peek_call_type(&mut self, init: ExprId) -> Option<TyId> {
        use crate::ast::ExprKind;
        let (mut e, mut awaited) = (init, false);
        loop {
            match &self.ast().expr(e).kind {
                ExprKind::Paren(x) | ExprKind::Try(x) => e = *x,
                ExprKind::Await(x) => {
                    awaited = true;
                    e = *x;
                }
                _ => break,
            }
        }
        let (tparams, ret, targs) = match &self.ast().expr(e).kind {
            ExprKind::Call { callee, type_args, .. } => {
                let targs = type_args.clone();
                match &self.ast().expr(*callee).kind {
                    ExprKind::Ident(s) => match self.scopes[self.cur as usize].values.get(s).map(|d| d.0) {
                        Some(Decl::Fn(fm, fi)) => {
                            let sig = self.fn_sig(fm, fi)?;
                            (sig.tparams, sig.ret, targs)
                        }
                        _ => return None,
                    },
                    ExprKind::Member { obj, name, .. } => {
                        let ExprKind::Ident(s) = self.ast().expr(*obj).kind else { return None };
                        let Some(Decl::Class(c)) = self.scopes[self.cur as usize].values.get(&s).map(|d| d.0) else { return None };
                        self.resolve_class(c);
                        let mm = self.classes[c as usize].statics.iter().find(|x| x.name == *name).cloned()?;
                        let ret = self.method_ret(&mm);
                        (mm.tparams.clone(), ret, targs)
                    }
                    _ => return None,
                }
            }
            _ => return None,
        };
        let mut map: HashMap<u32, TyId> = HashMap::default();
        let tscope = self.tscope();
        for (i, &p) in tparams.iter().enumerate() {
            let t = match targs.get(i) {
                Some(&ta) => self.resolve_type(ta, &tscope),
                None => self.gparam(p).default?,
            };
            map.insert(p, t);
        }
        let t = self.types.subst(ret, &map);
        if awaited {
            return match *self.types.get(t) {
                Ty::Promise(v, _) => Some(v),
                _ => Some(t),
            };
        }
        Some(t)
    }

    /// Validates the type of an unannotated binding.
    fn check_inferred_binding(&mut self, t: TyId, span: Span) -> TyId {
        if t == UNDEFINED {
            self.report(
                Diagnostic::new("T0306", span, "can't infer a type from `undefined` alone")
                    .note("instead", "annotate the binding: `let x: T | undefined = undefined`"),
            );
            return ERROR;
        }
        if t == VOID || t == NEVER {
            self.report(Diagnostic::new("T0307", span, format!("this expression has type `{}`, so it has no value to bind", self.show(t))));
            return ERROR;
        }
        if matches!(self.types.get(t), Ty::Namespace(_) | Ty::BuiltinNs(_) | Ty::Expect(_)) {
            self.report(Diagnostic::new("T0307", span, format!("`{}` is not a value", self.show(t))));
            return ERROR;
        }
        t
    }

    fn tscope(&self) -> Vec<(Sym, TyId)> {
        self.fcx.last().map(|f| f.tscope.clone()).unwrap_or_default()
    }

    pub(crate) fn fn_sig(&mut self, m: u32, ii: u32) -> Option<Sig> {
        if let Some(s) = self.sigs.get(&(m, ii)) {
            return Some(s.clone());
        }
        let ItemKind::Function(f) = &self.modules[m as usize].ast.items[ii as usize].kind else { return None };
        if self.sig_in_progress.contains(&(m, ii)) {
            // A recursive call while the body is checked for what it throws: the declared
            // signature, assuming it may throw.
            return self.sig_provisional.get(&(m, ii)).cloned();
        }
        self.sig_in_progress.insert((m, ii));
        let sig = self.committed(|c| {
            c.with_module(m, |c| {
                let mut tscope = Vec::new();
                let mut tparams = Vec::new();
                for tp in &f.tparams {
                    let id = c.new_gparam(tp.name, None);
                    let ty = c.types.intern(Ty::Param(id));
                    tscope.push((tp.name, ty));
                    tparams.push(id);
                }
                for (tp, &id) in f.tparams.iter().zip(&tparams) {
                    if let Some(b) = tp.bound {
                        let bound = c.resolve_type(b, &tscope);
                        let base = c.gparams_base.len();
                        c.gparams[id as usize - base].bound = Some(bound);
                    }
                    if let Some(d) = tp.default {
                        let default = c.resolve_type(d, &tscope);
                        let base = c.gparams_base.len();
                        c.gparams[id as usize - base].default = Some(default);
                    }
                }
                let mut params = Vec::new();
                let mut param_names = Vec::new();
                for (pi, p) in f.params.iter().enumerate() {
                    if p.rest {
                        if pi + 1 != f.params.len() {
                            c.report(Diagnostic::new("P0001", p.span, "a rest parameter `...xs` must be the last parameter"));
                        }
                        if p.optional || p.inout {
                            c.report(Diagnostic::new("P0001", p.span, "a rest parameter can't be optional or `inout`"));
                        }
                        if let Some(t) = p.ty
                            && !matches!(c.ast().ty(t).kind, TypeExprKind::Array(_))
                            && !matches!(&c.ast().ty(t).kind, TypeExprKind::Named { name, .. } if c.name(*name) == "Array")
                        {
                            let s = c.ast().ty(t).span;
                            c.report(Diagnostic::new("T0001", s, "a rest parameter has an array type: `...xs: T[]`"));
                        }
                    }
                    let ty = match p.ty {
                        Some(t) => c.resolve_type(t, &tscope),
                        None => {
                            let n = c.name(p.name).to_string();
                            c.report(
                                Diagnostic::new("T0301", p.span, format!("parameter `{n}` needs a type annotation"))
                                    .fix(Applicability::Placeholder, format!("annotate `{n}: T`"), p.span.empty_at_end(), ": T"),
                            );
                            ERROR
                        }
                    };
                    let ty = if p.optional { c.types.optional(ty) } else { ty };
                    params.push(FnParam { ty, inout: p.inout, optional: p.optional });
                    param_names.push(p.name);
                }
                let ret = f.ret.map(|t| c.resolve_type(t, &tscope));
                // An async function's body returns the `T` of its `Promise<T>`.
                let ret = match ret {
                    Some(r) if f.is_async => Some(c.async_inner(r, f.ret.map(|te| c.ast().ty(te).span).unwrap())),
                    r => r,
                };
                let throws = f.throws.map(|t| c.resolve_type(t, &tscope));
                if let Some(t) = throws {
                    c.check_throws_type(t, f.throws.map(|te| c.ast().ty(te).span).unwrap());
                }
                (tparams, params, param_names, ret, throws, tscope)
            })
        });
        let (tparams, params, param_names, ret, declared_throws, tscope) = sig;
        let body_throws = may_throw(&self.modules[m as usize].ast, f.body) || self.spec.throwing.contains(&(m, f.body));
        let mut throws = declared_throws.unwrap_or(if body_throws { UNKNOWN } else { NEVER });
        if let Some(r) = ret
            && throws == UNKNOWN
        {
            let err = self.error_class();
            let rest = f.params.last().map(|p| p.rest).unwrap_or(false);
            let (r, t) = if f.is_async { (self.types.promise(r, err), NEVER) } else { (r, err) };
            self.sig_provisional.insert((m, ii), Sig { tparams: tparams.clone(), params: params.clone(), param_names: param_names.clone(), ret: r, throws: t, rest });
        }
        let ret = match ret {
            Some(r) if throws != UNKNOWN => r,
            // No `return <value>` anywhere: `void`, and the body can wait for phase B.
            None if !returns_value(&self.modules[m as usize].ast, f.body) && throws != UNKNOWN => VOID,
            declared => {
                // Infer from the body (the return type, the errors, or both).
                let partial = Sig { tparams: tparams.clone(), params: params.clone(), param_names: param_names.clone(), ret: declared.unwrap_or(ERROR), throws, rest: false };
                let (inferred, errs) = self.check_body_of(m, ii, &partial, &tscope, declared.or(if returns_value(&self.modules[m as usize].ast, f.body) { None } else { Some(VOID) }));
                if throws == UNKNOWN {
                    throws = errs;
                }
                if let Some(r) = declared {
                    r
                } else {
                    let inferred = if returns_value(&self.modules[m as usize].ast, f.body) { inferred } else { VOID };
                inferred
                }
            }
        };
        let rest = f.params.last().map(|p| p.rest).unwrap_or(false);
        // Callers of an async function get a promise; its errors reject the promise.
        let (ret, throws) = if f.is_async { (self.types.promise(ret, throws), NEVER) } else { (ret, throws) };
        let sig = Sig { tparams, params, param_names, ret, throws, rest };
        self.sig_in_progress.remove(&(m, ii));
        Arc::make_mut(&mut self.sigs).insert((m, ii), sig.clone());
        Some(sig)
    }

    fn check_fn_body(&mut self, m: u32, ii: u32) {
        if self.checked_bodies.contains(&(m, ii)) || self.checked_local.contains(&(m, ii)) {
            return;
        }
        let Some(sig) = self.fn_sig(m, ii) else { return };
        if self.checked_bodies.contains(&(m, ii)) || self.checked_local.contains(&(m, ii)) {
            return;
        }
        let ItemKind::Function(f) = &self.modules[m as usize].ast.items[ii as usize].kind else { return };
        let tscope: Vec<(Sym, TyId)> = f.tparams.iter().zip(&sig.tparams).map(|(tp, &id)| (tp.name, self.types.intern(Ty::Param(id)))).collect();
        let sig = self.body_sig(f, &sig);
        self.check_body_of(m, ii, &sig, &tscope, Some(sig.ret));
    }

    /// The body's view of a signature: an async function's body returns the `T` of its
    /// `Promise<T>` and throws what the promise rejects with.
    pub(crate) fn body_sig(&self, f: &ast::FnDecl, sig: &Sig) -> Sig {
        if f.is_async
            && let Ty::Promise(v, e) = *self.types.get(sig.ret)
        {
            return Sig { ret: v, throws: e, ..sig.clone() };
        }
        sig.clone()
    }

    /// The `T` an async function's declared `Promise<T>` resolves to (reports anything else).
    pub(crate) fn async_inner(&mut self, declared: TyId, span: Span) -> TyId {
        match *self.types.get(declared) {
            Ty::Promise(v, _) => v,
            Ty::Error => ERROR,
            _ => {
                let shown = self.show(declared);
                self.report(
                    Diagnostic::new("T0850", span, format!("an async function returns a `Promise`, found `{shown}`"))
                        .fix(Applicability::Safe, format!("return `Promise<{shown}>`"), span, format!("Promise<{shown}>")),
                );
                declared
            }
        }
    }

    /// A class from the built-in prelude (`Error`, `SyntaxError`, ...), for code generation.
    pub(crate) fn builtin_error_class(&mut self, name: &str) -> TyId {
        self.builtin_class(name)
    }

    /// The built-in `Error` class type.
    pub(crate) fn error_class(&mut self) -> TyId {
        let b = self.modules.iter().position(|m| m.builtin);
        let sym = self.interner.lookup("Error");
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

    /// Checks a function body; returns the declared or inferred return type.
    fn check_body_of(&mut self, m: u32, ii: u32, sig: &Sig, tscope: &[(Sym, TyId)], ret: Option<TyId>) -> (TyId, TyId) {
        self.checked_local.insert((m, ii));
        let ItemKind::Function(f) = &self.modules[m as usize].ast.items[ii as usize].kind else { return (ERROR, NEVER) };
        self.check_decl_body(m, f, sig, tscope, ret, None, None)
    }

    /// Checks a function (or method) body; `this` for methods, `class` for member visibility.
    #[allow(clippy::too_many_arguments)]
    /// Returns the (declared or inferred) return type and the errors the body lets out.
    pub(crate) fn check_decl_body(&mut self, m: u32, f: &'a ast::FnDecl, sig: &Sig, tscope: &[(Sym, TyId)], ret: Option<TyId>, this: Option<class::ThisInfo>, class: Option<u32>) -> (TyId, TyId) {
        let mut promoted = HashSet::default();
        let mut result = ERROR;
        let mut thrown = NEVER;
        let saved = std::mem::take(&mut self.diags);
        for _round in 0..4 {
            self.diags.clear();
            let mut fcx = FnCtx { tscope: tscope.to_vec(), promoted: promoted.clone(), ..Default::default() };
            (fcx.closure_mutated, fcx.closure_assigned) = stmt::closure_mutated_stmt(&self.modules[m as usize].ast, f.body);
            fcx.scopes.push(Vec::new());
            // Without a `throws` clause the body's own `throw`s and the calls it makes decide.
            let decl = if sig.throws == UNKNOWN || (f.throws.is_none() && sig.throws == NEVER) { None } else { Some(sig.throws) };
            let mut frame = Frame::new(ret, Some(f.name), true, decl);
            frame.is_async = f.is_async;
            let (param_sites, ret_sites) = self.decl_sites(m, f);
            frame.ret_sites = ret_sites;
            fcx.frames.push(frame);
            fcx.class = class;
            if let Some(t) = &this {
                fcx.locals.push(Local { name: THIS_SYM, ty: t.ty, kind: LocalKind::Param, span: f.name_span, kw_span: None, promotable: false, frame: 0, global: None, sites: Vec::new() });
                fcx.scopes[0].push((THIS_SYM, fcx.locals.len() - 1, None));
                fcx.this_key = Some(f.name_span.start);
                fcx.ctor = t.ctor;
            }
            for ((p, fp), sites) in f.params.iter().zip(&sig.params).zip(param_sites) {
                let kind = if p.inout { LocalKind::Inout } else { LocalKind::Param };
                fcx.locals.push(Local { name: p.name, ty: fp.ty, kind, span: p.span, kw_span: None, promotable: false, frame: 0, global: None, sites });
                let id = fcx.locals.len() - 1;
                fcx.scopes[0].push((p.name, id, None));
            }
            self.fcx.push(fcx);
            let r = self.with_module(m, |c| {
                c.stmt(f.body);
                let body = f.body;
                let fcx = c.fcx.last_mut().unwrap();
                let returns = std::mem::take(&mut fcx.frames[0].returns);
                let errs = std::mem::take(&mut fcx.frames[0].thrown);
                thrown = c.types.union(&errs);
                match ret {
                    Some(r) => {
                        if r != VOID && r != ERROR && !c.always_returns(body) && !c.types.has_undefined(r) && !c.fcx.last().unwrap().reported_nonexhaustive {
                            let n = c.name(f.name).to_string();
                            let end = c.ast().stmt(body).span;
                            let (line, _) = c.sm.get(end.file).line_col(end.end.saturating_sub(1));
                            c.report(
                                Diagnostic::new("F0101", f.name_span, format!("function `{n}` doesn't return a value on every path"))
                                    .note("falls through", format!("at the closing `}}` on line {line}"))
                                    .note("expected", format!("every path ends in `return <{}>`", c.show(r))),
                            );
                        }
                        r
                    }
                    None => {
                        let mut all = returns;
                        if !c.always_returns(body) && !all.is_empty() {
                            all.push(UNDEFINED);
                        }
                        if all.is_empty() { VOID } else { c.types.union(&all) }
                    }
                }
            });
            self.check_shared_copies();
            let fcx = self.fcx.pop().unwrap();
            result = r;
            // A body that throws where its signature was taken not to (it only calls something
            // that throws): its callers are checked again knowing it can.
            if thrown != NEVER && f.throws.is_none() && sig.throws == NEVER {
                self.throw_hits.push((m, f.body));
            }
            let new: Vec<u32> = fcx.pending.difference(&promoted).copied().collect();
            if new.is_empty() {
                break;
            }
            promoted.extend(new);
        }
        let produced = std::mem::replace(&mut self.diags, saved);
        self.done.extend(produced);
        (result, thrown)
    }

    // ---------------------------------------------------------------- types

    pub(crate) fn alias_body(&mut self, a: u32) -> TyId {
        if let Some(b) = self.aliases[a as usize].body {
            return b;
        }
        let (m, ii) = (self.aliases[a as usize].module, self.aliases[a as usize].item);
        let Some((name, name_span, tparams, ty)) = alias_item(&self.modules[m as usize].ast.items[ii as usize].kind) else { return ERROR };
        if self.aliases[a as usize].resolving {
            let n = self.name(*name).to_string();
            self.done.push(Diagnostic::new("N0007", *name_span, format!("type `{n}` refers to itself without a record or array in between")));
            return ERROR;
        }
        Arc::make_mut(&mut self.aliases)[a as usize].resolving = true;
        let body = self.committed(|c| {
            c.with_module(m, |c| {
                let mut tscope = Vec::new();
                let mut params = Vec::new();
                for tp in tparams {
                    let id = c.new_gparam(tp.name, None);
                    tscope.push((tp.name, c.types.intern(Ty::Param(id))));
                    params.push(id);
                }
                Arc::make_mut(&mut c.aliases)[a as usize].params = params;
                let body = c.resolve_type(ty, &tscope);
                if tparams.is_empty() && !c.types.has_name(body) && matches!(c.types.get(body), Ty::Record(_) | Ty::Union(_)) {
                    let n = c.name(*name).to_string();
                    c.types.set_name(body, n);
                }
                body
            })
        });
        let body = if self.aliases[a as usize].recursive && self.unguarded_self_ref(a, body, &mut Vec::new()) {
            let n = self.name(*name).to_string();
            self.done.push(
                Diagnostic::new("N0007", *name_span, format!("type `{n}` refers to itself without a record or array in between"))
                    .note("example", "`type Tree = { kind: \"leaf\" } | { kind: \"node\", children: Tree[] }` is fine; `type A = A | int` is not"),
            );
            ERROR
        } else {
            body
        };
        let info = &mut Arc::make_mut(&mut self.aliases)[a as usize];
        info.resolving = false;
        info.body = Some(body);
        body
    }

    /// Does `ty` reach alias `a` through union members and aliases only (no record/array in between)?
    fn unguarded_self_ref(&mut self, a: u32, ty: TyId, seen: &mut Vec<u32>) -> bool {
        for m in self.types.members(ty) {
            if let Ty::Rec(b, _) = *self.types.get(m) {
                if b == a {
                    return true;
                }
                if seen.contains(&b) {
                    continue;
                }
                seen.push(b);
                let inner = match self.aliases[b as usize].body {
                    Some(body) => body,
                    None if !self.aliases[b as usize].resolving => self.alias_body(b),
                    None => continue,
                };
                if self.unguarded_self_ref(a, inner, seen) {
                    return true;
                }
            }
        }
        false
    }

    fn iface_fields(&mut self, i: u32) -> Vec<Field> {
        if let Some(f) = &self.ifaces[i as usize].fields {
            return f.clone();
        }
        if self.ifaces[i as usize].resolving {
            return Vec::new();
        }
        Arc::make_mut(&mut self.ifaces)[i as usize].resolving = true;
        let (m, ii) = (self.ifaces[i as usize].module, self.ifaces[i as usize].item);
        let ItemKind::Interface { tparams, members, .. } = &self.modules[m as usize].ast.items[ii as usize].kind else { return Vec::new() };
        let fields = self.committed(|c| {
            c.with_module(m, |c| {
                let mut tscope = Vec::new();
                let mut params = Vec::new();
                for tp in tparams {
                    let id = c.new_gparam(tp.name, None);
                    tscope.push((tp.name, c.types.intern(Ty::Param(id))));
                    params.push(id);
                }
                Arc::make_mut(&mut c.ifaces)[i as usize].params = params;
                let mut fields = Vec::new();
                let mut seen = HashSet::default();
                for f in members {
                    if !seen.insert(f.name) {
                        let n = c.name(f.name).to_string();
                        c.report(Diagnostic::new("N0006", f.name_span, format!("duplicate member `{n}`")));
                        continue;
                    }
                    let ty = c.resolve_type(f.ty, &tscope);
                    let ty = if f.optional { c.types.optional(ty) } else { ty };
                    fields.push(Field { name: f.name, ty, optional: f.optional });
                }
                fields.sort_by_key(|f| f.name);
                fields
            })
        });
        let info = &mut Arc::make_mut(&mut self.ifaces)[i as usize];
        info.resolving = false;
        info.fields = Some(fields.clone());
        fields
    }

    pub(crate) fn iface_fields_inst(&mut self, i: u32, args: &[TyId]) -> Vec<Field> {
        let fields = self.iface_fields(i);
        let params = self.ifaces[i as usize].params.clone();
        if params.is_empty() {
            return fields;
        }
        let map: HashMap<u32, TyId> = params.into_iter().zip(args.iter().copied()).collect();
        fields.into_iter().map(|f| Field { ty: self.types.subst(f.ty, &map), ..f }).collect()
    }

    /// One level of unfolding for recursive aliases.
    pub(crate) fn unfold(&mut self, ty: TyId) -> TyId {
        match *self.types.get(ty) {
            Ty::Rec(a, args) => {
                let body = self.alias_body(a);
                let params = self.aliases[a as usize].params.clone();
                let map: HashMap<u32, TyId> = params.into_iter().zip(self.types.tys(args).iter().copied()).collect();
                self.types.subst(body, &map)
            }
            _ => ty,
        }
    }

    /// Union members with recursive aliases unfolded.
    pub(crate) fn flat_members(&mut self, ty: TyId) -> Vec<TyId> {
        let mut out = Vec::new();
        self.flat_members_into(ty, &mut out, 0);
        out.sort();
        out.dedup();
        out
    }

    fn flat_members_into(&mut self, ty: TyId, out: &mut Vec<TyId>, depth: u32) {
        if depth > 32 {
            return;
        }
        let ty = self.unfold(ty);
        for m in self.types.members(ty) {
            let u = self.unfold(m);
            if u != m {
                self.flat_members_into(u, out, depth + 1);
            } else {
                out.push(m);
            }
        }
    }

    /// `typeof x` in a type: what the variable, function or field path `x` holds.
    fn typeof_type(&mut self, x: ExprId, span: Span) -> TyId {
        let t = match self.ast().expr(x).kind {
            ast::ExprKind::Ident(s) => match self.lookup(s) {
                Some((_, t)) => Some(t),
                None => match self.scopes[self.cur as usize].values.get(&s).map(|d| d.0) {
                    Some(Decl::Const(m, i)) => Some(self.const_type(m, i)),
                    Some(Decl::Fn(m, i)) => self.fn_sig(m, i).map(|sig| {
                        let (params, _) = sig.call_params(&self.types);
                        self.types.func_throws(params, sig.ret, if sig.throws == UNKNOWN { NEVER } else { sig.throws })
                    }),
                    _ => None,
                },
            },
            ast::ExprKind::Member { obj, name, .. } => {
                let o = self.typeof_type(obj, span);
                if o == ERROR {
                    return ERROR;
                }
                let o = self.types.without_undefined(o);
                self.field_of(o, name)
            }
            _ => None,
        };
        match t {
            Some(t) => self.types.widen(t),
            None => {
                let text = self.src(self.ast().expr(x).span).to_string();
                self.report(Diagnostic::new("X0016", span, format!("`typeof {text}`: only a variable, a function or a field path has a type here")));
                ERROR
            }
        }
    }

    /// The fields of a record, interface or class type, in order (`None`: not one of those).
    fn type_fields(&mut self, t: TyId) -> Option<Vec<Field>> {
        match *self.types.get(t) {
            Ty::Record(_) => Some(self.types.fields_in_order(t)),
            Ty::Interface(i, args) => {
                let args = self.types.tys(args).to_vec();
                Some(self.iface_fields_inst(i, &args))
            }
            Ty::Class(c, _) => {
                self.resolve_class(c);
                let fs = self.classes[c as usize].fields.clone();
                Some(fs.iter().filter(|f| f.vis == ast::Visibility::Public).map(|f| Field { name: f.name, ty: f.ty, optional: f.optional }).collect())
            }
            _ => None,
        }
    }

    /// `keyof T`: T's field names, as a union of string literals.
    fn keyof_type(&mut self, t: TyId, span: Span) -> TyId {
        if t == ERROR {
            return ERROR;
        }
        if matches!(self.types.get(t), Ty::Map(..)) {
            return STR;
        }
        match self.type_fields(t) {
            Some(fs) => {
                let lits: Vec<TyId> = fs.iter().map(|f| self.types.str_lit(f.name)).collect();
                if lits.is_empty() { NEVER } else { self.types.union(&lits) }
            }
            None => {
                let shown = self.show(t);
                self.report(Diagnostic::new("X0016", span, format!("`keyof {shown}`: only records, interfaces and classes have known keys")));
                ERROR
            }
        }
    }

    /// `T["k"]` (a union of keys gives the union of their types), `T[number]` (an element).
    fn access_type(&mut self, t: TyId, k: TyId, span: Span) -> TyId {
        if t == ERROR || k == ERROR {
            return ERROR;
        }
        if self.types.is_numeric(k)
            && let Ty::Array(e) = *self.types.get(t)
        {
            return e;
        }
        let mut out = Vec::new();
        for key in self.flat_members(k) {
            let Ty::StrLit(name) = *self.types.get(key) else {
                let shown = self.show(k);
                self.report(Diagnostic::new("X0016", span, format!("can't index a type with `{shown}`")).note("supported", "`T[\"field\"]`, a union of field names, `T[number]` for arrays"));
                return ERROR;
            };
            match self.field_of(t, name) {
                Some(ft) => out.push(ft),
                None => {
                    let (shown, n) = (self.show(t), self.name(name).to_string());
                    self.report(Diagnostic::new("T0102", span, format!("`{shown}` has no field `{n}`")));
                    return ERROR;
                }
            }
        }
        self.types.union(&out)
    }

    fn resolve_type(&mut self, te: ast::TypeId, tscope: &[(Sym, TyId)]) -> TyId {
        let ast = self.ast();
        let node = ast.ty(te);
        let span = node.span;
        match &node.kind {
            TypeExprKind::Error => ERROR,
            TypeExprKind::Typeof(x) => self.typeof_type(*x, span),
            TypeExprKind::Predicate(_, t) => {
                self.resolve_type(*t, tscope);
                BOOL
            }
            TypeExprKind::Keyof(t) => {
                let t = self.resolve_type(*t, tscope);
                self.keyof_type(t, span)
            }
            TypeExprKind::Access(t, k) => {
                let (t, k) = (*t, *k);
                let t = self.resolve_type(t, tscope);
                let k = self.resolve_type(k, tscope);
                self.access_type(t, k, span)
            }
            TypeExprKind::Undefined => UNDEFINED,
            TypeExprKind::Void => VOID,
            TypeExprKind::Null => NULL,
            TypeExprKind::StrLit(s) => self.types.str_lit(*s),
            TypeExprKind::Array(e) => {
                let e = self.resolve_type(*e, tscope);
                self.types.array(e)
            }
            TypeExprKind::Union(ms) => {
                let ms: Vec<TyId> = ms.iter().map(|&m| self.resolve_type(m, tscope)).collect();
                self.types.union(&ms)
            }
            TypeExprKind::Record(fields) => {
                let mut out = Vec::new();
                let mut seen = HashSet::default();
                for f in fields {
                    if !seen.insert(f.name) {
                        let n = self.name(f.name).to_string();
                        self.report(Diagnostic::new("N0006", f.name_span, format!("duplicate field `{n}`")));
                        continue;
                    }
                    let ty = self.resolve_type(f.ty, tscope);
                    let ty = if f.optional { self.types.optional(ty) } else { ty };
                    out.push(Field { name: f.name, ty, optional: f.optional });
                }
                let order: Vec<Sym> = out.iter().map(|f| f.name).collect();
                let t = self.types.record(out);
                self.types.note_field_order(t, order);
                if self.spec.on {
                    let m = self.cur;
                    for f in fields {
                        let sites = self.type_sites(m, f.ty);
                        if !sites.is_empty() {
                            self.spec_fields.entry((t, f.name)).or_default().extend(sites);
                        }
                    }
                }
                t
            }
            TypeExprKind::Func(params, ret, throws) => {
                let ps: Vec<FnParam> = params
                    .iter()
                    .map(|p| {
                        let ty = match p.ty {
                            Some(t) => self.resolve_type(t, tscope),
                            None => ERROR,
                        };
                        let ty = if p.optional { self.types.optional(ty) } else { ty };
                        FnParam { ty, inout: p.inout, optional: p.optional }
                    })
                    .collect();
                let r = self.resolve_type(*ret, tscope);
                let th = match throws {
                    Some(t) => {
                        let th = self.resolve_type(*t, tscope);
                        let span = self.ast().ty(*t).span;
                        self.check_throws_type(th, span);
                        th
                    }
                    None => NEVER,
                };
                self.types.func_throws(ps, r, th)
            }
            TypeExprKind::Named { ns: None, name, args, .. } if *name == self.syms.number && args.is_empty() && self.spec_int(te) && !tscope.iter().any(|(s, _)| s == name) => {
                self.spec_used = true;
                INT
            }
            TypeExprKind::Named { ns, name, name_span, args } => self.resolve_named(*ns, *name, *name_span, args, span, tscope),
        }
    }

    fn resolve_named(&mut self, ns: Option<(Sym, Span)>, name: Sym, name_span: Span, args: &[ast::TypeId], span: Span, tscope: &[(Sym, TyId)]) -> TyId {
        let targs: Vec<TyId> = args.iter().map(|&a| self.resolve_type(a, tscope)).collect();
        let text = self.name(name).to_string();
        let decl = if let Some((ns_sym, ns_span)) = ns {
            match self.scopes[self.cur as usize].values.get(&ns_sym).map(|d| d.0) {
                Some(Decl::Ns(t)) => {
                    let ts = &self.scopes[t as usize];
                    match ts.types.get(&name).filter(|_| ts.exported_types.contains(&name)) {
                        Some(d) => Some(d.0),
                        None => {
                            let mname = self.modules[t as usize].name.clone();
                            self.report(Diagnostic::new("N0102", name_span, format!("`{mname}` has no exported type `{text}`")));
                            return ERROR;
                        }
                    }
                }
                _ => {
                    let n = self.name(ns_sym).to_string();
                    self.report(Diagnostic::new("N0002", ns_span, format!("unknown namespace `{n}`")));
                    return ERROR;
                }
            }
        } else {
            if let Some(&(_, t)) = tscope.iter().rev().find(|(s, _)| *s == name) {
                if !targs.is_empty() {
                    self.report(Diagnostic::new("T0010", span, format!("type parameter `{text}` doesn't take type arguments")));
                }
                return t;
            }
            if let Some(t) = self.builtin_type(&text, &targs, span, name_span) {
                return t;
            }
            self.scopes[self.cur as usize].types.get(&name).map(|d| d.0)
        };
        match decl {
            // a type from an npm package: its values are JavaScript's
            Some(Decl::Npm(..)) => JS,
            Some(Decl::Alias(a)) => {
                let (am, ai) = (self.aliases[a as usize].module, self.aliases[a as usize].item);
                let arity = alias_item(&self.modules[am as usize].ast.items[ai as usize].kind).map_or(0, |a| a.2.len());
                let mut targs = targs;
                if targs.len() < arity {
                    let Some((_, _, tparams, _)) = alias_item(&self.modules[am as usize].ast.items[ai as usize].kind) else { unreachable!() };
                    let defaults: Vec<Option<crate::ast::TypeId>> = tparams[targs.len()..].iter().map(|tp| tp.default).collect();
                    if defaults.iter().all(|d| d.is_some()) {
                        for d in defaults.into_iter().flatten() {
                            let t = self.with_module(am, |c| c.resolve_type(d, &[]));
                            targs.push(t);
                        }
                    }
                }
                if arity != targs.len() {
                    self.report(Diagnostic::new(
                        "T0010",
                        span,
                        format!("`{text}` takes {arity} type argument{}, found {}", if arity == 1 { "" } else { "s" }, targs.len()),
                    ));
                    return ERROR;
                }
                if self.aliases[a as usize].recursive {
                    // Stays nominal; the body is resolved on demand, which allows self-reference.
                    return self.types.rec(a, &targs);
                }
                let body = self.alias_body(a);
                let params = self.aliases[a as usize].params.clone();
                if params.len() != targs.len() {
                    self.report(Diagnostic::new(
                        "T0010",
                        span,
                        format!("`{text}` takes {} type argument{}, found {}", params.len(), if params.len() == 1 { "" } else { "s" }, targs.len()),
                    ));
                    return ERROR;
                }
                if self.aliases[a as usize].recursive {
                    return self.types.rec(a, &targs);
                }
                if params.is_empty() {
                    return body;
                }
                let map: HashMap<u32, TyId> = params.into_iter().zip(targs.iter().copied()).collect();
                let inst = self.types.subst(body, &map);
                if !self.types.has_name(inst) && matches!(self.types.get(inst), Ty::Record(_) | Ty::Union(_)) {
                    let shown: Vec<String> = targs.iter().map(|&t| self.show(t)).collect();
                    self.types.set_name(inst, format!("{text}<{}>", shown.join(", ")));
                }
                inst
            }
            Some(Decl::Class(c)) => {
                let arity = self.class_decl(c).tparams.len();
                let mut targs = targs;
                if targs.len() < arity {
                    let cm = self.classes[c as usize].module;
                    let defaults: Vec<Option<crate::ast::TypeId>> = self.class_decl(c).tparams[targs.len()..].iter().map(|tp| tp.default).collect();
                    if defaults.iter().all(|d| d.is_some()) {
                        for d in defaults.into_iter().flatten() {
                            let t = self.with_module(cm, |c| c.resolve_type(d, &[]));
                            targs.push(t);
                        }
                    }
                }
                if arity != targs.len() {
                    self.report(Diagnostic::new("T0010", span, format!("`{text}` takes {arity} type argument{}, found {}", if arity == 1 { "" } else { "s" }, targs.len())));
                    return ERROR;
                }
                self.types.class(c, &targs)
            }
            Some(Decl::Iface(i)) => {
                self.iface_fields(i);
                let params = self.ifaces[i as usize].params.len();
                if params != targs.len() {
                    self.report(Diagnostic::new("T0010", span, format!("`{text}` takes {params} type argument(s), found {}", targs.len())));
                    return ERROR;
                }
                self.types.iface(i, &targs)
            }
            _ => {
                let mut candidates: Vec<String> = builtins::BUILTIN_TYPES.iter().map(|s| s.to_string()).collect();
                candidates.extend(self.scopes[self.cur as usize].types.keys().map(|&s| self.name(s).to_string()));
                candidates.extend(tscope.iter().map(|(s, _)| self.name(*s).to_string()));
                let mut d = Diagnostic::new("N0002", name_span, format!("unknown type `{text}`"));
                let sug = similar(&text, candidates.iter().map(|s| s.as_str()));
                if let Some(first) = sug.first() {
                    d = d.note("did you mean", sug.join(", ")).fix(Applicability::Maybe, format!("use `{first}`"), name_span, first.to_string());
                }
                if self.scopes[self.cur as usize].values.contains_key(&name) {
                    d = d.note("note", format!("`{text}` is a value, not a type"));
                }
                self.report(d);
                ERROR
            }
        }
    }

    fn builtin_type(&mut self, text: &str, targs: &[TyId], span: Span, name_span: Span) -> Option<TyId> {
        let prim = match text {
            "int" | "i64" => Some(INT),
            "f64" | "number" => Some(F64),
            "f32" => Some(F32),
            "i8" => Some(I8),
            "i16" => Some(I16),
            "i32" => Some(I32),
            "u8" => Some(U8),
            "u16" => Some(U16),
            "u32" => Some(U32),
            "u64" => Some(U64),
            "bigint" => Some(BIGINT),
            "decimal" => Some(DECIMAL),
            "bool" | "boolean" => Some(BOOL),
            "string" => Some(STR),
            "never" => Some(NEVER),
            "unknown" => Some(UNKNOWN),
            "Js" => Some(JS),
            _ => None,
        };
        if let Some(p) = prim {
            if !targs.is_empty() {
                self.report(Diagnostic::new("T0010", span, format!("`{text}` doesn't take type arguments")));
            }
            return Some(p);
        }
        if let Some(e) = builtins::typed_array_elem(text) {
            return Some(self.types.array(e));
        }
        let arity = |c: &mut Self, n: usize| -> bool {
            if targs.len() != n {
                c.report(Diagnostic::new("T0010", span, format!("`{text}` takes {n} type argument{}, found {}", if n == 1 { "" } else { "s" }, targs.len())));
                false
            } else {
                true
            }
        };
        match text {
            // (the read-only views are the types themselves: Tov doesn't check writes through them)
            "Array" | "ReadonlyArray" => Some(if arity(self, 1) { self.types.array(targs[0]) } else { ERROR }),
            "Map" | "ReadonlyMap" => Some(if arity(self, 2) { self.types.intern(Ty::Map(targs[0], targs[1])) } else { ERROR }),
            "Set" | "ReadonlySet" => Some(if arity(self, 1) { self.types.intern(Ty::Set(targs[0])) } else { ERROR }),
            "any" => {
                self.report(
                    Diagnostic::new("X0001", name_span, "`any` is not supported")
                        .note("instead", "use the precise type, a union, a generic `<T>`, or `unknown` with narrowing")
                        .fix(Applicability::Maybe, "use `unknown`", name_span, "unknown"),
                );
                Some(ERROR)
            }
            // `Promise<T>` never rejects; `Promise<T, E>` can reject with `E` (errors are checked).
            "Promise" => Some(match *targs {
                [v] => self.types.promise(v, NEVER),
                [v, e] => {
                    self.check_throws_type(e, span);
                    self.types.promise(v, e)
                }
                _ => {
                    self.report(
                        Diagnostic::new("T0010", span, format!("`Promise` takes 1 or 2 type arguments, found {}", targs.len()))
                            .note("expected", "`Promise<T>`, or `Promise<T, E>` for a promise that can reject with `E`"),
                    );
                    ERROR
                }
            }),
            "ReturnType" | "Awaited" | "NonNullable" | "Readonly" | "Partial" | "Required" => {
                if !arity(self, 1) {
                    return Some(ERROR);
                }
                let t = targs[0];
                if t == ERROR {
                    return Some(ERROR);
                }
                Some(match text {
                    "ReturnType" => match *self.types.get(t) {
                        Ty::Func(_, r, _) => r,
                        _ => {
                            let shown = self.show(t);
                            self.report(Diagnostic::new("T0010", span, format!("`ReturnType` takes a function type, found `{shown}`")));
                            ERROR
                        }
                    },
                    "Awaited" => match *self.types.get(t) {
                        Ty::Promise(v, _) => v,
                        _ => t,
                    },
                    "NonNullable" => self.types.without_nullish(t),
                    "Readonly" => t,
                    _ => match self.type_fields(t) {
                        Some(fs) => {
                            // (an optional field's type has `undefined`, as `x?: T` does)
                            let partial = text == "Partial";
                            let fs: Vec<Field> = fs
                                .into_iter()
                                .map(|f| {
                                    let ty = if partial { self.types.optional(f.ty) } else if f.optional { self.types.without_undefined(f.ty) } else { f.ty };
                                    Field { optional: partial, ty, ..f }
                                })
                                .collect();
                            let order: Vec<Sym> = fs.iter().map(|f| f.name).collect();
                            let r = self.types.record(fs);
                            self.types.note_field_order(r, order);
                            r
                        }
                        None => {
                            let shown = self.show(t);
                            self.report(Diagnostic::new("T0010", span, format!("`{text}` takes a record or interface type, found `{shown}`")));
                            ERROR
                        }
                    },
                })
            }
            "Pick" | "Omit" => {
                if !arity(self, 2) {
                    return Some(ERROR);
                }
                let (t, k) = (targs[0], targs[1]);
                if t == ERROR || k == ERROR {
                    return Some(ERROR);
                }
                let keys: Vec<Sym> = self.flat_members(k).into_iter().filter_map(|m| match *self.types.get(m) {
                    Ty::StrLit(s) => Some(s),
                    _ => None,
                }).collect();
                let Some(fs) = self.type_fields(t) else {
                    let shown = self.show(t);
                    self.report(Diagnostic::new("T0010", span, format!("`{text}` takes a record or interface type, found `{shown}`")));
                    return Some(ERROR);
                };
                let keep = text == "Pick";
                let fs: Vec<Field> = fs.into_iter().filter(|f| keys.contains(&f.name) == keep).collect();
                let order: Vec<Sym> = fs.iter().map(|f| f.name).collect();
                let r = self.types.record(fs);
                self.types.note_field_order(r, order);
                Some(r)
            }
            // `Record<string, V>` is a string-keyed `Map` that object literals can build.
            "Record" => {
                if !arity(self, 2) {
                    return Some(ERROR);
                }
                // `Record<"low" | "high", V>`: a record with those fields
                let lits: Vec<Sym> = self.flat_members(targs[0]).into_iter().filter_map(|m| match *self.types.get(m) {
                    Ty::StrLit(s) => Some(s),
                    _ => None,
                }).collect();
                if !lits.is_empty() && lits.len() == self.flat_members(targs[0]).len() {
                    let fs: Vec<Field> = lits.iter().map(|&name| Field { name, ty: targs[1], optional: false }).collect();
                    let r = self.types.record(fs);
                    self.types.note_field_order(r, lits);
                    return Some(r);
                }
                if targs[0] != STR && targs[0] != ERROR {
                    self.report(
                        Diagnostic::new("X0030", span, "`Record` keys must be `string`")
                            .note("instead", "declare a record type `{ a: T, b: T }` for fixed keys, or use `Map<K, V>`"),
                    );
                    return Some(ERROR);
                }
                Some(self.types.intern(Ty::Map(STR, targs[1])))
            }
            "object" | "Object" => {
                self.report(
                    Diagnostic::new("X0030", name_span, format!("`{text}` is not supported"))
                        .note("instead", "declare a record type `{ field: T }`, or use `Map<K, V>` for dynamic keys"),
                );
                Some(ERROR)
            }
            "Number" | "String" | "Boolean" => {
                let lower = text.to_ascii_lowercase();
                self.report(Diagnostic::new("N0002", name_span, format!("unknown type `{text}`")).fix(Applicability::Safe, format!("use `{lower}`"), name_span, lower));
                Some(ERROR)
            }
            "symbol" | "Function" | "Date" => {
                let msg = match text {
                    "Function" => "`Function` is not supported; write the function type: `(x: T) => R`".to_string(),
                    _ => format!("`{text}` is not supported"),
                };
                self.report(Diagnostic::new("X0030", name_span, msg));
                Some(ERROR)
            }
            _ => None,
        }
    }

    // ---------------------------------------------------------------- assignability

    pub(crate) fn assignable_pub(&mut self, src: TyId, dst: TyId) -> bool {
        self.assignable(src, dst)
    }

    /// Was this `switch` found exhaustive (used for "always returns")?
    pub(crate) fn exhaustive_switch(&self, m: u32, s: StmtId) -> bool {
        self.exhaustive.contains(&(m, s))
    }

    fn assignable(&mut self, src: TyId, dst: TyId) -> bool {
        let mut assuming = Vec::new();
        self.assignable_in(src, dst, true, &mut assuming)
    }

    fn assignable_in(&mut self, src: TyId, dst: TyId, top: bool, assuming: &mut Vec<(TyId, TyId)>) -> bool {
        if src == dst || src == ERROR || dst == ERROR || src == NEVER || dst == UNKNOWN {
            return true;
        }
        if assuming.contains(&(src, dst)) {
            return true;
        }
        if dst == JS {
            // converted for JavaScript
            return self.js_convertible(src);
        }
        let (s, d) = (*self.types.get(src), *self.types.get(dst));
        match (&s, &d) {
            (Ty::Int, Ty::F64 | Ty::F32) if top => return true,
            (Ty::Int | Ty::F64 | Ty::F32 | Ty::I8 | Ty::I16 | Ty::I32 | Ty::U8 | Ty::U16 | Ty::U32, Ty::I8 | Ty::I16 | Ty::I32 | Ty::U8 | Ty::U16 | Ty::U32) if top && self.wrap_store => return true,
            (Ty::F64 | Ty::F32, Ty::Int) if top && self.float_args => return true,
            (Ty::Int | Ty::I8 | Ty::I16 | Ty::I32 | Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64, Ty::Decimal) if top => return true,
            (Ty::StrLit(_), Ty::Str) => return true,
            (Ty::Undefined, Ty::Void) => return true,
            _ => {}
        }
        if matches!(s, Ty::Rec(..)) || matches!(d, Ty::Rec(..)) {
            assuming.push((src, dst));
            let (us, ud) = (self.unfold(src), self.unfold(dst));
            let r = self.assignable_in(us, ud, top, assuming);
            assuming.pop();
            return r;
        }
        if let Ty::Union(ms) = s {
            let ms = self.types.tys(ms).to_vec();
            return ms.iter().all(|&m| self.assignable_in(m, dst, top, assuming));
        }
        if let Ty::Union(ds) = d {
            let ds = self.types.tys(ds).to_vec();
            return ds.iter().any(|&m| self.assignable_in(src, m, top, assuming));
        }
        match (s, d) {
            (Ty::Array(a), Ty::Array(b)) => self.assignable_in(a, b, false, assuming),
            (Ty::Set(a), Ty::Set(b)) => a == b,
            // Promises are read-only: covariant in the value; the errors must fit.
            (Ty::Promise(v1, e1), Ty::Promise(v2, e2)) => self.assignable_in(v1, v2, false, assuming) && (e1 == NEVER || self.assignable_in(e1, e2, false, assuming)),
            (Ty::Map(k1, v1), Ty::Map(k2, v2)) => k1 == k2 && self.assignable_in(v1, v2, false, assuming),
            (Ty::Record(a), Ty::Record(b)) => {
                let (a, b) = (self.types.fields(a).to_vec(), self.types.fields(b).to_vec());
                a.len() == b.len()
                    && a.iter().zip(&b).all(|(fa, fb)| fa.name == fb.name && fa.optional == fb.optional)
                    && a.iter().zip(&b).all(|(fa, fb)| self.assignable_in(fa.ty, fb.ty, false, assuming))
            }
            (Ty::Func(pa, ra, ta), Ty::Func(pb, rb, tb)) => {
                // A function that can throw can't stand in for one that can't.
                if ta != NEVER && !self.assignable_in(ta, tb, true, assuming) {
                    return false;
                }
                let (pa, pb) = (self.types.params(pa).to_vec(), self.types.params(pb).to_vec());
                if pa.len() > pb.len() {
                    // A callback may ignore trailing parameters, but can't demand more.
                    if pa[pb.len()..].iter().any(|p| !p.optional) {
                        return false;
                    }
                }
                for (x, y) in pa.iter().zip(&pb) {
                    if x.inout != y.inout || !self.assignable_in(y.ty, x.ty, true, assuming) {
                        return false;
                    }
                }
                rb == VOID || self.assignable_in(ra, rb, true, assuming)
            }
            (Ty::Class(..), Ty::Class(..)) => self.class_assignable(src, dst),
            (_, Ty::Interface(i, args)) => {
                let args = self.types.tys(args).to_vec();
                let fields = self.iface_fields_inst(i, &args);
                self.satisfies(src, &fields, assuming)
            }
            (Ty::Param(p), _) => match self.gparam(p).bound {
                Some(b) => self.assignable_in(b, dst, top, assuming),
                None => false,
            },
            _ => false,
        }
    }

    fn satisfies(&mut self, src: TyId, fields: &[Field], assuming: &mut Vec<(TyId, TyId)>) -> bool {
        let src_fields = match *self.types.get(src) {
            Ty::Record(fs) => self.types.fields(fs).to_vec(),
            Ty::Interface(i, args) => {
                let args = self.types.tys(args).to_vec();
                self.iface_fields_inst(i, &args)
            }
            Ty::Class(..) => self.class_as_fields(src),
            _ => return false,
        };
        fields.iter().all(|f| match src_fields.iter().find(|sf| sf.name == f.name) {
            Some(sf) => self.assignable_in(sf.ty, f.ty, false, assuming),
            None => f.optional,
        })
    }

    /// A class instance given where an interface made a record is expected (see `collect`):
    /// that interface is checked again as an interface, which classes satisfy structurally.
    fn class_to_record(&mut self, found: TyId, expected: TyId) {
        fn walk(c: &mut Checker, t: TyId, depth: u32, f: &mut dyn FnMut(&mut Checker, TyId)) {
            if depth > 8 {
                return;
            }
            f(c, t);
            let inner: Vec<TyId> = match *c.types.get(t) {
                Ty::Array(e) | Ty::Set(e) | Ty::Promise(e, _) => vec![e],
                Ty::Map(k, v) => vec![k, v],
                Ty::Union(ms) => c.types.tys(ms).to_vec(),
                Ty::Record(fs) => c.types.fields(fs).iter().map(|f| f.ty).collect(),
                Ty::Func(ps, r, _) => c.types.params(ps).iter().map(|p| p.ty).chain([r]).collect(),
                _ => Vec::new(),
            };
            for t in inner {
                walk(c, t, depth + 1, f);
            }
        }
        // (or another interface, or a record with more fields: TypeScript's structural typing)
        let mut object = false;
        walk(self, found, 0, &mut |c, t| object |= matches!(c.types.get(t), Ty::Class(..) | Ty::Interface(..) | Ty::Record(_) | Ty::Rec(..)));
        if !object {
            return;
        }
        // (the interfaces whose members are a record's fields, or whose recursive alias it is)
        let mut shapes: Vec<(u32, Vec<Sym>, u32, u32)> = Vec::new();
        for (a, info) in self.aliases.iter().enumerate() {
            if let ItemKind::Interface { members, .. } = &self.modules[info.module as usize].ast.items[info.item as usize].kind {
                let mut names: Vec<Sym> = members.iter().map(|f| f.name).collect();
                names.sort();
                shapes.push((a as u32, names, info.module, info.item));
            }
        }
        if shapes.is_empty() {
            return;
        }
        let mut hits = Vec::new();
        walk(self, expected, 0, &mut |c, t| match *c.types.get(t) {
            Ty::Rec(a, _) => hits.extend(shapes.iter().filter(|s| s.0 == a).map(|s| (s.2, s.3))),
            Ty::Record(fs) => {
                let names: Vec<Sym> = c.types.fields(fs).iter().map(|f| f.name).collect();
                hits.extend(shapes.iter().filter(|s| s.1 == names).map(|s| (s.2, s.3)));
            }
            _ => {}
        });
        self.global_hits.extend(hits);
    }

    /// Can values of these types ever be equal?
    fn comparable(&mut self, a: TyId, b: TyId) -> bool {
        if a == ERROR || b == ERROR || a == UNKNOWN || b == UNKNOWN || a == JS || b == JS {
            return true;
        }
        if self.types.is_numeric(a) && self.types.is_numeric(b) {
            return true;
        }
        let am = self.flat_members(a);
        let bm = self.flat_members(b);
        for &x in &am {
            for &y in &bm {
                if self.assignable(x, y) || self.assignable(y, x) {
                    return true;
                }
                if self.types.is_numeric(x) && self.types.is_numeric(y) {
                    return true;
                }
            }
        }
        false
    }

    fn expect_assignable(&mut self, found: TyId, expected: TyId, span: Span, context: Option<String>) -> bool {
        if self.assignable(found, expected) {
            if found == JS && self.spec.on {
                // (converted from JavaScript, where it may be any number)
                self.spec_poison(expected);
            }
            return true;
        }
        let d = self.mismatch(found, expected, span, context);
        self.report(d);
        false
    }

    fn mismatch(&mut self, found: TyId, expected: TyId, span: Span, context: Option<String>) -> Diagnostic {
        self.spec_mismatch(found, expected, span);
        self.class_to_record(found, expected);
        let (fs, es) = (self.show(found), self.show(expected));
        let mut d = Diagnostic::new("T0001", span, format!("expected `{es}`, found `{fs}`"));
        if let Some(c) = context {
            d = d.note("context", c);
        }
        let text = self.src(span).to_string();
        let exp_no_undef = self.types.without_undefined(expected);
        if found == DECIMAL && self.types.is_numeric(exp_no_undef) {
            let conv = if self.types.is_float(exp_no_undef) { "Number".to_string() } else { self.show(exp_no_undef) };
            d = d.note("why", "a `decimal` never converts to a number implicitly").fix(Applicability::Maybe, format!("convert: `{conv}({text})`"), span, format!("{conv}({text})"));
        } else if exp_no_undef == DECIMAL && self.types.is_float(found) {
            d = d.note("why", "a float is binary: `0.1` isn't exactly a tenth, so it never becomes a `decimal` implicitly").fix(Applicability::Maybe, format!("convert (its shortest decimal form): `decimal({text})`"), span, format!("decimal({text})"));
        } else if found == BIGINT && self.types.is_numeric(exp_no_undef) {
            let conv = if self.types.is_float(exp_no_undef) { "Number".to_string() } else { self.show(exp_no_undef) };
            let how = if self.types.is_float(exp_no_undef) { "the nearest number" } else { "traps if out of range" };
            d = d.note("why", "a `bigint` never converts to a number implicitly").fix(Applicability::Maybe, format!("convert ({how}): `{conv}({text})`"), span, format!("{conv}({text})"));
        } else if exp_no_undef == BIGINT && self.types.is_int(found) {
            let lit = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit() || b == b'_') || text.starts_with("0x");
            d = if lit { d.fix(Applicability::Safe, format!("make it a bigint literal: `{text}n`"), span, format!("{text}n")) } else { d.fix(Applicability::Maybe, format!("convert: `BigInt({text})`"), span, format!("BigInt({text})")) };
        } else if self.types.is_float(found) && self.types.is_int(exp_no_undef) {
            d = d
                .note("why", "floats never convert to integers implicitly")
                .fix(Applicability::Maybe, format!("round toward zero: `Math.trunc({text})`"), span, format!("Math.trunc({text})"))
                .fix(Applicability::Maybe, format!("round to nearest: `Math.round({text})`"), span, format!("Math.round({text})"));
        } else if self.types.is_int(found) && self.types.is_int(exp_no_undef) && found != exp_no_undef {
            let target = self.show(exp_no_undef);
            d = d
                .note("why", "integer types don't mix implicitly")
                .fix(Applicability::Maybe, format!("convert (traps if out of range): `{target}({text})`"), span, format!("{target}({text})"));
        } else if self.types.has_undefined(found) {
            let inner = self.types.without_undefined(found);
            if self.assignable(inner, expected) {
                d = d
                    .note("why", "the value may be `undefined`")
                    .fix(Applicability::Placeholder, "provide a default: `?? <default>`", span.empty_at_end(), " ?? <default>")
                    .fix(Applicability::Maybe, "assert it is defined (traps if not): `!`", span.empty_at_end(), "!");
            }
        } else if let (Ty::Record(_), Some((c, _))) = {
            let e = self.types.without_undefined(expected);
            (*self.types.get(found), self.class_of(e))
        } {
            let cls = self.class_names[c as usize].clone();
            d = d.note("why", format!("`{cls}` is a class: instances are created with `new {cls}(...)`, not object literals"));
        } else if let (Ty::Record(ff), Ty::Record(ef)) = {
            let unfolded = self.unfold(expected);
            (*self.types.get(found), *self.types.get(unfolded))
        } {
            let (ff, ef) = (self.types.fields(ff).to_vec(), self.types.fields(ef).to_vec());
            let missing: Vec<String> = ef.iter().filter(|e| !ff.iter().any(|f| f.name == e.name)).map(|e| self.name(e.name).to_string()).collect();
            let extra: Vec<String> = ff.iter().filter(|f| !ef.iter().any(|e| e.name == f.name)).map(|f| self.name(f.name).to_string()).collect();
            if !missing.is_empty() {
                d = d.note("missing fields", missing.join(", "));
            }
            if !extra.is_empty() {
                d = d.note("extra fields", extra.join(", "));
            }
        }
        d
    }

    // ---------------------------------------------------------------- locals

    fn fcx(&mut self) -> &mut FnCtx {
        self.fcx.last_mut().expect("no function context")
    }

    fn push_scope(&mut self) {
        self.fcx().scopes.push(Vec::new());
    }

    fn pop_scope(&mut self) {
        self.fcx().scopes.pop();
    }

    fn declare(&mut self, name: Sym, ty: TyId, kind: LocalKind, span: Span, kw_span: Option<Span>, promotable: bool) -> LocalId {
        let fcx = self.fcx.last_mut().unwrap();
        let frame = fcx.frames.len().saturating_sub(1);
        if let Some(scope) = fcx.scopes.last()
            && let Some(&(_, prev, _)) = scope.iter().find(|(s, id, _)| *s == name && fcx.locals[*id].frame == frame && fcx.locals[*id].span != span && fcx.locals[*id].global.is_none()) {
                let prev_span = fcx.locals[prev].span;
                let (line, _) = self.sm.get(prev_span.file).line_col(prev_span.start);
                let n = self.interner.get(name).to_string();
                self.diags.push(Diagnostic::new("N0003", span, format!("`{n}` is already declared in this scope")).note("previous", format!("line {line}")));
            }
        let fcx = self.fcx.last_mut().unwrap();
        fcx.locals.push(Local { name, ty, kind, span, kw_span, promotable, frame, global: None, sites: Vec::new() });
        let id = fcx.locals.len() - 1;
        fcx.scopes.last_mut().unwrap().push((name, id, None));
        id
    }

    /// Finds a local and its current (narrowed) type.
    fn lookup(&self, name: Sym) -> Option<(LocalId, TyId)> {
        let fcx = self.fcx.last()?;
        let cur_frame = fcx.frames.len().saturating_sub(1);
        for scope in fcx.scopes.iter().rev() {
            for &(s, id, narrowed) in scope.iter().rev() {
                if s == name {
                    let local = &fcx.locals[id];
                    // Narrowing of a mutable variable from an enclosing function isn't trusted inside a closure.
                    let ty = match narrowed {
                        Some(t) if !(local.frame < cur_frame && local.kind == LocalKind::Let) => t,
                        _ => local.ty,
                    };
                    return Some((id, ty));
                }
            }
        }
        None
    }

    /// The tuple type `[a, b, ...]` (up to 4 elements).
    pub(crate) fn tuple_ty(&mut self, elems: &[TyId]) -> TyId {
        let fields: Vec<Field> = elems.iter().zip(self.syms.digits).map(|(&ty, name)| Field { name, ty, optional: false }).collect();
        let order: Vec<Sym> = fields.iter().map(|f| f.name).collect();
        let t = self.types.record(fields);
        self.types.note_field_order(t, order);
        t
    }

    /// Would TypeScript share this value between two variables (objects, arrays, maps, sets)?
    /// A copy of a value of `ty` would be shared in TypeScript but is its own in Tov: records
    /// (arrays, maps, sets and recursive aliases' boxes are shared in Tov too).
    fn shared_in_ts(&mut self, ty: TyId) -> bool {
        self.types.members(ty).into_iter().any(|m| matches!(self.types.get(m), Ty::Record(_) | Ty::Interface(..)))
    }

    /// V0120: `const b = a; b.push(x); use(a)` — TypeScript shares `a` and `b`, Tov copies.
    /// (Likewise `const row = grid[0]!`, a value put into a collection, and copies to or from
    /// class instances and module variables, V0121.)
    fn check_shared_copies(&mut self) {
        let fcx = self.fcx.last().unwrap();
        let after = |l: LocalId, pos: u32, depth: u32| fcx.mutations.iter().filter(move |&&(m, sp, d)| m == l && sp.start > pos && d >= depth);
        let read_after = |l: LocalId, sp: Span| fcx.reads.iter().any(|&(r, at)| r == l && at > sp.end);
        let mut copies = Vec::new();
        for &(copy, src, pos, depth, init) in &fcx.aliases {
            // (changing the copy, then reading the source; or changing the source where the
            // copy came from — not replacing it or other parts — then reading the copy)
            let hit = after(copy, pos, 0).find(|&&(_, sp, _)| read_after(src, sp)).map(|&(_, sp, _)| (sp, copy, src)).or_else(|| after(src, pos, depth).find(|&&(_, sp, _)| read_after(copy, sp)).map(|&(_, sp, _)| (sp, src, copy)));
            if let Some((sp, changed, other)) = hit {
                copies.push((sp, fcx.locals[changed].name, fcx.locals[other].name, fcx.locals[copy].name, init, matches!(self.types.get(fcx.locals[copy].ty), Ty::Array(_))));
            }
        }
        let mut stored = Vec::new();
        for &(container, value, pos) in &fcx.stored {
            if let Some(&(_, sp, _)) = after(value, pos, 0).find(|&&(_, sp, _)| read_after(container, sp)) {
                stored.push((sp, fcx.locals[value].name, fcx.locals[container].name));
            }
        }
        let mut heap = Vec::new();
        for &(l, pos, place) in &fcx.heap_links {
            if let Some(&(_, sp, _)) = after(l, pos, 0).next() {
                heap.push((sp, fcx.locals[l].name, place));
            }
        }
        for (sp, changed, other, copy, init, is_array) in copies {
            let (c, o, cp, sr) = (self.name(changed).to_string(), self.name(other).to_string(), self.name(copy).to_string(), self.src(init).to_string());
            let copy_note = if is_array { format!("say so explicitly: `{cp} = {sr}.slice()`") } else { "say so explicitly: build a new value".to_string() };
            // (`xs.find(f)` / `xs.at(i)`: an element, changed where it is through its index)
            let found = [".find(", ".findLast(", ".at("].iter().find_map(|m| sr.find(m).map(|at| (&sr[..at], &sr[at + 1..])));
            let one_note = match found {
                Some((xs, call)) if call.starts_with("at(") => format!("modify `{xs}[i]` directly instead of going through `{cp}`"),
                Some((xs, call)) => {
                    let index = if call.starts_with("findLast(") { call.replacen("findLast(", "findLastIndex(", 1) } else { call.replacen("find(", "findIndex(", 1) };
                    format!("find its index (`const i = {xs}.{index}`) and modify `{xs}[i]` directly")
                }
                None => format!("modify `{sr}` directly instead of going through `{cp}`"),
            };
            self.report(
                Diagnostic::new("V0120", sp, format!("this changes `{c}`, but `{o}` is a separate copy (TypeScript would change both)"))
                    .note("why", format!("`{cp}` was initialized from `{sr}`: Tov records (object literals) are values, so assignment copies"))
                    .note("if you meant a copy", copy_note)
                    .note("if you meant one value", one_note),
            );
        }
        for (sp, value, container) in stored {
            let (v, c) = (self.name(value).to_string(), self.name(container).to_string());
            self.report(
                Diagnostic::new("V0120", sp, format!("this changes `{v}`, but `{c}` holds a separate copy of it (TypeScript would change both)"))
                    .note("why", format!("`{v}` was copied into `{c}`: Tov records (object literals) are values"))
                    .note("instead", format!("finish changing `{v}` before storing it, change it through `{c}`, or make it a class instance (a reference)")),
            );
        }
        for (sp, name, place) in heap {
            let (n, p) = (self.name(name).to_string(), self.src(place).to_string());
            self.report(
                Diagnostic::new("V0121", sp, format!("this changes `{n}`, but `{p}` holds a separate copy (TypeScript would change both)"))
                    .note("why", "Tov records (object literals) are values, so assignment copies")
                    .note("instead", format!("change `{p}` directly, or make the shared value a class instance (a reference)")),
            );
        }
    }

    fn narrow(&mut self, local: LocalId, ty: TyId) {
        let fcx = self.fcx.last_mut().unwrap();
        let (name, declared) = (fcx.locals[local].name, fcx.locals[local].ty);
        let changed = fcx.closure_assigned.contains(&name) || fcx.closure_mutated.contains(&name) && !self.references_only(declared);
        let fcx = self.fcx.last_mut().unwrap();
        if changed && ty != declared {
            return;
        }
        fcx.scopes.last_mut().unwrap().push((name, local, Some(ty)));
    }

    /// Only references (class instances, promises, functions, `undefined`): changing what they
    /// point to never changes which of them a value is.
    fn references_only(&mut self, ty: TyId) -> bool {
        self.flat_members(ty).iter().all(|&m| matches!(self.types.get(m), Ty::Class(..) | Ty::Promise(..) | Ty::Func(..) | Ty::Undefined | Ty::Null))
    }

    /// Narrows a module constant (it can't be rebound, so what a test learns holds).
    fn narrow_global(&mut self, key: (u32, u32), ty: TyId) {
        let fcx = self.fcx.last_mut().unwrap();
        fcx.global_table.push((key, ty));
        let idx = fcx.global_table.len() - 1;
        fcx.scopes.last_mut().unwrap().push((GLOBAL_SYM, idx, None));
    }

    /// A module constant's narrowed type here, if a test narrowed it.
    pub(super) fn lookup_global(&self, key: (u32, u32)) -> Option<TyId> {
        let fcx = self.fcx.last()?;
        for scope in fcx.scopes.iter().rev() {
            for &(s, idx, _) in scope.iter().rev() {
                if s == GLOBAL_SYM && fcx.global_table[idx].0 == key {
                    return Some(fcx.global_table[idx].1);
                }
            }
        }
        None
    }

    /// Narrows (or with `None`, forgets narrowings under) a field path of a local.
    fn narrow_path(&mut self, local: LocalId, path: Vec<Sym>, ty: Option<TyId>) {
        let fcx = self.fcx.last_mut().unwrap();
        if ty.is_some() && fcx.closure_mutated.contains(&fcx.locals[local].name) {
            return;
        }
        fcx.path_table.push((local, path, ty));
        let idx = fcx.path_table.len() - 1;
        fcx.scopes.last_mut().unwrap().push((PATH_SYM, idx, None));
    }

    /// The narrowed type of `local.path`, if it is narrowed (and not invalidated since).
    fn lookup_path(&self, local: LocalId, path: &[Sym]) -> Option<TyId> {
        let fcx = self.fcx.last()?;
        let cur_frame = fcx.frames.len().saturating_sub(1);
        let l = &fcx.locals[local];
        if l.frame < cur_frame && l.kind == LocalKind::Let {
            return None;
        }
        for scope in fcx.scopes.iter().rev() {
            for &(s, idx, _) in scope.iter().rev() {
                if s == PATH_SYM {
                    let (pl, pp, pt) = &fcx.path_table[idx];
                    if *pl != local {
                        continue;
                    }
                    match pt {
                        None if path.starts_with(pp) => return None,
                        Some(t) if pp.as_slice() == path => return Some(*t),
                        _ => {}
                    }
                }
            }
        }
        None
    }

    fn this_expr(&mut self, e: ExprId, span: Span) -> TyId {
        match self.lookup(THIS_SYM) {
            Some((id, ty)) => {
                let key = self.local(id).span.start;
                self.rec_ident(e, IdentFact::Local(key));
                ty
            }
            None => {
                self.report(Diagnostic::new("T0802", span, "`this` is only available inside class methods and constructors"));
                ERROR
            }
        }
    }

    fn local(&self, id: LocalId) -> &Local {
        &self.fcx.last().unwrap().locals[id]
    }
}

/// A type alias, or an interface made one (see `collect`): (name, name span, parameters, body).
fn alias_item(k: &ItemKind) -> Option<(&Sym, &Span, &Vec<ast::TypeParam>, ast::TypeId)> {
    match k {
        ItemKind::TypeAlias { name, name_span, tparams, ty } => Some((name, name_span, tparams, *ty)),
        ItemKind::Interface { name, name_span, tparams, body, .. } => Some((name, name_span, tparams, *body)),
        _ => None,
    }
}

fn collect_type_refs(ast: &Ast, te: ast::TypeId, out: &mut Vec<(Option<Sym>, Sym)>) {
    match &ast.ty(te).kind {
        TypeExprKind::Named { ns, name, args, .. } => {
            out.push((ns.map(|n| n.0), *name));
            for &a in args {
                collect_type_refs(ast, a, out);
            }
        }
        TypeExprKind::Array(e) => collect_type_refs(ast, *e, out),
        TypeExprKind::Union(ms) => ms.iter().for_each(|&m| collect_type_refs(ast, m, out)),
        TypeExprKind::Record(fs) => fs.iter().for_each(|f| collect_type_refs(ast, f.ty, out)),
        TypeExprKind::Func(ps, r, th) => {
            ps.iter().filter_map(|p| p.ty).for_each(|t| collect_type_refs(ast, t, out));
            collect_type_refs(ast, *r, out);
            if let Some(t) = th {
                collect_type_refs(ast, *t, out);
            }
        }
        _ => {}
    }
}

/// Can this function body throw or pass an error on (a `throw` statement or a `try` expression
/// outside nested arrow functions)? Syntactic: decides whether `throws` needs inferring.
pub(crate) fn may_throw(ast: &Ast, s: StmtId) -> bool {
    use crate::ast::{ExprKind, StmtKind};
    fn expr(ast: &Ast, e: ExprId) -> bool {
        match &ast.expr(e).kind {
            ExprKind::Try(_) => true,
            ExprKind::Arrow(_) => false,
            ExprKind::Unary(_, x) | ExprKind::Paren(x) | ExprKind::NonNull(x) | ExprKind::Typeof(x) | ExprKind::Spread(x) | ExprKind::As(x, _) | ExprKind::Await(x) => expr(ast, *x),
            ExprKind::Binary(_, l, r) | ExprKind::Assign(_, l, r) => expr(ast, *l) || expr(ast, *r),
            ExprKind::Update { target, .. } => expr(ast, *target),
            ExprKind::Call { callee, args, .. } | ExprKind::New { callee, args, .. } => expr(ast, *callee) || args.iter().any(|a| expr(ast, a.expr)),
            ExprKind::Member { obj, .. } => expr(ast, *obj),
            ExprKind::Index { obj, index, .. } => expr(ast, *obj) || expr(ast, *index),
            ExprKind::Cond(a, b, c) => expr(ast, *a) || expr(ast, *b) || expr(ast, *c),
            ExprKind::Array(xs) | ExprKind::Template(_, xs) | ExprKind::Seq(xs) => xs.iter().any(|&x| expr(ast, x)),
            ExprKind::Object(fs) => fs.iter().any(|f| expr(ast, f.value)),
            _ => false,
        }
    }
    match &ast.stmt(s).kind {
        StmtKind::Throw(_) => true,
        StmtKind::Expr(e) | StmtKind::Return(Some(e)) => expr(ast, *e),
        StmtKind::Let { init: Some(e), .. } => expr(ast, *e),
        StmtKind::Block(ss) => ss.iter().any(|&x| may_throw(ast, x)),
        StmtKind::If(c, t, e) => expr(ast, *c) || may_throw(ast, *t) || e.map(|e| may_throw(ast, e)).unwrap_or(false),
        StmtKind::While(c, b) | StmtKind::DoWhile(b, c) => expr(ast, *c) || may_throw(ast, *b),
        StmtKind::For { init, cond, step, body } => {
            init.map(|i| may_throw(ast, i)).unwrap_or(false) || [cond, step].into_iter().flatten().any(|&e| expr(ast, e)) || may_throw(ast, *body)
        }
        StmtKind::ForOf { iter, body, .. } => expr(ast, *iter) || may_throw(ast, *body),
        StmtKind::Switch(d, cases) => expr(ast, *d) || cases.iter().any(|c| c.body.iter().any(|&x| may_throw(ast, x))),
        // A `catch` handles the block's errors, but the catch/finally bodies can throw again.
        StmtKind::Try { catch, finally, body } => {
            catch.as_ref().map(|c| may_throw(ast, c.body)).unwrap_or(false) || finally.map(|f| may_throw(ast, f)).unwrap_or(false) || (catch.is_none() && may_throw(ast, *body))
        }
        _ => false,
    }
}

/// Does any `return` in this function body (outside nested arrow functions) carry a value?
pub(crate) fn returns_value(ast: &Ast, s: StmtId) -> bool {
    use crate::ast::StmtKind;
    match &ast.stmt(s).kind {
        StmtKind::Return(v) => v.is_some(),
        StmtKind::Block(ss) => ss.iter().any(|&x| returns_value(ast, x)),
        StmtKind::If(_, t, e) => returns_value(ast, *t) || e.map(|e| returns_value(ast, e)).unwrap_or(false),
        StmtKind::While(_, b) | StmtKind::DoWhile(b, _) | StmtKind::ForOf { body: b, .. } | StmtKind::For { body: b, .. } => returns_value(ast, *b),
        StmtKind::Switch(_, cases) => cases.iter().any(|c| c.body.iter().any(|&x| returns_value(ast, x))),
        _ => false,
    }
}
