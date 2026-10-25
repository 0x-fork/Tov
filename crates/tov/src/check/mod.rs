//! Name resolution and type checking.

pub(crate) mod builtins;

/// A sized integer (`u8`, `i32`, ...) as it computes: `int` (see `expr::promoted`).
pub(crate) fn promoted_ty(t: TyId) -> TyId {
    expr::promoted(t)
}
pub(crate) mod class;
mod assigned;
mod expr;
pub mod facts;
mod objects;
pub mod rewrite;
mod spec;
pub(crate) mod stmt;

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

/// What a constructor type (`new <T>(x: number) => Queue<T>`, `typeof C`) says of the class
/// given for it: its own type parameters, its constructor's parameters, its instances' type.
#[derive(Clone)]
pub(crate) struct CtorSig {
    pub(crate) tparams: Vec<u32>,
    pub(crate) params: Vec<FnParam>,
    pub(crate) ret: TyId,
}

#[derive(Clone)]
pub(crate) struct GParam {
    pub(crate) name: Sym,
    pub(crate) bound: Option<TyId>,
    /// A parameter whose type is a constructor type stands for the class each call gives it.
    pub(crate) ctor: Option<CtorSig>,
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
    /// A closure's expected function type (for `spec::blame_fn_type` when it throws).
    exp_fn: Option<TyId>,
    /// A generator: what it yields, as declared (`Generator<T>`; `None` to infer from `yields`).
    generator: Option<Option<TyId>>,
    yields: Vec<TyId>,
}

impl Frame {
    fn new(ret: Option<TyId>, name: Option<Sym>, can_throw: bool, throws_decl: Option<TyId>) -> Frame {
        Frame { ret, ret_sites: Vec::new(), returns: Vec::new(), loops: 0, switches: 0, labels: Vec::new(), name, can_throw, throws_decl, thrown: Vec::new(), try_frames: Vec::new(), try_expr: 0, module_init: false, is_async: false, exp_fn: None, generator: None, yields: Vec::new() }
    }
}

/// Kinds of evolving bindings (see `FnCtx::evolving`).
const EV_ARRAY: u8 = 0;
const EV_LET: u8 = 1;
const EV_COLLECTION: u8 = 2;
/// `const lru = new LRU(size)` of a generic class whose type arguments the constructor's don't
/// give: what its methods are given (`lru.put(k, v)`), as TypeScript's `LRU<unknown, unknown>`
/// takes anything.
const EV_CLASS: u8 = 3;

#[derive(Default)]
struct FnCtx {
    locals: Vec<Local>,
    /// Innermost last. Each entry: (name, local, narrowed type).
    pub(crate) scopes: Vec<Vec<(Sym, LocalId, Option<TyId>)>>,
    /// Locals holding what an untyped map's `get` gave (TypeScript's `any`), by declaration:
    /// read where a value must be, they're checked to be there, as `x!`.
    loose: HashSet<u32>,
    frames: Vec<Frame>,
    tscope: Vec<(Sym, TyId)>,
    promoted: HashSet<u32>,
    pending: HashSet<u32>,
    /// `const xs = []` (no type written) whose elements an earlier check of the body found in
    /// what it's given (`xs.push(x)`, `xs[i] = x`): the element type, by declaration.
    evolved: HashMap<u32, TyId>,
    /// Such arrays this check: what they're given so far (see `evolve`), and where they are.
    /// (kind `EV_LET`: a `let x` with neither type nor value, typed by what's assigned to it;
    /// `EV_COLLECTION`: a `new Map()` / `new Set()`, given whole map and set types)
    evolving: Vec<(u32, Span, Vec<TyId>, u8)>,
    /// `const { done, value } = r`'s locals (`const done = $t.done`): (the local, the `$t` read, the
    /// field). Testing one of a discriminated union narrows `$t`, and so the others.
    field_aliases: Vec<(LocalId, ExprId, Sym)>,
    /// A non-exhaustive `switch` was reported; "missing return" would only repeat it.
    reported_nonexhaustive: bool,
    /// Narrowings of field paths (`t.left`): (local, path, narrowed type or `None` = invalidated).
    /// Scope entries named `PATH_SYM` point into this table, so they follow scope lifetimes.
    path_table: Vec<(LocalId, Vec<Sym>, Option<TyId>)>,
    /// Narrowings of module constants (`const port = process.env.PORT; if (port !== undefined)`);
    /// scope entries named `GLOBAL_SYM` point into it.
    global_table: Vec<((u32, u32), TyId)>,
    /// Names of locals assigned or changed (through a field, an element or a method call) inside a
    /// closure: never narrowed (the closure may run between the check and the use)...
    closure_mutated: HashSet<Sym>,
    /// ...except that a local holding only references (class instances, promises, functions) keeps
    /// its narrowing unless a closure assigns it: changing an object doesn't change its class.
    closure_assigned: HashSet<Sym>,
    /// Locals a closure assigns, narrowed (see `narrow`): back to their declared types at a call.
    closure_narrowed: Vec<LocalId>,
    /// `routes` handlers' request parameters and their route (`"/users/:id"`): `req.params`
    /// is typed from the route.
    route_locals: Vec<(LocalId, Sym)>,
    /// Active narrowings of paths that go through a class instance (`this.left`): any call may
    /// change them through another reference, so calls forget them.
    heap_paths: Vec<(LocalId, Vec<Sym>)>,
    /// Path narrowings kept past a call (see `invalidate_heap`): `path_table` indexes.
    asserted: HashSet<usize>,
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
    digits: [Sym; 16],
}

fn intern_syms(interner: &mut Interner) -> Syms {
    let digits = std::array::from_fn(|i| interner.intern(&i.to_string()));
    Syms { kind: interner.intern("kind"), main: interner.intern("main"), u: interner.intern("U"), number: interner.intern("number"), digits }
}

/// Checking runs in two phases. Phase A (one thread) resolves everything that forms a module's
/// interface: type aliases, interfaces, function signatures (checking bodies of functions whose
/// return type is inferred) and constants. Phase B checks the remaining function bodies and tests,
/// one module at a time on every core. The tables behind `Arc` are only written in phase A;
/// phase B workers share them read-only and put new types in a per-module overlay, so results
/// (and diagnostic text) don't depend on how modules are spread across threads.
pub struct Checker<'a> {
    /// What a field assignment just stored, when narrower than the field (see `assign`).
    assigned_field: Option<TyId>,
    /// The type wanted of the member being checked (for a generic static method as a value).
    member_exp: Option<TyId>,
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
    /// Fields written with no type (TypeScript's `any`) typed by what they're given: (class,
    /// field). A map's `get` reads its value where one must be, as `any`'s would.
    untyped_fields: HashSet<(u32, Sym)>,
    /// Checking `const x = new C(...)` with no type: type arguments nothing gives are left to the
    /// calls on `x` (`lenient_hit` when that happened; see `EV_CLASS`).
    lenient_new: Option<Span>,
    /// `any` written in a function's parameters: each a type parameter of its own the calls bind,
    /// by (module, where the name is).
    any_types: HashMap<(u32, u32), TyId>,
    lenient_hit: bool,
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
    /// Functions whose result is being inferred, innermost last; a guessed result (`ret_probe`)
    /// only answers a function's calls to itself.
    sig_stack: Vec<(u32, u32)>,
    sig_recursed: HashSet<(u32, u32)>,
    /// The body inferred with a guessed result for its calls to itself: returns that fail
    /// under the guess don't count toward the next guess.
    ret_probe: Option<(u32, StmtId)>,
    /// The same for a local arrow that calls itself (by its span).
    arrow_probe: Option<Span>,
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
    /// Inferred records' `int` fields from integer literals (`{ n: 0 }`): their `lit_key`s.
    lit_fields: HashMap<(TyId, Sym), Vec<(u32, u32)>>,
    /// Function types written without `throws`: the `fn_type_key`s of the annotations that gave
    /// each (frozen phase-A base, then this checker's own).
    fn_sites_base: Arc<HashMap<TyId, Vec<(u32, u32)>>>,
    fn_sites: HashMap<TyId, Vec<(u32, u32)>>,
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
    /// Checking an object literal written `as const` (see `object`).
    as_const: bool,
    /// The place just made is a map's key (`m[k]`, which may be missing: `m[k] ??= v`).
    map_place: bool,
    /// Index reads whose value may be missing, `undefined` (see `stmt::tested`): (module, read).
    absent_reads: HashSet<(u32, ExprId)>,
    /// What the driver rewrites before checking again (see `rewrite`).
    pub(crate) rewrites: rewrite::Rewrites,
    /// The next expression checked may be absent (see `expr_inner`).
    want_optional: bool,
    /// Checking a built-in method's arguments: an integer parameter takes a number (truncated).
    float_args: bool,
    /// What callbacks given to built-in methods throw (`xs.map((x) => parse(x))`), as their
    /// arrows are checked: the method call passes it on.
    cb_throws: Vec<TyId>,
    /// Arrows that narrow their first parameter, as a type guard (`(x): x is T => ...`, or one
    /// whose body does exactly that: `(x) => x !== undefined`): (module, arrow) → what passes.
    arrow_guards: crate::hash::FxMap<(u32, ExprId), TyId>,
    /// The next arrow checked is a test's body (it may throw: the test fails).
    test_callback: bool,
    /// Record types whose values JavaScript would show to be shared (changed in place, or
    /// compared by identity): objects on the heap, every reference seeing every change. Records
    /// never changed or compared stay values, which nothing can tell apart (see `find_objects`).
    pub(crate) objects: HashSet<TyId>,
    /// Record types changed in place: objects unless nothing shares their values (see
    /// `objects.rs`).
    mutated: HashSet<TyId>,
    /// Field names changed through a type parameter (`o.x = v`, `o: T`): every record type with
    /// one is an object. And the field names of generic objects, whose instances are too.
    pub(crate) object_fields: HashSet<Sym>,
    pub(crate) object_shapes: HashSet<Vec<Sym>>,
}

/// `threads`: phase-B worker count (default: all cores). Results don't depend on it.
fn new_checker<'a>(modules: &'a [Module], interner: &'a Interner, sm: &'a SourceMap, syms: Syms, spec: Arc<spec::Spec>) -> Checker<'a> {
    Checker {
        assigned_field: None,
        member_exp: None,
        modules,
        interner,
        sm,
        types: {
            let mut t = Types::default();
            t.set_tuple_syms(syms.digits.to_vec());
            t
        },
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
        untyped_fields: HashSet::default(),
        lenient_new: None,
        any_types: HashMap::default(),
        lenient_hit: false,
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
        sig_stack: Vec::new(),
        sig_recursed: HashSet::default(),
        ret_probe: None,
        arrow_probe: None,
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
        lit_fields: HashMap::default(),
        fn_sites_base: Arc::default(),
        fn_sites: HashMap::default(),
        spec_flow: Vec::new(),
        spec_srcs_base: Arc::default(),
        spec_srcs: HashMap::default(),
        spec_fields_base: Arc::default(),
        spec_fields: HashMap::default(),
        call_spec: None,
        wrap_store: false,
        as_const: false,
        map_place: false,
        absent_reads: HashSet::default(),
        rewrites: rewrite::Rewrites::default(),
        want_optional: false,
        float_args: false,
        cb_throws: Vec::new(),
        arrow_guards: Default::default(),
        test_callback: false,
        objects: HashSet::default(),
        mutated: HashSet::default(),
        object_fields: HashSet::default(),
        object_shapes: HashSet::default(),
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
        c.find_objects();
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
    c.fn_sites_base = Arc::new(std::mem::take(&mut c.fn_sites));
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
            assigned_field: None,
            member_exp: None,
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
            untyped_fields: self.untyped_fields.clone(),
            lenient_new: None,
            any_types: self.any_types.clone(),
            lenient_hit: false,
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
            sig_stack: Vec::new(),
            sig_recursed: HashSet::default(),
            ret_probe: None,
            arrow_probe: None,
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
            lit_fields: HashMap::default(),
            fn_sites_base: self.fn_sites_base.clone(),
            fn_sites: HashMap::default(),
            spec_flow: Vec::new(),
            spec_srcs_base: self.spec_srcs_base.clone(),
            spec_srcs: HashMap::default(),
            spec_fields_base: self.spec_fields_base.clone(),
            spec_fields: HashMap::default(),
            call_spec: None,
            wrap_store: false,
            as_const: false,
        map_place: false,
        absent_reads: HashSet::default(),
            rewrites: rewrite::Rewrites::default(),
            want_optional: false,
            float_args: false,
            cb_throws: Vec::new(),
        arrow_guards: Default::default(),
        test_callback: false,
        objects: HashSet::default(),
        mutated: HashSet::default(),
        object_fields: HashSet::default(),
        object_shapes: HashSet::default(),
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
        self.lit_fields.clear();
        self.fn_sites.clear();
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
            f.calls.insert(e, CallFact { callee, targs: Vec::new(), params: Vec::new(), rest: None, ret: ERROR, own_ret: ERROR });
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
        let shown = Display {
            types: &self.types,
            interner: self.interner,
            param_names: (&self.param_names_base, &self.param_names),
            rec_names: &self.alias_names,
            iface_names: &self.iface_names,
            class_names: &self.class_names,
            module_names: &self.module_names,
        }
        .show(ty);
        // (names a module made: a namespace's `N$$x` as `N.x`, a local class's `C$$3` as `C`)
        if !shown.contains("$$") {
            return shown;
        }
        let mut out = String::new();
        let mut rest = shown.as_str();
        while let Some(i) = rest.find("$$") {
            out.push_str(&rest[..i]);
            let after = &rest[i + 2..];
            let digits = after.chars().take_while(|c| c.is_ascii_digit()).count();
            if digits > 0 && !after[digits..].starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                rest = &after[digits..];
            } else {
                out.push('.');
                rest = after;
            }
        }
        out.push_str(rest);
        out
    }

    /// The type of a parameter with a default value and no type (`b = CURVE.P`): the default's,
    /// widened, if it checks on its own (outside the function: not naming other parameters).
    fn default_param_type(&mut self, name_span: Span) -> Option<TyId> {
        let e = *self.ast().param_defaults.get(&name_span.start)?;
        let saved = std::mem::take(&mut self.diags);
        self.fcx.push(FnCtx::default());
        self.fcx.last_mut().unwrap().scopes.push(Vec::new());
        let t = self.expr(e, None);
        self.fcx.pop();
        let failed = !self.diags.is_empty();
        self.diags = saved;
        let t = self.types.widen(t);
        (!failed && t != ERROR).then_some(t)
    }

    /// `var IUB = { a: 0.27, c: 0.12 }` given where a dictionary goes (`{ [key: string]: number }`):
    /// the variable is checked again as a map.
    fn dict_hit(&mut self, found: TyId, expected: TyId, span: Span) {
        let expected = self.types.without_undefined(expected);
        if !matches!(self.types.get(found), Ty::Record(_)) || !matches!(*self.types.get(expected), Ty::Map(k, _) if k == STR) {
            return;
        }
        let text = self.src(span).trim();
        let Some(sym) = self.interner.lookup(text) else { return };
        let m = self.cur;
        // (a local: by where it's declared)
        if let Some(f) = self.fcx.last()
            && let Some(&(_, local, _)) = f.scopes.iter().rev().flat_map(|s| s.iter().rev()).find(|(n, ..)| *n == sym)
        {
            let at = self.local(local).span.start;
            self.global_hits.push(spec::dict_key(m, 0x8000_0000 | at));
            return;
        }
        for (ii, it) in self.modules[m as usize].ast.items.iter().enumerate() {
            if let ItemKind::Const { name, ty: None, init, .. } = &it.kind
                && *name == sym
                && matches!(self.modules[m as usize].ast.expr(*init).kind, ast::ExprKind::Object(_))
            {
                self.global_hits.push(spec::dict_key(m, ii as u32));
                return;
            }
        }
    }

    /// An object literal given where a dictionary goes, checked again as one: a map of what its
    /// fields hold.
    pub(super) fn as_dict(&mut self, init: ExprId, found: TyId) -> TyId {
        let Ty::Record(fs) = *self.types.get(found) else { return found };
        if !matches!(self.ast().expr(init).kind, ast::ExprKind::Object(_)) {
            return found;
        }
        let tys: Vec<TyId> = self.types.fields(fs).iter().map(|f| f.ty).collect();
        let v = if tys.iter().all(|&t| self.types.is_numeric(t)) {
            if tys.iter().any(|&t| self.types.is_float(t)) { F64 } else { INT }
        } else {
            let ws: Vec<TyId> = tys.iter().map(|&t| self.types.widen(t)).collect();
            self.types.union(&ws)
        };
        let map = self.types.intern(Ty::Map(STR, v));
        self.expr(init, Some(map))
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

    /// A type parameter standing for the class a constructor type is given.
    fn ctor_param(&mut self, name: Sym, sig: CtorSig) -> TyId {
        let id = self.new_gparam(name, None);
        let base = self.gparams_base.len();
        self.gparams[id as usize - base].ctor = Some(sig);
        self.types.intern(Ty::Param(id))
    }

    fn new_gparam(&mut self, name: Sym, bound: Option<TyId>) -> u32 {
        self.gparams.push(GParam { name, bound, default: None, ctor: None });
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
        // (an interface a class implements implements those it extends too)
        loop {
            let before = implemented.len();
            for module in modules.iter() {
                for item in &module.ast.items {
                    if let ItemKind::Interface { name, extends, .. } = &item.kind
                        && implemented.contains(name)
                    {
                        for &te in extends {
                            if let TypeExprKind::Named { name, .. } = &module.ast.ty(te).kind {
                                implemented.insert(*name);
                            }
                        }
                    }
                }
            }
            if implemented.len() == before {
                break;
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
                        Arc::make_mut(&mut self.class_names).push(shown_class_name(self.interner.get(cd.name)));
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
                if is_type && builtins::is_type_keyword(self.interner.get(name)) {
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

    /// Binds every module's imports, and its exports (`export { a as b }`, `export * from`,
    /// `export default`): alternately until nothing changes (a module's exports can be another's
    /// imports, passed on), then once more, reporting what's missing.
    fn resolve_imports(&mut self) {
        let count = |c: &Self| c.scopes.iter().map(|s| s.values.len() + s.types.len() + s.exported_values.len() + s.exported_types.len()).sum::<usize>();
        for _ in 0..=self.modules.len() {
            let before = count(self);
            self.bind_imports(false);
            self.apply_exports(false);
            if count(self) == before {
                break;
            }
        }
        self.bind_imports(true);
        self.apply_exports(true);
    }

    /// Exports a module declares by list or passes on from another (see `ast::ExportSpec`).
    fn apply_exports(&mut self, report: bool) {
        let modules = self.modules;
        for (mi, module) in modules.iter().enumerate() {
            let ast = &module.ast;
            let default = self.interner.lookup("default");
            for spec in &ast.exports {
                let target = match spec.from {
                    Some(item) => match module.imports.get(&item) {
                        Some(&t) => Some(t),
                        None => continue,
                    },
                    None => None,
                };
                if let Some(t) = target
                    && modules[t as usize].npm.is_some()
                {
                    // (a package's exports: JavaScript values)
                    for &(orig, out, span) in &spec.names {
                        let what = if Some(orig) == default { facts::NpmName::Default } else { facts::NpmName::Named(orig) };
                        self.add_export(mi, out, span, Decl::Npm(t, what), false);
                        self.add_export(mi, out, span, Decl::Npm(t, what), true);
                    }
                    if let Some((ns, span)) = spec.namespace {
                        self.add_export(mi, ns, span, Decl::Npm(t, facts::NpmName::Ns), false);
                    }
                    continue;
                }
                if let Some((ns, span)) = spec.namespace
                    && let Some(t) = target
                {
                    self.add_export(mi, ns, span, Decl::Ns(t), false);
                    self.add_export(mi, ns, span, Decl::Ns(t), true);
                }
                if spec.star
                    && let Some(t) = target
                {
                    let ts = self.scopes[t as usize].clone();
                    let span = ast.items[spec.from.unwrap() as usize].span;
                    for &name in ts.exported_values.iter().filter(|&&n| Some(n) != default) {
                        if let Some(&(d, _)) = ts.values.get(&name) {
                            self.add_export(mi, name, span, d, false);
                        }
                    }
                    for &name in ts.exported_types.iter().filter(|&&n| Some(n) != default) {
                        if let Some(&(d, _)) = ts.types.get(&name) {
                            self.add_export(mi, name, span, d, true);
                        }
                    }
                }
                for &(orig, out, span) in &spec.names {
                    let (value, ty) = {
                        let src = &self.scopes[target.unwrap_or(mi as u32) as usize];
                        let exported = |set: &HashSet<Sym>| target.is_none() || set.contains(&orig);
                        (
                            src.values.get(&orig).filter(|_| exported(&src.exported_values)).map(|d| d.0),
                            src.types.get(&orig).filter(|_| exported(&src.exported_types)).map(|d| d.0),
                        )
                    };
                    if let Some(v) = value {
                        self.add_export(mi, out, span, v, false);
                    }
                    if let Some(t) = ty {
                        self.add_export(mi, out, span, t, true);
                    }
                    if report && value.is_none() && ty.is_none() {
                        let n = self.interner.get(orig).to_string();
                        let d = match target {
                            Some(t) => Diagnostic::new("N0102", span, format!("`{}` has no export named `{n}`", modules[t as usize].name)),
                            None => Diagnostic::new("N0001", span, format!("unknown name `{n}`")),
                        };
                        self.done.push(d);
                    }
                }
            }
            if let (Some((name, span)), Some(default)) = (ast.default_export, default) {
                let src = &self.scopes[mi];
                let (value, ty) = (src.values.get(&name).map(|d| d.0), src.types.get(&name).map(|d| d.0));
                if let Some(v) = value {
                    self.add_export(mi, default, span, v, false);
                }
                if let Some(t) = ty {
                    self.add_export(mi, default, span, t, true);
                }
                if report && value.is_none() && ty.is_none() {
                    let n = self.interner.get(name).to_string();
                    self.done.push(Diagnostic::new("N0001", span, format!("unknown name `{n}`")));
                }
            }
        }
    }

    /// Exports `decl` from module `m` as `name` (and makes the name known there).
    fn add_export(&mut self, m: usize, name: Sym, span: Span, decl: Decl, is_type: bool) {
        let scope = &mut Arc::make_mut(&mut self.scopes)[m];
        let (table, exported) = if is_type { (&mut scope.types, &mut scope.exported_types) } else { (&mut scope.values, &mut scope.exported_values) };
        match table.get(&name) {
            Some((existing, _)) if *existing != decl => return,
            Some(_) => {}
            None => {
                table.insert(name, (decl, span));
            }
        }
        exported.insert(name);
    }

    fn bind_imports(&mut self, report: bool) {
        let modules = self.modules;
        let default_sym = self.interner.lookup("default");
        for (mi, module) in modules.iter().enumerate() {
            for (ii, item) in module.ast.items.iter().enumerate() {
                let ItemKind::Import(imp) = &item.kind else { continue };
                if imp.reexport {
                    continue;
                }
                let Some(&target) = module.imports.get(&(ii as u32)) else { continue };
                let local_of = |k: usize| imp.locals.get(k).copied().unwrap_or(imp.names[k].0);
                if modules[target as usize].npm.is_some() {
                    // Anything a package exports is a JavaScript value; names are checked when
                    // the program runs (a missing export reads as `undefined`, as in JavaScript).
                    let binds = imp.namespace.map(|n| (n, facts::NpmName::Ns)).into_iter()
                        .chain(imp.default.map(|n| (n, facts::NpmName::Default)))
                        .chain(imp.names.iter().enumerate().map(|(k, &(s, sp))| ((local_of(k), sp), if Some(s) == default_sym { facts::NpmName::Default } else { facts::NpmName::Named(s) })));
                    for ((name, span), what) in binds {
                        self.bind_import(mi, name, span, Decl::Npm(target, what), false, report);
                        self.bind_import(mi, name, span, Decl::Npm(target, what), true, report);
                    }
                    continue;
                }
                // `import Big from "big.js"`, a package Tov has natively: its main export
                let shim = modules[target as usize].std.then(|| crate::driver::NPM_SHIMS.iter().find(|s| s.0 == modules[target as usize].name)).flatten();
                if let (Some((name, span)), Some(s)) = (imp.default, shim) {
                    let ts = &self.scopes[target as usize];
                    let main = self.interner.lookup(s.2);
                    let value = main.and_then(|m| ts.values.get(&m)).map(|d| d.0);
                    let ty = main.and_then(|m| ts.types.get(&m)).map(|d| d.0);
                    if let Some(v) = value {
                        self.bind_import(mi, name, span, v, false, report);
                    }
                    if let Some(t) = ty {
                        self.bind_import(mi, name, span, t, true, report);
                    }
                } else if let Some((name, span)) = imp.default
                    && modules[target as usize].std
                    && modules[target as usize].name.starts_with("node:")
                {
                    // `import path from "node:path"`: a built-in module's default is the module
                    self.bind_import(mi, name, span, Decl::Ns(target), false, report);
                    self.bind_import(mi, name, span, Decl::Ns(target), true, report);
                } else if let Some((name, span)) = imp.default {
                    // the module's `export default`
                    let ts = &self.scopes[target as usize];
                    let value = default_sym.filter(|d| ts.exported_values.contains(d)).and_then(|d| ts.values.get(&d)).map(|d| d.0);
                    let ty = default_sym.filter(|d| ts.exported_types.contains(d)).and_then(|d| ts.types.get(&d)).map(|d| d.0);
                    if let Some(v) = value {
                        self.bind_import(mi, name, span, v, false, report);
                    }
                    if let Some(t) = ty {
                        self.bind_import(mi, name, span, t, true, report);
                    }
                    if report && value.is_none() && ty.is_none() {
                        let target_name = &modules[target as usize].name;
                        self.done.push(Diagnostic::new("N0102", span, format!("`{target_name}` has no default export")).note("instead", "import its named exports: `import { name } from ...`"));
                    }
                }
                if let Some((ns, span)) = imp.namespace {
                    self.bind_import(mi, ns, span, Decl::Ns(target), false, report);
                    self.bind_import(mi, ns, span, Decl::Ns(target), true, report);
                    continue;
                }
                for (k, &(name, span)) in imp.names.iter().enumerate() {
                    let local = local_of(k);
                    let ts = &self.scopes[target as usize];
                    let value = ts.values.get(&name).filter(|_| ts.exported_values.contains(&name)).map(|d| d.0);
                    let ty = ts.types.get(&name).filter(|_| ts.exported_types.contains(&name)).map(|d| d.0);
                    if value.is_none() && ty.is_none() {
                        if !report {
                            continue;
                        }
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
                        self.bind_import(mi, local, span, v, false, report);
                    }
                    if let Some(t) = ty {
                        self.bind_import(mi, local, span, t, true, report);
                    }
                }
            }
        }
    }

    fn bind_import(&mut self, m: usize, name: Sym, span: Span, decl: Decl, is_type: bool, report: bool) {
        let scope = &mut Arc::make_mut(&mut self.scopes)[m];
        let table = if is_type { &mut scope.types } else { &mut scope.values };
        if let Some((existing, prev)) = table.get(&name) {
            if *existing != decl && report {
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
        // (an imported module's top-level statements run before the program's, once, as
        // JavaScript runs a module when it's first imported; not those that wait)
        if let Some(span) = ast.script_span
            && !self.modules[m as usize].entry
            && ast.script.is_some_and(|si| matches!(&ast.items[si as usize].kind, ItemKind::Function(f) if f.is_async))
        {
            self.done.push(
                Diagnostic::new("P0201", span, "an imported module's top level can't `await`")
                    .note("instead", "export an async function and call it from the entry file"),
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
        let mut evolved = HashMap::default();
        let saved = std::mem::take(&mut self.diags);
        for _round in 0..4 {
            self.diags.clear();
            self.fcx.push(FnCtx { closure_mutated: closure_mutated.clone(), closure_assigned: closure_assigned.clone(), test_body: true, evolved: std::mem::take(&mut evolved), ..Default::default() });
            self.fcx.last_mut().unwrap().scopes.push(Vec::new());
            let t = self.expr(body, Some(expected));
            if !self.assignable(t, expected) {
                let span = self.ast().expr(body).span;
                let msg = format!("a test body must be `() => {{ ... }}`, found `{}`", self.show(t));
                self.report(Diagnostic::new("T0001", span, msg));
            }
            let fcx = self.fcx.pop().unwrap();
            evolved = fcx.evolved;
            if !self.settle_evolving(&mut evolved, fcx.evolving) {
                break;
            }
        }
        let produced = std::mem::replace(&mut self.diags, saved);
        self.diags.extend(produced);
        let produced = std::mem::take(&mut self.diags);
        self.done.extend(produced);
    }

    /// After a check of a body: the element types its untyped empty arrays were given join
    /// `evolved` (true when any did: the body is checked again with them); one given nothing
    /// is reported, as TypeScript can't type it either.
    fn settle_evolving(&mut self, evolved: &mut HashMap<u32, TyId>, evolving: Vec<(u32, Span, Vec<TyId>, u8)>) -> bool {
        let mut grew = false;
        for (key, span, given, kind) in evolving {
            let whole = kind == EV_LET;
            if kind == EV_CLASS {
                // (each type argument: what the calls gave it)
                let mut t = None;
                for &g in &given {
                    if let Ty::Class(c, args) = *self.types.get(g) {
                        let args = self.types.tys(args).to_vec();
                        t = match t {
                            None => Some((c, args.iter().map(|&a| vec![a]).collect::<Vec<_>>())),
                            Some((c0, mut acc)) => {
                                for (k, &a) in args.iter().enumerate() {
                                    acc[k].push(a);
                                }
                                Some((c0, acc))
                            }
                        };
                    }
                }
                let Some((c, acc)) = t else { continue };
                let mut args = Vec::new();
                for ts in acc {
                    let ts: Vec<TyId> = ts.into_iter().filter(|&t| t != ERROR).collect();
                    if ts.is_empty() {
                        break;
                    }
                    args.push(self.types.union(&ts));
                }
                if args.len() != self.classes[c as usize].params.len() {
                    let n = self.src(span).to_string();
                    self.report(Diagnostic::new("T0305", span, format!("can't infer the type arguments of `{n}`")).note("instead", "pass them explicitly: `new C<K, V>(...)`"));
                    continue;
                }
                let t = self.types.class(c, &args);
                if evolved.insert(key, t) != Some(t) {
                    grew = true;
                }
                continue;
            }
            if kind == EV_COLLECTION {
                // (a map's keys and values, a set's elements: what each `set`/`add` gave it)
                let (mut ks, mut vs, mut es) = (Vec::new(), Vec::new(), Vec::new());
                for &g in &given {
                    match *self.types.get(g) {
                        Ty::Map(k, v) => {
                            ks.push(k);
                            vs.push(v);
                        }
                        Ty::Set(e) => es.push(e),
                        _ => {}
                    }
                }
                let t = if !ks.is_empty() {
                    let (k, v) = (self.types.union(&ks), self.types.union(&vs));
                    self.types.intern(Ty::Map(k, v))
                } else if !es.is_empty() {
                    let e = self.types.union(&es);
                    self.types.intern(Ty::Set(e))
                } else {
                    let what = if self.src(span).contains("Set") { "the element type of this set" } else { "the key and value types of this map" };
                    self.report(Diagnostic::new("T0301", span, format!("can't infer {what}")).note("instead", "`new Map<string, int>()`, or annotate the binding"));
                    continue;
                };
                if evolved.insert(key, t) != Some(t) {
                    grew = true;
                }
                continue;
            }
            if given.is_empty() && whole {
                let n = self.src(span).to_string();
                self.report(
                    Diagnostic::new("T0301", span, format!("`{n}` needs a type annotation or an initial value"))
                        .fix(Applicability::Placeholder, format!("annotate `{n}: T`"), span.empty_at_end(), ": T"),
                );
                continue;
            }
            if given.is_empty() {
                self.report(
                    Diagnostic::new("T0301", span, "can't infer the element type of an empty array")
                        .note("instead", "annotate the binding: `const xs: T[] = []`"),
                );
                continue;
            }
            let t = self.types.union(&given);
            if evolved.insert(key, t) != Some(t) {
                grew = true;
            }
        }
        grew
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
                // (checked again while its untyped empty arrays find their elements, see
                // `settle_evolving`)
                let mut evolved = HashMap::default();
                let saved = std::mem::take(&mut c.diags);
                let mut out = (ERROR, false);
                for _round in 0..4 {
                    c.diags.clear();
                    c.fcx.push(FnCtx { evolved: std::mem::take(&mut evolved), ..Default::default() });
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
                            // (`const u = db[id]` tested in the script: may be `undefined`, see `stmt::tested`)
                            if matches!(c.ast().expr(init).kind, ast::ExprKind::Index { .. })
                                && let Some(si) = c.ast().script
                                && let ItemKind::Function(f) = &c.ast().items[si as usize].kind
                                && let ast::StmtKind::Block(body) = &c.ast().stmt(f.body).kind
                                && stmt::tested(c.ast(), *name, body)
                            {
                                c.want_optional = true;
                            }
                            let mut found = c.expr(init, None);
                            if c.spec.float_globals.contains(&spec::dict_key(m, ii)) {
                                found = c.as_dict(init, found);
                            }
                            let t = c.check_inferred_binding(found, c.ast().expr(init).span);
                            // `let s = ""` holds any string later, as in TypeScript; `let x = 0`
                            // that's given a float is an `f64`
                            let t = if mutable { c.types.widen(t) } else { t };
                            if t == INT && c.spec.float_globals.contains(&(m, ii)) { F64 } else { t }
                        }
                    };
                    let throws = !c.fcx.last().unwrap().frames[0].thrown.is_empty();
                    let fcx = c.fcx.pop().unwrap();
                    out = (t, throws);
                    evolved = fcx.evolved;
                    if !c.settle_evolving(&mut evolved, fcx.evolving) {
                        break;
                    }
                }
                let produced = std::mem::replace(&mut c.diags, saved);
                c.diags.extend(produced);
                out
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
            // signature, assuming it may throw. Without a declared result, only the function's
            // own calls get the guessed one.
            if f.ret.is_none() && self.sig_stack.last() != Some(&(m, ii)) {
                return None;
            }
            self.sig_recursed.insert((m, ii));
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
                // (`x: any`: a type parameter of its own, bound by each call to what it's given, as
                // TypeScript's `any` takes anything; `xs: any[]` an array of one)
                let ts = c.ts_module();
                let body_span = c.ast().stmt(f.body).span;
                for p in f.params.iter().filter(|_| ts) {
                    let mut anys = Vec::new();
                    if let Some(t) = p.ty {
                        // (unless the body uses it as JavaScript does, a property of it read or
                        // set, it called: then an `any`, checked as it's used)
                        if !c.used_dynamically(t, body_span, p.name) {
                            c.any_uses(t, &mut anys);
                        }
                    }
                    for at in anys {
                        let name = c.interner.lookup("any").unwrap_or(p.name);
                        let id = c.new_gparam(name, None);
                        let ty = c.types.intern(Ty::Param(id));
                        tparams.push(id);
                        let cur = c.cur;
                        c.any_types.insert((cur, at), ty);
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
                        None if let Some(t) = c.default_param_type(p.span) => t,
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
                // (`: any` as the result: what the body returns, as if it weren't written)
                let ret = f.ret.filter(|&t| !c.ts_module() || !matches!(&c.ast().ty(t).kind, TypeExprKind::Named { ns: None, name, args, .. } if args.is_empty() && c.name(*name) == "any")).map(|t| c.resolve_type(t, &tscope));
                // An async function's body returns the `T` of its `Promise<T>`.
                let ret = match ret {
                    Some(r) if f.is_async && !f.is_gen => Some(c.async_inner(r, f.ret.map(|te| c.ast().ty(te).span).unwrap())),
                    r => r,
                };
                let throws = f.throws.map(|t| c.resolve_type(t, &tscope));
                if let Some(t) = throws {
                    c.check_throws_type(t, f.throws.map(|te| c.ast().ty(te).span).unwrap());
                }
                // (a class passed as a value, `Q: new <T>() => Queue<T>`: a type parameter too,
                // which each call binds to the class it gives)
                for p in &params {
                    let t = c.types.without_undefined(p.ty);
                    if let Ty::Param(id) = *c.types.get(t)
                        && c.gparam(id).ctor.is_some()
                        && !tparams.contains(&id)
                    {
                        tparams.push(id);
                    }
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
        // (a generator's result comes from its `yield`s)
        let rv = f.is_gen || returns_value(&self.modules[m as usize].ast, f.body);
        let ret = match ret {
            Some(r) if throws != UNKNOWN => r,
            // No `return <value>` anywhere: `void`, and the body can wait for phase B.
            None if !rv && throws != UNKNOWN => VOID,
            None if !f.is_async && !f.is_gen => self.infer_recursive(m, ii, f, &tparams, &params, &param_names, &tscope, &mut throws),
            declared => {
                // Infer from the body (the return type, the errors, or both).
                let partial = Sig { tparams: tparams.clone(), params: params.clone(), param_names: param_names.clone(), ret: declared.unwrap_or(ERROR), throws, rest: false };
                let (inferred, errs) = self.check_body_of(m, ii, &partial, &tscope, declared.or(if rv { None } else { Some(VOID) }));
                if throws == UNKNOWN {
                    throws = errs;
                }
                if let Some(r) = declared {
                    r
                } else {
                    let inferred = if rv { inferred } else { VOID };
                inferred
                }
            }
        };
        let rest = f.params.last().map(|p| p.rest).unwrap_or(false);
        // Callers of an async function get a promise; its errors reject the promise. (An async
        // generator's errors reject the promise its `next()` gives.)
        let (ret, throws) = if f.is_async && f.is_gen { (ret, NEVER) } else if f.is_async { (self.types.promise(ret, throws), NEVER) } else { (ret, throws) };
        let sig = Sig { tparams, params, param_names, ret, throws, rest };
        self.sig_in_progress.remove(&(m, ii));
        Arc::make_mut(&mut self.sigs).insert((m, ii), sig.clone());
        Some(sig)
    }

    /// Infers a function's result from its body. Calls to itself first take the result to be
    /// `never`, then what the returns gave, until the two agree (as `fact(n - 1) * n` does once
    /// `return 1` says the result is a number).
    #[allow(clippy::too_many_arguments)]
    fn infer_recursive(&mut self, m: u32, ii: u32, f: &ast::FnDecl, tparams: &[u32], params: &[FnParam], param_names: &[Sym], tscope: &[(Sym, TyId)], throws: &mut TyId) -> TyId {
        let rest = f.params.last().map(|p| p.rest).unwrap_or(false);
        let guessed = if *throws == UNKNOWN { self.error_class() } else { *throws };
        let partial = Sig { tparams: tparams.to_vec(), params: params.to_vec(), param_names: param_names.to_vec(), ret: ERROR, throws: *throws, rest: false };
        let mark = self.done.len();
        let mut guess = NEVER;
        let mut result = ERROR;
        let mut settled = false;
        for round in 0..4 {
            self.sig_provisional.insert((m, ii), Sig { tparams: tparams.to_vec(), params: params.to_vec(), param_names: param_names.to_vec(), ret: guess, throws: guessed, rest });
            self.sig_recursed.remove(&(m, ii));
            self.sig_stack.push((m, ii));
            let saved = self.ret_probe.replace((m, f.body));
            let (inferred, errs) = self.check_body_of(m, ii, &partial, tscope, None);
            self.ret_probe = saved;
            self.sig_stack.pop();
            if *throws == UNKNOWN || round > 0 {
                *throws = errs;
            }
            result = inferred;
            if !self.sig_recursed.contains(&(m, ii)) || (round > 0 && inferred == guess && self.done.len() == mark) {
                settled = true;
                break;
            }
            if round == 3 {
                break;
            }
            if inferred == ERROR || inferred == NEVER || inferred == VOID {
                self.done.truncate(mark);
                let n = self.name(f.name).to_string();
                self.report(Diagnostic::new("T0303", f.name_span, format!("`{n}` calls itself, and its result can't be told from the returns that don't, so it needs an explicit return type")));
                result = ERROR;
                break;
            }
            self.done.truncate(mark);
            guess = inferred;
        }
        // (`createTree(d)` returning `{ left: createTree(d - 1), ... }`: each guess nests the
        // last one, so the result is a recursive type of its own, as TypeScript's would be)
        if !settled
            && result != ERROR
            && let Some(r) = self.infer_rec_result(m, ii, f, &partial, tparams, params, param_names, tscope, guessed, rest, mark, throws)
        {
            result = r;
        }
        self.sig_provisional.remove(&(m, ii));
        result
    }

    /// `infer_recursive`'s result as a recursive type `R` (a hidden alias): the body checked
    /// with its calls to itself giving `R`, `R` then what the body returns; checked again with it.
    #[allow(clippy::too_many_arguments)]
    fn infer_rec_result(&mut self, m: u32, ii: u32, f: &ast::FnDecl, partial: &Sig, tparams: &[u32], params: &[FnParam], param_names: &[Sym], tscope: &[(Sym, TyId)], guessed: TyId, rest: bool, mark: usize, throws: &mut TyId) -> Option<TyId> {
        let saved_done = self.done.split_off(mark.min(self.done.len()));
        let a = self.aliases.len() as u32;
        Arc::make_mut(&mut self.aliases).push(AliasInfo { module: m, item: u32::MAX, params: Vec::new(), recursive: true, body: Some(ERROR), resolving: false });
        let n = format!("ReturnType<typeof {}>", self.name(f.name));
        Arc::make_mut(&mut self.alias_names).push(n);
        let r = self.types.rec(a, &[]);
        for round in 0..2 {
            self.sig_provisional.insert((m, ii), Sig { tparams: tparams.to_vec(), params: params.to_vec(), param_names: param_names.to_vec(), ret: r, throws: guessed, rest });
            self.sig_stack.push((m, ii));
            let saved = self.ret_probe.replace((m, f.body));
            let (inferred, errs) = self.check_body_of(m, ii, partial, tscope, None);
            self.ret_probe = saved;
            self.sig_stack.pop();
            let failed = self.done.len() != mark || inferred == ERROR || (round == 0 && !self.contains_ty(inferred, r, 0));
            if failed {
                self.done.truncate(mark);
                self.done.extend(saved_done);
                return None;
            }
            *throws = errs;
            if round == 0 {
                Arc::make_mut(&mut self.aliases)[a as usize].body = Some(inferred);
            }
        }
        Some(r)
    }

    /// `t` appears in `ty` (through records, unions, arrays, maps and sets).
    fn contains_ty(&self, ty: TyId, t: TyId, depth: u32) -> bool {
        if ty == t {
            return true;
        }
        if depth > 16 {
            return false;
        }
        match *self.types.get(ty) {
            Ty::Array(e) | Ty::Set(e) => self.contains_ty(e, t, depth + 1),
            Ty::Map(k, v) => self.contains_ty(k, t, depth + 1) || self.contains_ty(v, t, depth + 1),
            Ty::Union(ms) => self.types.tys(ms).iter().any(|&x| self.contains_ty(x, t, depth + 1)),
            Ty::Record(fs) => self.types.fields(fs).iter().any(|fl| self.contains_ty(fl.ty, t, depth + 1)),
            _ => false,
        }
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
        let mut evolved = HashMap::default();
        let mut result = ERROR;
        let mut thrown = NEVER;
        let saved = std::mem::take(&mut self.diags);
        for _round in 0..4 {
            self.diags.clear();
            let mut fcx = FnCtx { tscope: tscope.to_vec(), promoted: promoted.clone(), evolved: std::mem::take(&mut evolved), ..Default::default() };
            (fcx.closure_mutated, fcx.closure_assigned) = stmt::closure_mutated_stmt(&self.modules[m as usize].ast, f.body);
            fcx.scopes.push(Vec::new());
            // Without a `throws` clause the body's own `throw`s and the calls it makes decide.
            let decl = if sig.throws == UNKNOWN || (f.throws.is_none() && sig.throws == NEVER) { None } else { Some(sig.throws) };
            // A generator yields what its `Generator<T>` says (or what its `yield`s give); its
            // `return` only ends it.
            let generator = f.is_gen.then(|| match ret.map(|r| *self.types.get(r)) {
                Some(Ty::Gen(t)) if !f.is_async => Some(t),
                Some(Ty::AsyncGen(t)) if f.is_async => Some(t),
                Some(Ty::Error) => Some(ERROR),
                Some(_) => {
                    let shown = self.show(ret.unwrap());
                    self.report(Diagnostic::new("T0001", f.name_span, format!("a generator's result is a `Generator<T>`, not `{shown}`")));
                    Some(ERROR)
                }
                None => None,
            });
            let mut frame = Frame::new(if f.is_gen { None } else { ret }, Some(f.name), true, decl);
            frame.is_async = f.is_async;
            frame.generator = generator;
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
                let yields = std::mem::take(&mut fcx.frames[0].yields);
                thrown = c.types.union(&errs);
                if f.is_gen {
                    return match ret {
                        Some(r) => r,
                        None => {
                            let t = if yields.is_empty() { UNDEFINED } else { c.types.union(&yields) };
                            let t = c.types.widen(t);
                            if f.is_async { c.types.intern(Ty::AsyncGen(t)) } else { c.types.intern(Ty::Gen(t)) }
                        }
                    };
                }
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
                        if c.ret_probe == Some((m, body)) && c.sig_stack.last().is_some_and(|k| c.sig_recursed.contains(k)) {
                            all.retain(|&t| !c.types.mentions_error(t));
                        }
                        if !c.always_returns(body) && !all.is_empty() {
                            all.push(UNDEFINED);
                        }
                        if all.is_empty() { VOID } else { c.types.union(&all) }
                    }
                }
            });
                let fcx = self.fcx.pop().unwrap();
            result = r;
            // A body that throws where its signature was taken not to (it only calls something
            // that throws): its callers are checked again knowing it can.
            if thrown != NEVER && f.throws.is_none() && sig.throws == NEVER {
                self.throw_hits.push((m, f.body));
            }
            let new: Vec<u32> = fcx.pending.difference(&promoted).copied().collect();
            evolved = fcx.evolved;
            let grew = self.settle_evolving(&mut evolved, fcx.evolving);
            if new.is_empty() && !grew {
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
        let ItemKind::Interface { tparams, members, extends, .. } = &self.modules[m as usize].ast.items[ii as usize].kind else { return Vec::new() };
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
                // (`extends A, B<T>`: their members, unless it has its own of the name)
                for &te in extends {
                    let t = c.resolve_type(te, &tscope);
                    let base: Vec<Field> = match *c.types.get(t) {
                        Ty::Interface(j, args) => {
                            let args = c.types.tys(args).to_vec();
                            c.iface_fields_inst(j, &args)
                        }
                        Ty::Record(_) => c.types.fields_in_order(t),
                        Ty::Class(..) => c.class_as_fields(t),
                        Ty::Error => Vec::new(),
                        _ => {
                            let shown = c.show(t);
                            let s = c.ast().ty(te).span;
                            c.report(Diagnostic::new("T0809", s, format!("an interface extends interfaces and object types, found `{shown}`")));
                            Vec::new()
                        }
                    };
                    for f in base {
                        if seen.insert(f.name) {
                            fields.push(f);
                        }
                    }
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

    /// `A & B`: one record with the fields of each (a field in both has the narrower type, and is
    /// optional only if it is in both).
    fn intersection(&mut self, tys: &[TyId], span: Span) -> TyId {
        let mut out: Vec<Field> = Vec::new();
        let mut sites: Vec<(Sym, Vec<spec::Site>)> = Vec::new();
        for &t in tys {
            let fields = match *self.types.get(t) {
                Ty::Error => return ERROR,
                Ty::Record(_) => self.types.fields_in_order(t),
                Ty::Interface(i, args) => {
                    let args = self.types.tys(args).to_vec();
                    self.iface_fields_inst(i, &args)
                }
                _ => {
                    let shown = self.show(t);
                    self.report(
                        Diagnostic::new("X0014", span, format!("an intersection (`&`) combines record types, and `{shown}` isn't one"))
                            .note("instead", "declare a record type with all the fields"),
                    );
                    return ERROR;
                }
            };
            for f in fields {
                sites.push((f.name, self.record_field_sites(t, f.name)));
                match out.iter_mut().find(|x| x.name == f.name) {
                    Some(x) => {
                        if x.ty != f.ty && self.assignable(f.ty, x.ty) {
                            x.ty = f.ty;
                        }
                        x.optional = x.optional && f.optional;
                    }
                    None => out.push(f),
                }
            }
        }
        let order: Vec<Sym> = out.iter().map(|f| f.name).collect();
        let r = self.types.record(out);
        self.types.note_field_order(r, order);
        for (name, s) in sites {
            if !s.is_empty() {
                self.spec_fields.entry((r, name)).or_default().extend(s);
            }
        }
        r
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
                    // `typeof Shape` of a class: the class (or one extending it) as a value
                    Some(Decl::Class(c)) => {
                        self.resolve_class(c);
                        let info = &self.classes[c as usize];
                        let (tparams, params) = (info.params.clone(), info.ctor.params.clone());
                        let args: Vec<TyId> = tparams.iter().map(|&p| self.types.intern(Ty::Param(p))).collect();
                        let ret = self.types.class(c, &args);
                        return self.ctor_param(s, CtorSig { tparams, params, ret });
                    }
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
        // (of a type parameter, a generic alias's body seen as written: its keys, strings; each use
        // is the alias with its arguments, see `type_level`)
        if self.types.has_param(t) {
            return STR;
        }
        if matches!(self.types.get(t), Ty::Map(..)) {
            return STR;
        }
        // (a class's: its public fields and methods)
        if let Ty::Class(c, _) = *self.types.get(t) {
            self.resolve_class(c);
            let info = &self.classes[c as usize];
            let names: Vec<Sym> = info.fields.iter().filter(|f| f.vis == ast::Visibility::Public).map(|f| f.name).chain(info.methods.iter().filter(|m| m.vis == ast::Visibility::Public).map(|m| m.name)).collect();
            let names: Vec<Sym> = names.into_iter().filter(|&n| !self.name(n).starts_with('#')).collect();
            let lits: Vec<TyId> = names.into_iter().map(|n| self.types.str_lit(n)).collect();
            return if lits.is_empty() { NEVER } else { self.types.union(&lits) };
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
        if self.types.has_param(t) || self.types.has_param(k) {
            return UNKNOWN;
        }
        if self.types.is_numeric(k)
            && let Ty::Array(e) = *self.types.get(t)
        {
            return e;
        }
        // (of a tuple, `as const`'s: any of its elements)
        if self.types.is_numeric(k)
            && let Some(fs) = self.tuple_of(t)
        {
            let tys: Vec<TyId> = fs.iter().map(|f| f.ty).collect();
            return self.types.union(&tys);
        }
        let mut out = Vec::new();
        for key in self.flat_members(k) {
            let Ty::StrLit(name) = *self.types.get(key) else {
                let shown = self.show(k);
                self.report(Diagnostic::new("X0016", span, format!("can't index a type with `{shown}`")).note("supported", "`T[\"field\"]`, a union of field names, `T[number]` for arrays"));
                return ERROR;
            };
            // (a class's method: its function type)
            let method = if matches!(self.types.get(t), Ty::Class(..)) {
                match self.class_member(t, name) {
                    Some(mr @ super::check::class::ClassMemberRef::Method(_)) => Some(self.member_type(&mr)),
                    _ => None,
                }
            } else {
                None
            };
            match method.or_else(|| self.field_of(t, name)) {
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

    /// A local or parameter of written type `te` (`any`, `any[]`) is used in `region` as a
    /// JavaScript value of no fixed type is: a property of it (of an element, for `any[]`) read
    /// or set, it called or indexed.
    pub(super) fn used_dynamically(&self, te: ast::TypeId, region: Span, name: Sym) -> bool {
        let is_any = |c: &Self, t: ast::TypeId| matches!(&c.ast().ty(t).kind, TypeExprKind::Named { ns: None, name, args, .. } if args.is_empty() && c.name(*name) == "any");
        let elems = match &self.ast().ty(te).kind {
            _ if is_any(self, te) => false,
            TypeExprKind::Array(e) if is_any(self, *e) => true,
            TypeExprKind::Union(ms) if ms.iter().any(|&m| is_any(self, m)) => false,
            // (`Record<string, any>`, `Map<K, any>`: holding `any`s, as written)
            _ => return true,
        };
        let ast = self.ast();
        let inside = |s: Span| s.file == region.file && s.start >= region.start && s.end <= region.end;
        let is_name = |e: ExprId| matches!(ast.expr(e).kind, ast::ExprKind::Ident(n) if n == name);
        let hit = |e: ExprId| if elems { matches!(ast.expr(e).kind, ast::ExprKind::Index { obj, .. } if is_name(obj)) } else { is_name(e) };
        ast.exprs.iter().any(|x| {
            inside(x.span)
                && match x.kind {
                    ast::ExprKind::Member { obj, .. } => hit(obj),
                    ast::ExprKind::Call { callee, .. } => hit(callee),
                    ast::ExprKind::Index { obj, .. } => !elems && hit(obj),
                    // (given other values: of whatever type each is)
                    ast::ExprKind::Assign(_, t, _) => !elems && is_name(t),
                    ast::ExprKind::Update { target, .. } => hit(target),
                    // (asked its type, computed with: by what it holds)
                    ast::ExprKind::Typeof(x) | ast::ExprKind::Unary(_, x) => hit(x),
                    ast::ExprKind::Binary(op, l, r) => !matches!(op, ast::BinOp::Eq | ast::BinOp::Ne | ast::BinOp::And | ast::BinOp::Or | ast::BinOp::Nullish) && (hit(l) || hit(r)),
                    _ => false,
                }
        })
    }

    /// Whether a written type mentions `any` (`any`, `any[]`, `Record<string, any>`).
    fn mentions_any(&self, t: ast::TypeId) -> bool {
        match &self.ast().ty(t).kind {
            TypeExprKind::Named { ns, name, args, .. } => (ns.is_none() && self.name(*name) == "any") || args.iter().any(|&a| self.mentions_any(a)),
            TypeExprKind::Array(e) => self.mentions_any(*e),
            TypeExprKind::Union(ms) | TypeExprKind::Intersection(ms) => ms.iter().any(|&m| self.mentions_any(m)),
            TypeExprKind::Record(fs) => fs.iter().any(|f| self.mentions_any(f.ty)),
            _ => false,
        }
    }

    fn resolve_type(&mut self, te: ast::TypeId, tscope: &[(Sym, TyId)]) -> TyId {
        let ast = self.ast();
        let node = ast.ty(te);
        let span = node.span;
        match &node.kind {
            TypeExprKind::Error => ERROR,
            TypeExprKind::Typeof(x) => self.typeof_type(*x, span),
            // `new <T>(n: number) => Queue<T>`: a class given as a value (see `CtorSig`)
            TypeExprKind::Ctor(tps, ps, ret) => {
                let mut scope = tscope.to_vec();
                let mut tparams = Vec::new();
                for tp in tps {
                    let id = self.new_gparam(tp.name, None);
                    tparams.push(id);
                    scope.push((tp.name, self.types.intern(Ty::Param(id))));
                }
                let mut params = Vec::new();
                for p in ps {
                    let ty = match p.ty {
                        Some(t) => self.resolve_type(t, &scope),
                        None => ERROR,
                    };
                    let ty = if p.optional { self.types.optional(ty) } else { ty };
                    params.push(FnParam { ty, inout: false, optional: p.optional });
                }
                let ret = self.resolve_type(*ret, &scope);
                let name = self.interner.lookup("new").expect("interned by the parser");
                self.ctor_param(name, CtorSig { tparams, params, ret })
            }
            TypeExprKind::Predicate(_, t) => {
                self.resolve_type(*t, tscope);
                BOOL
            }
            TypeExprKind::Keyof(t) => {
                let t = self.resolve_type(*t, tscope);
                self.keyof_type(t, span)
            }
            TypeExprKind::Mapped { param, src, value, optional } => {
                let (param, src, value, optional) = (*param, *src, *value, *optional);
                // (`{ [K in keyof T]: ... }`: T's fields' optionality kept, as TypeScript's)
                let homomorphic = match &self.ast().ty(src).kind {
                    TypeExprKind::Keyof(t) => Some(*t),
                    _ => None,
                };
                let from = homomorphic.map(|t| self.resolve_type(t, tscope));
                let keys = self.resolve_type(src, tscope);
                if keys == ERROR {
                    return ERROR;
                }
                let members = self.flat_members(keys);
                if members.iter().any(|&m| m == STR || self.types.has_param(m)) || keys == STR {
                    let mut scope = tscope.to_vec();
                    scope.push((param, STR));
                    let v = self.resolve_type(value, &scope);
                    return self.types.intern(Ty::Map(STR, v));
                }
                let mut out = Vec::new();
                for m in members {
                    let Ty::StrLit(name) = *self.types.get(m) else {
                        let shown = self.show(keys);
                        self.report(Diagnostic::new("X0016", span, format!("a mapped type over `{shown}`: only string keys are supported")));
                        return ERROR;
                    };
                    let mut scope = tscope.to_vec();
                    scope.push((param, m));
                    let v = self.resolve_type(value, &scope);
                    let was_optional = from.and_then(|f| self.type_fields(f)).and_then(|fs| fs.iter().find(|x| x.name == name).map(|x| x.optional)).unwrap_or(false);
                    let opt = match optional {
                        Some(o) => o,
                        None => was_optional,
                    };
                    let v = if opt { self.types.optional(v) } else if optional == Some(false) { self.types.without_undefined(v) } else { v };
                    out.push(Field { name, ty: v, optional: opt });
                }
                let order: Vec<Sym> = out.iter().map(|f| f.name).collect();
                let t = self.types.record(out);
                self.types.note_field_order(t, order);
                t
            }
            TypeExprKind::Cond { check, ext, yes, no } => {
                let (check, ext, yes, no) = (*check, *ext, *yes, *no);
                let c = self.resolve_type(check, tscope);
                if c == ERROR {
                    return ERROR;
                }
                // (a type parameter still: undecided, either branch)
                if self.types.has_param(c) {
                    let mut infers = Vec::new();
                    collect_infers(self.ast(), ext, &mut infers);
                    let mut scope = tscope.to_vec();
                    scope.extend(infers.into_iter().map(|x| (x, UNKNOWN)));
                    let y = self.resolve_type(yes, &scope);
                    let n = self.resolve_type(no, tscope);
                    return self.types.union(&[y, n]);
                }
                // (`T extends U ? ...` of a union given for T: each member's answer)
                let naked = matches!(&self.ast().ty(check).kind, TypeExprKind::Named { ns: None, name, args, .. } if args.is_empty() && tscope.iter().any(|(s, _)| s == name));
                let parts = if naked { self.flat_members(c) } else { vec![c] };
                let mut out = Vec::new();
                for part in parts {
                    let mut infers = Vec::new();
                    collect_infers(self.ast(), ext, &mut infers);
                    let mut scope = tscope.to_vec();
                    let mut ids = Vec::new();
                    for &x in &infers {
                        let id = self.new_gparam(x, None);
                        ids.push(id);
                        scope.push((x, self.types.intern(Ty::Param(id))));
                    }
                    let e = self.resolve_type(ext, &scope);
                    let mut map: HashMap<u32, TyId> = HashMap::default();
                    if !ids.is_empty() {
                        self.unify(e, part, &ids, &mut map);
                    }
                    let e2 = if map.is_empty() { e } else { self.types.subst(e, &map) };
                    let fits = !self.types.has_param(e2) && self.assignable(part, e2);
                    let mut branch_scope = tscope.to_vec();
                    for (k, &x) in infers.iter().enumerate() {
                        branch_scope.push((x, map.get(&ids[k]).copied().unwrap_or(UNKNOWN)));
                    }
                    out.push(if fits { self.resolve_type(yes, &branch_scope) } else { self.resolve_type(no, tscope) });
                }
                if out.is_empty() { NEVER } else { self.types.union(&out) }
            }
            TypeExprKind::Infer(x) => match tscope.iter().rev().find(|(s, _)| s == x) {
                Some(&(_, t)) => t,
                None => {
                    let n = self.name(*x).to_string();
                    self.report(Diagnostic::new("P0003", span, format!("`infer {n}` belongs in a conditional type's `extends` clause")));
                    ERROR
                }
            },
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
            TypeExprKind::BoolLit(b) => self.types.bool_lit(*b),
            TypeExprKind::Array(e) => {
                let e = self.resolve_type(*e, tscope);
                self.types.array(e)
            }
            TypeExprKind::Union(ms) => {
                let ms: Vec<TyId> = ms.iter().map(|&m| self.resolve_type(m, tscope)).collect();
                self.types.union(&ms)
            }
            TypeExprKind::Intersection(ms) => {
                let ms: Vec<TyId> = ms.iter().map(|&m| self.resolve_type(m, tscope)).collect();
                self.intersection(&ms, span)
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
                        // (`...args: A`: the rest of the parameters, as the tuple `A`)
                        // (`...args: T[]`: the arguments from there on, each a `T`)
                        let ty = if p.rest && ty != ERROR { self.types.intern(Ty::Spread(ty)) } else { ty };
                        FnParam { ty, inout: p.inout, optional: p.optional }
                    })
                    .collect();
                let r = self.resolve_type(*ret, tscope);
                // (written without `throws`: as TypeScript's, it may throw once a function that
                // does is given to it, see `spec::blame_fn_type`)
                let key = spec::fn_type_key(self.cur, te);
                let th = match throws {
                    Some(t) => {
                        let th = self.resolve_type(*t, tscope);
                        let span = self.ast().ty(*t).span;
                        self.check_throws_type(th, span);
                        th
                    }
                    None if self.spec.float_globals.contains(&key) => self.error_class(),
                    None => NEVER,
                };
                let t = self.types.func_throws(ps, r, th);
                if throws.is_none() && th == NEVER {
                    self.fn_sites.entry(t).or_default().push(key);
                }
                t
            }
            TypeExprKind::Named { ns: None, name, args, .. } if *name == self.syms.number && args.is_empty() && self.spec_int(te) && !tscope.iter().any(|(s, _)| s == name) => {
                self.spec_used = true;
                INT
            }
            TypeExprKind::Named { ns, name, name_span, args } => self.resolve_named(*ns, *name, *name_span, args, span, tscope),
        }
    }

    /// Where `any` is written in a type (its name's position): a parameter's `any`, `any[]`,
    /// `Map<string, any>`.
    fn any_uses(&self, te: ast::TypeId, out: &mut Vec<u32>) {
        match &self.ast().ty(te).kind {
            TypeExprKind::Named { ns: None, name, name_span, args } if args.is_empty() && self.name(*name) == "any" => out.push(name_span.start),
            TypeExprKind::Named { args, .. } => args.iter().for_each(|&a| self.any_uses(a, out)),
            TypeExprKind::Array(e) | TypeExprKind::Keyof(e) => self.any_uses(*e, out),
            TypeExprKind::Union(ts) | TypeExprKind::Intersection(ts) => ts.iter().for_each(|&t| self.any_uses(t, out)),
            TypeExprKind::Record(fs) => fs.iter().for_each(|f| self.any_uses(f.ty, out)),
            _ => {}
        }
    }

    fn resolve_named(&mut self, ns: Option<(Sym, Span)>, name: Sym, name_span: Span, args: &[ast::TypeId], span: Span, tscope: &[(Sym, TyId)]) -> TyId {
        if ns.is_none()
            && args.is_empty()
            && let Some(&t) = self.any_types.get(&(self.cur, name_span.start))
        {
            return t;
        }
        // (`Generator<T, TReturn, TNext>`: only what it yields matters here)
        let args = if ns.is_none() && args.len() > 1 && matches!(self.name(name), "Generator" | "Iterator" | "IterableIterator" | "AsyncGenerator") && !self.scopes[self.cur as usize].types.contains_key(&name) { &args[..1] } else { args };
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
            // (a module's own `interface Map<K, V>` hides the built-in, as in TypeScript)
            match self.scopes[self.cur as usize].types.get(&name).map(|d| d.0) {
                Some(d) => Some(d),
                None => match self.builtin_type(&text, &targs, span, name_span) {
                    Some(t) => return t,
                    None => None,
                },
            }
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
                // (`type R<T> = T extends string ? number : boolean`: its body made with these
                // arguments, the type-level operators evaluated on them)
                if !targs.is_empty()
                    && let Some((_, _, tparams, body_te)) = alias_item(&self.modules[am as usize].ast.items[ai as usize].kind)
                    && type_level(&self.modules[am as usize].ast, body_te)
                {
                    let body_te = body_te;
                    let scope: Vec<(Sym, TyId)> = tparams.iter().map(|tp| tp.name).zip(targs.iter().copied()).collect();
                    return self.with_module(am, |c| c.resolve_type(body_te, &scope));
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
            "Map" | "ReadonlyMap" | "WeakMap" => Some(if arity(self, 2) { self.types.intern(Ty::Map(targs[0], targs[1])) } else { ERROR }),
            "Set" | "ReadonlySet" | "WeakSet" => Some(if arity(self, 1) { self.types.intern(Ty::Set(targs[0])) } else { ERROR }),
            // (a generator's result, its return and `next` types being `void`/`undefined`)
            "Generator" | "IterableIterator" | "Iterator" | "Iterable" if !targs.is_empty() => Some(self.types.intern(Ty::Gen(targs[0]))),
            // (without its `T`: of anything, as TypeScript's `AsyncGenerator` is of `any`)
            "AsyncGenerator" | "AsyncIterableIterator" | "AsyncIterator" | "AsyncIterable" => Some(self.types.intern(Ty::AsyncGen(targs.first().copied().unwrap_or(UNKNOWN)))),
            "any" if self.ts_module() => Some(ANY),
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
                // (`Promise<T>`, as TypeScript's, may reject once a promise that does is given to
                // it: see `spec::blame_fn_type`)
                [v] => {
                    let key = spec::promise_key(self.cur, span.start);
                    let e = if self.spec.float_globals.contains(&key) { self.error_class() } else { NEVER };
                    let t = self.types.promise(v, e);
                    if e == NEVER {
                        self.fn_sites.entry(t).or_default().push(key);
                    }
                    t
                }
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
            // `Uppercase<"a" | "b">`: each string literal changed so (`string` when it can't be told)
            "Capitalize" | "Uncapitalize" | "Uppercase" | "Lowercase" => {
                if !arity(self, 1) {
                    return Some(ERROR);
                }
                let mut out = Vec::new();
                for m in self.flat_members(targs[0]) {
                    let Ty::StrLit(lit) = *self.types.get(m) else { return Some(if targs[0] == ERROR { ERROR } else { STR }) };
                    let s = self.name(lit).to_string();
                    let changed = match text {
                        "Uppercase" => s.to_uppercase(),
                        "Lowercase" => s.to_lowercase(),
                        _ => {
                            let mut cs = s.chars();
                            match cs.next() {
                                Some(c) if text == "Capitalize" => c.to_uppercase().chain(cs).collect(),
                                Some(c) => c.to_lowercase().chain(cs).collect(),
                                None => s.clone(),
                            }
                        }
                    };
                    match self.interner.lookup(&changed) {
                        Some(sym) => out.push(self.types.intern(Ty::StrLit(sym))),
                        None => return Some(STR),
                    }
                }
                Some(self.types.union(&out))
            }
            "ReturnType" | "Parameters" | "Awaited" | "NonNullable" | "Readonly" | "Partial" | "Required" => {
                if !arity(self, 1) {
                    return Some(ERROR);
                }
                let t = targs[0];
                if t == ERROR {
                    return Some(ERROR);
                }
                Some(match text {
                    "Parameters" => match *self.types.get(t) {
                        Ty::Func(ps, _, _) => {
                            let tys: Vec<TyId> = self.types.params(ps).iter().map(|p| p.ty).collect();
                            self.tuple_ty(&tys)
                        }
                        _ if self.types.has_param(t) => UNKNOWN,
                        _ => {
                            let shown = self.show(t);
                            self.report(Diagnostic::new("T0010", span, format!("`Parameters` takes a function type, found `{shown}`")));
                            ERROR
                        }
                    },
                    "ReturnType" => match *self.types.get(t) {
                        Ty::Func(_, r, _) => r,
                        _ if self.types.has_param(t) => UNKNOWN,
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
                // (`Record<number, V>`: a number-keyed map; `Record<K, V>`, K a type parameter
                // (`K extends string`), a map keyed by whatever K is)
                if self.types.is_numeric(targs[0]) || matches!(self.types.get(targs[0]), Ty::Param(_)) {
                    return Some(self.types.intern(Ty::Map(targs[0], targs[1])));
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
            // (in TypeScript, any object: held as an `unknown`, tested by what it holds)
            "object" | "Object" if self.ts_module() => Some(UNKNOWN),
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
            "symbol" | "Function" => {
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
        // (`any` goes anywhere, and anything into one: converted, and checked, when it runs)
        if src == ANY || dst == ANY {
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
            (Ty::Int | Ty::F64 | Ty::F32 | Ty::I8 | Ty::I16 | Ty::I32 | Ty::U8 | Ty::U8C | Ty::U16 | Ty::U32, Ty::I8 | Ty::I16 | Ty::I32 | Ty::U8 | Ty::U8C | Ty::U16 | Ty::U32) if top && self.wrap_store => return true,
            (Ty::F64 | Ty::F32, Ty::Int) if top && self.float_args => return true,
            (Ty::Int | Ty::I8 | Ty::I16 | Ty::I32 | Ty::U8 | Ty::U8C | Ty::U16 | Ty::U32 | Ty::U64, Ty::Decimal) if top => return true,
            (Ty::StrLit(_), Ty::Str) => return true,
            (Ty::BoolLit(_), Ty::Bool) => return true,
            (Ty::Undefined, Ty::Void) => return true,
            // (a tuple spread into parameters: each element as an argument)
            (Ty::Spread(a), Ty::Spread(b)) => {
                let (a, b) = (*a, *b);
                if let (Some(xs), Some(ys)) = (self.types.tuple_elems(a), self.types.tuple_elems(b)) {
                    return xs.len() == ys.len() && xs.iter().zip(&ys).all(|(&x, &y)| self.assignable_in(x, y, true, assuming));
                }
                return self.assignable_in(a, b, top, assuming);
            }
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
            // (read only: what it yields fits; an array is iterable too, through a generator
            // over it)
            (Ty::Gen(a), Ty::Gen(b)) => self.assignable_in(a, b, false, assuming),
            (Ty::Array(a), Ty::Gen(b)) if top => a == b || self.assignable_in(a, b, false, assuming) && self.types.is_string(a) == self.types.is_string(b),
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
                // (a rest parameter, `...xs: T[]`, on either side: the arguments it takes, each a `T`)
                let (sa, sb) = (pa.last().and_then(|p| self.types.rest_elem(p.ty)), pb.last().and_then(|p| self.types.rest_elem(p.ty)));
                if sa.is_some() || sb.is_some() {
                    let fa = &pa[..pa.len() - sa.is_some() as usize];
                    let fb = &pb[..pb.len() - sb.is_some() as usize];
                    for (i, y) in fb.iter().enumerate() {
                        let x = match (fa.get(i), sa) {
                            (Some(x), _) => {
                                if x.inout != y.inout {
                                    return false;
                                }
                                x.ty
                            }
                            (None, Some(e)) if !y.inout => e,
                            (None, Some(_)) => return false,
                            // (a callback may ignore trailing parameters)
                            (None, None) => continue,
                        };
                        if !self.assignable_in(y.ty, x, true, assuming) {
                            return false;
                        }
                    }
                    for x in fa.iter().skip(fb.len()) {
                        match sb {
                            Some(e) if !x.inout => {
                                let e = if x.optional { self.types.optional(e) } else { e };
                                let bare = self.types.without_undefined(e);
                                if !self.assignable_in(e, x.ty, true, assuming) && !self.assignable_in(bare, x.ty, true, assuming) {
                                    return false;
                                }
                            }
                            None if x.optional => {}
                            _ => return false,
                        }
                    }
                    if let (Some(a), Some(b)) = (sa, sb)
                        && !self.assignable_in(b, a, true, assuming)
                    {
                        return false;
                    }
                    return rb == VOID || self.assignable_in(ra, rb, true, assuming);
                }
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
            // (a type parameter fits what its bound fits)
            (Ty::Param(p), _) if self.gparam(p).bound.is_some() => {
                let b = self.gparam(p).bound.unwrap();
                self.assignable_in(b, dst, top, assuming)
            }
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

    /// A value of `ty` is changed in place: its records are objects if anything shares them.
    pub(crate) fn note_mutated(&mut self, ty: TyId) {
        for m in self.types.members(ty) {
            if let Ty::Record(_) = self.types.get(m) {
                self.mutated.insert(m);
            }
        }
    }

    /// Records of type `ty` changed in place or shared (objects, or maybe objects).
    pub(crate) fn maybe_object(&self, ty: TyId) -> bool {
        self.objects.contains(&ty) || self.mutated.contains(&ty)
    }

    /// A value of `ty` is compared by identity: its records are objects.
    pub(crate) fn note_object(&mut self, ty: TyId) {
        for m in self.types.members(ty) {
            if let Ty::Record(_) = self.types.get(m) {
                self.objects.insert(m);
            }
        }
    }

    /// The rest of `objects`: records searched for in arrays (`indexOf`, `includes`), set
    /// members and map keys (found by identity), and records with fields changed generically.
    fn find_objects(&mut self) {
        let mut found = Vec::new();
        if let Some(facts) = &self.facts {
            for (m, f) in facts.iter().enumerate() {
                let ast = &self.modules[m].ast;
                for (&e, call) in &f.calls {
                    if let facts::Callee::Method(recv) = call.callee
                        && let Ty::Array(el) = *self.types.get(recv)
                        && let ast::ExprKind::Call { callee, .. } = &ast.expr(e).kind
                        && let ast::ExprKind::Member { name, .. } = &ast.expr(*callee).kind
                        && matches!(self.interner.get(*name), "indexOf" | "lastIndexOf" | "includes")
                    {
                        found.push(el);
                    }
                }
            }
        }
        for i in 0..self.types.count() {
            match *self.types.get(TyId(i)) {
                Ty::Set(k) | Ty::Map(k, _) => found.push(k),
                Ty::Record(fs) if self.types.fields(fs).iter().any(|f| self.object_fields.contains(&f.name)) => found.push(TyId(i)),
                _ => {}
            }
        }
        for t in found {
            self.note_object(t);
        }
        // Records changed in place: objects if something shares them.
        let cands: HashSet<TyId> = self.mutated.iter().copied().filter(|t| !self.objects.contains(t)).collect();
        let shared = self.shared_records(&cands);
        self.objects.extend(shared);
        let generic: Vec<TyId> = self.objects.iter().copied().filter(|&t| self.types.has_param(t)).collect();
        for t in generic {
            if let Ty::Record(fs) = *self.types.get(t) {
                let names = self.types.fields(fs).iter().map(|f| f.name).collect();
                self.object_shapes.insert(names);
            }
        }
    }

    /// Can values of these types ever be equal?
    fn comparable(&mut self, a: TyId, b: TyId) -> bool {
        if a == ERROR || b == ERROR || a == UNKNOWN || b == UNKNOWN || a == JS || b == JS || a == ANY || b == ANY {
            return true;
        }
        // (a parameter's `any`: compared with anything, as TypeScript's is)
        let any = self.interner.lookup("any");
        if [a, b].iter().any(|&t| matches!(*self.types.get(t), Ty::Param(p) if Some(self.gparam(p).name) == any)) {
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
            if (found == JS || found == ANY) && self.spec.on {
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
        self.dict_hit(found, expected, span);
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

    fn narrow(&mut self, local: LocalId, ty: TyId) {
        let fcx = self.fcx.last_mut().unwrap();
        let (name, declared) = (fcx.locals[local].name, fcx.locals[local].ty);
        let assigned_by_closure = fcx.closure_assigned.contains(&name);
        let mutated_by_closure = fcx.closure_mutated.contains(&name);
        let changed = assigned_by_closure || mutated_by_closure && !self.references_only(declared);
        // (a TypeScript module's `let` a closure assigns narrows as TypeScript's does, until the
        // next call, which could run the closure: see `invalidate_heap`)
        let ts_narrows = assigned_by_closure && self.ts_module();
        let fcx = self.fcx.last_mut().unwrap();
        if changed && ty != declared && !ts_narrows {
            return;
        }
        if ts_narrows && ty != declared && !fcx.closure_narrowed.contains(&local) {
            fcx.closure_narrowed.push(local);
        }
        // (where another local of the same name hides it, as a closure's parameter hides a
        // script variable: beneath that one, right after its own latest entry)
        let mut hidden = None;
        'find: for (si, scope) in fcx.scopes.iter().enumerate().rev() {
            for (ei, &(s, id, _)) in scope.iter().enumerate().rev() {
                if s == name {
                    if id != local {
                        hidden = Some(None);
                        continue;
                    }
                    if hidden.is_some() {
                        hidden = Some(Some((si, ei)));
                    }
                    break 'find;
                }
            }
        }
        match hidden {
            None => fcx.scopes.last_mut().unwrap().push((name, local, Some(ty))),
            Some(Some((si, ei))) => fcx.scopes[si].insert(ei + 1, (name, local, Some(ty))),
            Some(None) => {}
        }
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
        self.lookup_path_ex(local, path).map(|(t, _)| t)
    }

    /// `lookup_path`, and whether a call came since (the narrowing to be checked when read).
    fn lookup_path_ex(&self, local: LocalId, path: &[Sym]) -> Option<(TyId, bool)> {
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
                        Some(t) if pp.as_slice() == path => return Some((*t, fcx.asserted.contains(&idx))),
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
        TypeExprKind::Union(ms) | TypeExprKind::Intersection(ms) => ms.iter().for_each(|&m| collect_type_refs(ast, m, out)),
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

/// A class's name as written: a local class's (`C$$3`, see the parser's `lift_classes`) or a
/// namespace's (`N$$C`) without what made it the module's own.
fn shown_class_name(n: &str) -> String {
    match n.rsplit_once("$$") {
        Some((a, b)) if b.chars().all(|c| c.is_ascii_digit()) => a.to_string(),
        Some((_, b)) => b.to_string(),
        None => n.to_string(),
    }
}

/// The `infer X` names in a type.
fn collect_infers(ast: &ast::Ast, te: ast::TypeId, out: &mut Vec<Sym>) {
    match &ast.ty(te).kind {
        TypeExprKind::Infer(x) => out.push(*x),
        TypeExprKind::Named { args, .. } => args.iter().for_each(|&a| collect_infers(ast, a, out)),
        TypeExprKind::Array(e) | TypeExprKind::Keyof(e) => collect_infers(ast, *e, out),
        TypeExprKind::Union(ms) | TypeExprKind::Intersection(ms) => ms.iter().for_each(|&m| collect_infers(ast, m, out)),
        TypeExprKind::Record(fs) => fs.iter().for_each(|f| collect_infers(ast, f.ty, out)),
        TypeExprKind::Func(ps, r, _) => {
            ps.iter().filter_map(|p| p.ty).for_each(|t| collect_infers(ast, t, out));
            collect_infers(ast, *r, out);
        }
        TypeExprKind::Access(a, b) => {
            collect_infers(ast, *a, out);
            collect_infers(ast, *b, out);
        }
        _ => {}
    }
}

/// A type written with type-level operators (conditional, mapped, `keyof`, indexed): a generic
/// alias of one is made again for each use, with its arguments (see `resolve_named`).
pub(crate) fn type_level(ast: &ast::Ast, te: ast::TypeId) -> bool {
    match &ast.ty(te).kind {
        TypeExprKind::Cond { .. } | TypeExprKind::Mapped { .. } | TypeExprKind::Keyof(_) | TypeExprKind::Access(..) | TypeExprKind::Infer(_) => true,
        TypeExprKind::Named { args, .. } => args.iter().any(|&a| type_level(ast, a)),
        TypeExprKind::Array(e) => type_level(ast, *e),
        TypeExprKind::Union(ms) | TypeExprKind::Intersection(ms) => ms.iter().any(|&m| type_level(ast, m)),
        TypeExprKind::Record(fs) => fs.iter().any(|f| type_level(ast, f.ty)),
        TypeExprKind::Func(ps, r, _) => ps.iter().filter_map(|p| p.ty).any(|t| type_level(ast, t)) || type_level(ast, *r),
        _ => false,
    }
}
