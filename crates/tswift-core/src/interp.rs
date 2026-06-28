//! The `eval(node, env)` tree-walker.
//!
//! Control flow (`return`, and later `break`/`continue`/`throw`) unwinds through
//! the `Err` channel as a [`Signal`], so a `?` naturally propagates it up to the
//! construct that handles it — without panicking. Real interpreter failures ride
//! the same channel as [`Signal::Error`].

use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;

use tswift_frontend::{Analysis, Node, NodeKind};

use crate::env::{BindError, Env, Scope};
use crate::ops;
use crate::stdlib::{
    AlgoFn, Arg, BuiltinReceiver, FreeFn, MethodEntry, Outcome, PropertyFn, StaticFn, StdContext,
    StdError, StructMethodFn,
};
use std::cell::RefCell;
use std::rc::Rc as StdRc;

use crate::value::{ClassObj, EnumObj, IntValue, IntWidth, StructObj, SwiftValue};

// Declaration modifier bits used by this milestone (see msf.h §9).
const MOD_STATIC: u32 = 1 << 5;
const MOD_MUTATING: u32 = 1 << 8;
const MOD_LAZY: u32 = 1 << 10;
const MOD_WEAK: u32 = 1 << 11;

/// Maximum nested Swift call depth before the interpreter traps, converting
/// unbounded recursion into a catchable error instead of a native stack
/// overflow.
const MAX_CALL_DEPTH: usize = 5000;

/// Maximum number of elements a custom sequence algorithm may eagerly
/// materialize before trapping. `for-in` remains lazy and can still terminate an
/// unbounded sequence with `break`; eager algorithms like `map` cannot.
#[cfg(not(test))]
const MAX_SEQUENCE_MATERIALIZE: usize = 100_000;
#[cfg(test)]
const MAX_SEQUENCE_MATERIALIZE: usize = 32;

/// A native (Rust-implemented) Swift function. It receives the output sink and
/// the already-evaluated arguments, and returns its result value.
pub type NativeFn = fn(&mut dyn Write, &[SwiftValue]) -> SwiftValue;

/// A failure while evaluating the AST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// A construct the evaluator does not implement yet.
    Unsupported(String),
    /// A call to a function name with no registered native.
    UnknownFunction(String),
    /// Use of an unbound variable.
    UnknownVariable(String),
    /// Assignment to a `let` binding.
    Immutable(String),
    /// A runtime trap: overflow, division by zero, deep recursion, etc.
    Trap(String),
    /// A type error the evaluator detected at runtime.
    Type(String),
    /// The source failed to analyze; carries msf's diagnostics, joined.
    Analysis(String),
    /// Writing to the output sink failed.
    Io(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::Unsupported(what) => write!(f, "unsupported construct: {what}"),
            EvalError::UnknownFunction(name) => write!(f, "unknown function: {name}"),
            EvalError::UnknownVariable(name) => write!(f, "unknown variable: {name}"),
            EvalError::Immutable(name) => {
                write!(f, "cannot assign to `{name}`: it is a `let` constant")
            }
            EvalError::Trap(msg) => write!(f, "fatal error: {msg}"),
            EvalError::Type(msg) => write!(f, "type error: {msg}"),
            EvalError::Analysis(diags) => write!(f, "analysis failed:\n{diags}"),
            EvalError::Io(e) => write!(f, "output error: {e}"),
        }
    }
}

impl std::error::Error for EvalError {}

/// A non-local control transfer produced while evaluating a node. Travels up the
/// `Err` channel so `?` propagates it to the handling construct.
///
/// `Break`/`Continue`/`Throw` are wired in the control-flow and error-handling
/// milestones; they exist here so the dispatch shape is stable.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
enum Signal {
    /// `return [value]` — unwinds to the enclosing function call.
    Return(SwiftValue),
    /// `break [label]` — unwinds to the targeted loop/switch.
    Break(Option<String>),
    /// `continue [label]` — unwinds to the targeted loop.
    Continue(Option<String>),
    /// `fallthrough` — proceed to the next `switch` case body.
    Fallthrough,
    /// A thrown Swift error value (error handling milestone).
    Throw(SwiftValue),
    /// A genuine interpreter error (not Swift control flow).
    Error(EvalError),
}

impl From<EvalError> for Signal {
    fn from(e: EvalError) -> Self {
        Signal::Error(e)
    }
}

/// Collapse a [`Signal`] into an [`EvalError`] for the public render API, which
/// cannot legitimately observe loop/`return`/`throw` control flow escaping a
/// top-level struct instantiation or member read.
fn signal_eval(sig: Signal) -> EvalError {
    match sig {
        Signal::Error(e) => e,
        Signal::Throw(v) => EvalError::Trap(format!("uncaught error: {v}")),
        other => EvalError::Unsupported(format!("stray control flow: {other:?}")),
    }
}

/// Convenience: an operator/runtime trap message becomes a [`Signal::Error`].
fn trap(msg: String) -> Signal {
    Signal::Error(EvalError::Trap(msg))
}

type Eval = Result<SwiftValue, Signal>;

/// One declared Swift parameter, precomputed from its `AST_PARAM` node.
struct Param {
    label: Option<String>,
    name: String,
    /// The written parameter type (`Double`, …), used to coerce integer
    /// literal arguments into floating parameters.
    ty: Option<String>,
    variadic: bool,
    inout_: bool,
    /// `@autoclosure`: the argument expression is wrapped in a zero-argument
    /// thunk and only evaluated when the parameter is called.
    autoclosure: bool,
    default: Option<Node<'static>>,
}

/// A user-defined function: its parameters, body, and captured scope chain.
struct FuncDef {
    params: Vec<Param>,
    body: Option<Node<'static>>,
    captured: Vec<Scope>,
    /// Names of the function's generic type parameters (`<T: P, U>` → `[T, U]`),
    /// used to bind placeholders to concrete argument types at the call.
    generic_params: Vec<String>,
    /// Element labels of a tuple return type (`-> (lo: Int, hi: Int)`), applied
    /// to a returned tuple so `f().lo` works even when the `return` expression
    /// itself was unlabeled.
    ret_tuple_labels: Option<Vec<Option<String>>>,
}

/// A stored property of a struct.
struct StoredProp {
    name: String,
    /// The written type annotation (`Double`, `Int`, …), used to coerce an
    /// integer initializer literal into a floating field at construction.
    ty: Option<String>,
    default: Option<Node<'static>>,
    lazy: bool,
    will_set: Option<(String, Node<'static>)>,
    did_set: Option<(String, Node<'static>)>,
}

/// A computed property of a struct.
struct ComputedProp {
    getter: Option<Node<'static>>,
    setter: Option<Node<'static>>,
    setter_param: Option<String>,
    /// The setter is `nonmutating` (writes through a reference), so it may run
    /// on an immutable value-type binding without a writeback.
    setter_nonmutating: bool,
    /// `static`/`class` (type-level) computed property, read as `Type.prop`.
    is_static: bool,
}

/// A method of a struct.
struct MethodDef {
    params: Vec<Param>,
    body: Option<Node<'static>>,
    mutating: bool,
    /// Names of the method's generic type parameters (`<T: P>`), used to bind
    /// placeholders to concrete argument types for static dispatch (`T.zero()`).
    generic_params: Vec<String>,
    /// `static`/`class` (type) method, callable through `Type.m()` and via an
    /// implicit member `.m()` in a contextual position.
    is_static: bool,
}

/// An instance `subscript` declaration. A type may declare several overloads,
/// selected by the number of index parameters at the call site.
struct SubscriptDef {
    params: Vec<Param>,
    getter: Option<Node<'static>>,
    setter: Option<Node<'static>>,
    /// The setter's value parameter name (`newValue` by default).
    setter_param: String,
}

/// A struct type declaration.
struct StructDef {
    stored: Vec<StoredProp>,
    computed: std::collections::HashMap<String, ComputedProp>,
    methods: std::collections::HashMap<String, MethodDef>,
    /// Same-named methods that differ by argument label (`buildEither(first:)`
    /// vs `(second:)`). `methods` keeps only the last such declaration; this
    /// records the full overload set so a call can be dispatched by its labels.
    method_overloads: std::collections::HashMap<String, Vec<MethodDef>>,
    subscripts: Vec<SubscriptDef>,
    /// A `static subscript`, addressed as `Type[index]`.
    static_subscript: Option<MethodDef>,
    /// A custom initializer, if the struct declares one (else memberwise).
    init: Option<MethodDef>,
    /// All custom initializer overloads, selected by argument labels/types.
    init_overloads: Vec<MethodDef>,
    /// Stored property name → its `@propertyWrapper` type, when wrapped.
    wrappers: std::collections::HashMap<String, String>,
    /// `@dynamicMemberLookup`: an unresolved member name routes through the
    /// type's `subscript(dynamicMember:)`.
    dynamic_member_lookup: bool,
    /// `@dynamicCallable`: calling an instance routes through the type's
    /// `dynamicallyCall(withArguments:)` / `dynamicallyCall(withKeywordArguments:)`.
    dynamic_callable: bool,
}

/// One case of an enum.
struct EnumCaseDef {
    name: String,
    /// The precomputed raw value (with Swift's auto-increment / name defaults).
    raw: Option<SwiftValue>,
    /// The written type of each associated value (`circle(radius: Double)` →
    /// `[Some("Double")]`), used to coerce integer literals into floating
    /// payload slots at construction.
    payload_types: Vec<Option<String>>,
}

/// The backing type of an enum's raw values.
#[derive(Clone, Copy)]
enum RawKind {
    Int,
    Str,
}

/// An enum type declaration.
struct EnumDef {
    cases: Vec<EnumCaseDef>,
    methods: std::collections::HashMap<String, MethodDef>,
    computed: std::collections::HashMap<String, ComputedProp>,
}

/// A class type declaration.
struct ClassDef {
    superclass: Option<String>,
    stored: Vec<StoredProp>,
    weak_fields: Vec<String>,
    computed: std::collections::HashMap<String, ComputedProp>,
    methods: std::collections::HashMap<String, MethodDef>,
    init: Option<MethodDef>,
    /// All custom initializer overloads, selected by argument labels/types.
    init_overloads: Vec<MethodDef>,
    deinit: Option<Node<'static>>,
    /// A `static subscript`, addressed as `Type[index]`.
    static_subscript: Option<MethodDef>,
}

/// A protocol declaration: its inherited protocols and any default member
/// implementations supplied through `extension Protocol { … }`.
struct ProtoDef {
    inherited: Vec<String>,
    methods: std::collections::HashMap<String, MethodDef>,
    computed: std::collections::HashMap<String, ComputedProp>,
}

/// A closure value's definition: either a user closure (parameters + body
/// statements) or a synthesized operator-function reference (`+`, `<`, …) made
/// when a bare operator is passed as a function value (`reduce(0, +)`).
enum ClosureDef {
    User {
        params: Vec<Param>,
        body: Vec<Node<'static>>,
    },
    Operator(String),
    /// A key path `\Root.a.b`, represented as a callable value. Calling it with
    /// one argument walks the path from that argument; used both as a function
    /// (`names.map(\.count)`) and via `root[keyPath:]` subscripting. An empty
    /// component list is the identity key path (`\.self`).
    KeyPath(Vec<String>),
}

/// An assignable storage location: a root variable plus a field path.
#[derive(Debug, Clone)]
struct Place {
    root: String,
    path: Vec<String>,
}

/// A single evaluated call argument: its label, value, and (for `inout`) the
/// caller location to write back to.
struct CallArg {
    label: Option<String>,
    value: SwiftValue,
    place: Option<Place>,
}

/// The tree-walking interpreter.
pub struct Interpreter<'w> {
    out: &'w mut dyn Write,
    natives: HashMap<String, NativeFn>,
    /// Free-function intrinsics served through the [`StdContext`] seam.
    free_fns: HashMap<String, FreeFn>,
    /// Method intrinsics keyed by `(builtin receiver, method name)`.
    intrinsics: HashMap<(BuiltinReceiver, String), MethodEntry>,
    /// Computed-property intrinsics keyed by `(builtin receiver, property name)`.
    properties: HashMap<(BuiltinReceiver, String), PropertyFn>,
    /// Static (type-level) method intrinsics keyed by `(builtin receiver, name)`.
    static_methods: HashMap<(BuiltinReceiver, String), StaticFn>,
    /// `Sequence`/`Collection` algorithms keyed by method name, applied to any
    /// builtin sequence receiver (layer 2 of the dispatch seam).
    algorithms: HashMap<String, AlgoFn>,
    /// Generic method intrinsics dispatched on any struct receiver by name, a
    /// fallback after user methods and builtin receivers (SwiftUI modifiers).
    struct_methods: HashMap<String, StructMethodFn>,
    env: Env,
    funcs: Vec<FuncDef>,
    structs: HashMap<String, StructDef>,
    enums: HashMap<String, EnumDef>,
    classes: HashMap<String, ClassDef>,
    protocols: HashMap<String, ProtoDef>,
    /// type name → protocols it conforms to (directly).
    conformances: HashMap<String, Vec<String>>,
    /// Protocol-composition typealiases (`typealias X = A & B`) → their
    /// component protocol names, so a conformance to `X` expands to `A` + `B`
    /// for default-implementation lookup.
    protocol_aliases: HashMap<String, Vec<String>>,
    closures: Vec<(ClosureDef, Vec<Scope>)>,
    statics: HashMap<String, SwiftValue>,
    /// Stack of type names for the `static` methods currently executing, so an
    /// unqualified reference inside a `static func` resolves to a type-level
    /// (static) property of that type.
    static_ctx: Vec<String>,
    /// Stack of class names for the methods currently executing (for `super`).
    class_ctx: Vec<String>,
    /// Stack of generic type-parameter substitutions for the calls currently
    /// executing, so a static reference through a generic placeholder
    /// (`T.zero()` where `T == Vec2`) resolves to the concrete type.
    type_bindings: Vec<HashMap<String, String>>,
    /// User extension methods declared on builtin types (`extension Int`,
    /// `extension Array`, …), keyed by the builtin type name then method name.
    builtin_ext_methods: HashMap<String, HashMap<String, MethodDef>>,
    /// User extension computed properties on builtin types, keyed the same way.
    builtin_ext_computed: HashMap<String, HashMap<String, ComputedProp>>,
    /// Per-scope stack of `defer` blocks, run LIFO on scope exit.
    defer_stack: Vec<Vec<Node<'static>>>,
    /// The `@main` entry type, if one was declared.
    main_type: Option<String>,
    /// The structured-concurrency task table (see ADR-0005). Each `async let`,
    /// `Task { }`, and `group.addTask` pushes a slot; `await`-ing a
    /// `SwiftValue::Task` drives the matching slot to completion.
    tasks: Vec<TaskSlot>,
    /// `withTaskGroup` groups: each holds the task ids added via `addTask`,
    /// drained in order by `for await`.
    groups: Vec<Vec<usize>>,
    /// `with*Continuation` slots, one per continuation handed to a body. The
    /// state machine (`Pending` → `Resumed` → `Consumed`) lets `resume(...)`
    /// diagnose both double resume *and* late resume after the enclosing
    /// `with*Continuation` has already read the value.
    continuations: Vec<ContinuationState>,
    /// Source file name for `#file`.
    filename: String,
    depth: usize,
    /// SplitMix64 state for builtin randomness (`Bool.random()`, …). Seeded
    /// from wall-clock time so runs vary; advanced per draw.
    rng_state: u64,
    /// Stack of expected (contextual) parameter types for the arguments
    /// currently being evaluated. Pushed per-argument by `eval_args_with` so an
    /// implicit-member expression (`.custom("x")`) can resolve against the
    /// call-site parameter type when its own node lacks an inferred type. The
    /// top of the stack is the active hint; `None` entries mean "no hint".
    type_hint: Vec<Option<String>>,
}

/// A spawned structured-concurrency task: a zero-argument closure producing the
/// task's result, plus the class context it was spawned in and its run state.
struct TaskSlot {
    /// Index into [`Interpreter::closures`] of the task body (a 0-arg thunk).
    closure: usize,
    /// The `super`/`self` dispatch context captured at spawn time.
    class_ctx: Vec<String>,
    state: TaskState,
    /// Cooperative-cancellation flag (set by `cancelAll()` / group teardown).
    cancelled: bool,
}

/// Where a task is in its lifecycle.
enum TaskState {
    /// Spawned but not yet run.
    Pending,
    /// Currently executing (used to detect `await`-on-self deadlock).
    Running,
    /// Finished, carrying its memoized outcome (value or thrown signal).
    Done(Eval),
}

/// Lifecycle of a `with*Continuation` slot. Distinguishing `Resumed` from
/// `Consumed` is what lets the runtime trap on a *late* resume (after the
/// continuation's value has already been read back) the same way it traps on a
/// double resume.
enum ContinuationState {
    /// Handed to the body but not yet resumed.
    Pending,
    /// `resume(...)` stored an outcome that has not been read yet.
    Resumed(Eval),
    /// The enclosing `with*Continuation` already read the value; any further
    /// `resume(...)` is misuse.
    Consumed,
}

/// Seed for the interpreter's SplitMix64 RNG.
///
/// On native targets this is the wall-clock nanosecond count. On
/// `wasm32-unknown-unknown` the std clock is unimplemented and `SystemTime::now()`
/// panics, so a fixed SplitMix64-friendly constant is used instead.
fn initial_rng_seed() -> u64 {
    const FALLBACK: u64 = 0x9E3779B97F4A7C15;
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(FALLBACK)
    }
    #[cfg(target_arch = "wasm32")]
    {
        FALLBACK
    }
}

impl<'w> Interpreter<'w> {
    /// Create an interpreter that writes program output to `out`.
    pub fn new(out: &'w mut dyn Write) -> Self {
        Interpreter {
            out,
            natives: HashMap::new(),
            free_fns: HashMap::new(),
            intrinsics: HashMap::new(),
            properties: HashMap::new(),
            static_methods: HashMap::new(),
            algorithms: HashMap::new(),
            struct_methods: HashMap::new(),
            env: Env::new(),
            type_bindings: Vec::new(),
            builtin_ext_methods: HashMap::new(),
            builtin_ext_computed: HashMap::new(),
            funcs: Vec::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            classes: HashMap::new(),
            protocols: HashMap::new(),
            conformances: HashMap::new(),
            protocol_aliases: HashMap::new(),
            closures: Vec::new(),
            statics: HashMap::new(),
            static_ctx: Vec::new(),
            class_ctx: Vec::new(),
            defer_stack: Vec::new(),
            main_type: None,
            tasks: Vec::new(),
            groups: Vec::new(),
            continuations: Vec::new(),
            filename: "main.swift".into(),
            depth: 0,
            // SplitMix64 tolerates any seed (including 0), so the wall-clock
            // nanos are used as-is rather than forcing the low bit.
            rng_state: initial_rng_seed(),
            type_hint: Vec::new(),
        }
    }

    /// Set the source file name reported by `#file`.
    pub fn set_filename(&mut self, name: &str) {
        self.filename = name.to_string();
    }

    /// Register a native function callable from Swift source by `name`.
    pub fn register_native(&mut self, name: &str, f: NativeFn) {
        self.natives.insert(name.to_string(), f);
    }

    /// Register a free-function intrinsic served through the [`StdContext`] seam.
    pub fn register_free_fn(&mut self, name: &str, f: FreeFn) {
        self.free_fns.insert(name.to_string(), f);
    }

    /// The keys of every registered standard-library entry, for coverage
    /// tooling. Free functions are bare names; method/property intrinsics are
    /// `Type.member`; sequence algorithms are `Sequence.member`. Sorted and
    /// deduplicated so the output is stable.
    pub fn registered_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        keys.extend(self.free_fns.keys().cloned());
        for (recv, name) in self.intrinsics.keys() {
            keys.push(format!("{}.{}", recv.type_name(), name));
        }
        for (recv, name) in self.properties.keys() {
            keys.push(format!("{}.{}", recv.type_name(), name));
        }
        for (recv, name) in self.static_methods.keys() {
            keys.push(format!("{}.{}", recv.type_name(), name));
        }
        for name in self.algorithms.keys() {
            keys.push(format!("Sequence.{name}"));
        }
        keys.sort();
        keys.dedup();
        keys
    }

    /// Register a computed-property intrinsic on a builtin receiver type.
    pub fn register_property(&mut self, recv: BuiltinReceiver, name: &str, f: PropertyFn) {
        self.properties.insert((recv, name.to_string()), f);
    }

    /// Register a static (type-level) method intrinsic on a builtin type.
    pub fn register_static(&mut self, recv: BuiltinReceiver, name: &str, f: StaticFn) {
        self.static_methods.insert((recv, name.to_string()), f);
    }

    /// Register a `Sequence`/`Collection` algorithm by method name.
    pub fn register_algorithm(&mut self, name: &str, f: AlgoFn) {
        self.algorithms.insert(name.to_string(), f);
    }

    /// Register a generic method intrinsic dispatched on any struct receiver by
    /// name (the SwiftUI view-modifier seam). Tried only after user-declared
    /// methods and builtin-receiver intrinsics fail to match, so a user method
    /// of the same name always wins.
    pub fn register_struct_method(&mut self, name: &str, f: StructMethodFn) {
        self.struct_methods.insert(name.to_string(), f);
    }

    /// Instantiate a user struct `type_name` with `args` (label, value) pairs,
    /// the public entry point a render host uses to construct a root `View`.
    pub fn make_struct(
        &mut self,
        type_name: &str,
        args: &[(Option<String>, SwiftValue)],
    ) -> Result<SwiftValue, EvalError> {
        self.instantiate_struct(type_name, args)
            .map_err(signal_eval)
    }

    /// Read `name` from a struct `value` — a stored field or a computed getter
    /// (e.g. a `View`'s `body`). The public counterpart of member access for a
    /// render host driving `body` evaluation.
    pub fn get_member(&mut self, value: &SwiftValue, name: &str) -> Result<SwiftValue, EvalError> {
        self.read_struct_member(value, name).map_err(signal_eval)
    }

    /// Write `new` to `name` on struct `value`, running a computed setter when
    /// one exists. A render host uses this to push a control's new value through
    /// a `Binding` (whose `nonmutating set` stores into a shared reference box),
    /// so the bound `@State` updates. Returns the (possibly rebuilt) value.
    pub fn set_member(
        &mut self,
        value: &SwiftValue,
        name: &str,
        new: SwiftValue,
    ) -> Result<SwiftValue, EvalError> {
        self.set_struct_field(value.clone(), name, new)
            .map_err(signal_eval)
    }

    /// Invoke the closure value with table id `id` and already-evaluated `args`,
    /// the public entry point a render host uses to run an event handler (a
    /// `Button`'s captured `action`).
    pub fn invoke_closure(
        &mut self,
        id: usize,
        args: Vec<SwiftValue>,
    ) -> Result<SwiftValue, EvalError> {
        match Interpreter::call_closure(self, id, args) {
            Ok(v) | Err(Signal::Return(v)) => Ok(v),
            Err(sig) => Err(signal_eval(sig)),
        }
    }

    /// Register a method intrinsic on a builtin receiver type.
    pub fn register_intrinsic(&mut self, recv: BuiltinReceiver, name: &str, entry: MethodEntry) {
        self.intrinsics.insert((recv, name.to_string()), entry);
    }

    /// Map a [`Signal`] escaping a closure call into a [`StdError`] for the seam.
    /// Loop/`return` control flow cannot legitimately cross an intrinsic call.
    fn signal_to_std_error(sig: Signal) -> StdError {
        match sig {
            Signal::Throw(v) => StdError::Throw(v),
            Signal::Error(e) => StdError::Error(e),
            Signal::Return(_) | Signal::Break(_) | Signal::Continue(_) | Signal::Fallthrough => {
                StdError::Error(EvalError::Trap(
                    "control flow escaped a builtin call".into(),
                ))
            }
        }
    }

    /// Lift a [`StdError`] from the seam back into the interpreter's [`Signal`].
    fn std_error_to_signal(err: StdError) -> Signal {
        match err {
            StdError::Throw(v) => Signal::Throw(v),
            StdError::Error(e) => Signal::Error(e),
        }
    }

    /// Dispatch a method call on a builtin receiver through the intrinsic
    /// registry, if one is registered. Returns `None` when no intrinsic matches
    /// so the caller can fall through to the existing ad-hoc paths.
    fn dispatch_intrinsic(
        &mut self,
        recv_value: SwiftValue,
        method: &str,
        args: Vec<SwiftValue>,
        base_place: Option<Place>,
    ) -> Option<Eval> {
        let kind = BuiltinReceiver::of(&recv_value)?;
        let entry = *self.intrinsics.get(&(kind, method.to_string()))?;
        let outcome = (entry.func)(self, recv_value, args);
        Some(match outcome {
            Ok(Outcome { result, receiver }) => {
                if entry.mutating {
                    if let Some(place) = base_place {
                        if let Err(sig) = self.write_place(&place, receiver) {
                            return Some(Err(sig));
                        }
                    }
                }
                Ok(result)
            }
            Err(err) => Err(Self::std_error_to_signal(err)),
        })
    }

    /// Predeclare `Result<Success, Failure>` as a two-case enum so
    /// `.success`/`.failure` construct and `.get()` can throw.
    fn register_builtin_result(&mut self) {
        self.enums
            .entry("Result".into())
            .or_insert_with(|| EnumDef {
                cases: vec![
                    EnumCaseDef {
                        name: "success".into(),
                        raw: None,
                        payload_types: Vec::new(),
                    },
                    EnumCaseDef {
                        name: "failure".into(),
                        raw: None,
                        payload_types: Vec::new(),
                    },
                ],
                methods: std::collections::HashMap::new(),
                computed: std::collections::HashMap::new(),
            });
    }

    /// Evaluate a fully-analyzed program.
    pub fn run(&mut self, analysis: &'static Analysis) -> Result<(), EvalError> {
        if !analysis.is_ok() {
            let diags = analysis
                .diagnostics()
                .iter()
                .map(|d| format!("  {}:{}: {}", d.line, d.col, d.message))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(EvalError::Analysis(diags));
        }
        self.register_builtin_result();
        let mut outcome = self.eval(&analysis.root());
        // Run the `@main` entry point, if one was declared.
        if outcome.is_ok() {
            if let Some(main_type) = self.main_type.clone() {
                outcome =
                    self.call_struct_method(SwiftValue::Void, &main_type, "main", vec![], None);
            }
        }
        // Drain any spawned-but-unawaited tasks (detached `Task { }`) so their
        // side effects run before the program's outermost scope exits.
        if outcome.is_ok() {
            if let Err(sig) = self.drain_pending_tasks() {
                outcome = Err(sig);
            }
        }
        // Run `deinit` for class instances still alive at program end (LIFO).
        let mut released = self.env.drain_global();
        released.reverse();
        for v in released {
            self.run_deinit(&v);
        }
        match outcome {
            Ok(_) => Ok(()),
            Err(Signal::Error(e)) => Err(e),
            Err(Signal::Throw(v)) => Err(EvalError::Trap(format!("uncaught error: {v}"))),
            Err(Signal::Return(_)) => Ok(()),
            Err(other) => Err(EvalError::Unsupported(format!(
                "stray control flow: {other:?}"
            ))),
        }
    }

    /// Evaluate a node, returning its value (or a propagating [`Signal`]).
    fn eval(&mut self, node: &Node<'static>) -> Eval {
        match node.kind() {
            NodeKind::SourceFile => self.eval_block(node),
            NodeKind::Block => self.eval_scoped_block(node),
            NodeKind::ExprStmt => self.eval_seq(node),
            NodeKind::FuncDecl => Ok(SwiftValue::Void), // hoisted by eval_block
            NodeKind::StructDecl
            | NodeKind::EnumDecl
            | NodeKind::ClassDecl
            | NodeKind::ProtocolDecl
            | NodeKind::ActorDecl
            | NodeKind::ExtensionDecl
            | NodeKind::OperatorDecl
            | NodeKind::PrecedenceGroupDecl
            | NodeKind::TypeAliasDecl
            | NodeKind::ImportDecl => Ok(SwiftValue::Void), // hoisted/ignored
            NodeKind::ClosureExpr => self.eval_closure(node),
            NodeKind::CastExpr => self.eval_cast(node),
            NodeKind::AwaitExpr => self.eval_await(node),
            NodeKind::ReturnStmt => {
                let value = match node.children().next() {
                    Some(e) => self.eval(&e)?,
                    None => SwiftValue::Void,
                };
                Err(Signal::Return(value))
            }
            NodeKind::ThrowStmt => {
                let e = node
                    .children()
                    .next()
                    .ok_or_else(|| EvalError::Unsupported("throw without a value".into()))?;
                let value = self.eval(&e)?;
                Err(Signal::Throw(value))
            }
            NodeKind::DoStmt => self.eval_do(node),
            NodeKind::TryExpr => self.eval_try(node),
            NodeKind::DeferStmt => {
                if let Some(block) = node.children().next() {
                    if let Some(frame) = self.defer_stack.last_mut() {
                        frame.push(block);
                    }
                }
                Ok(SwiftValue::Void)
            }
            NodeKind::IfStmt => self.eval_if(node),
            NodeKind::GuardStmt => self.eval_guard(node),
            NodeKind::WhileStmt => self.eval_while(node),
            NodeKind::RepeatStmt => self.eval_repeat(node),
            NodeKind::ForStmt => self.eval_for(node),
            NodeKind::SwitchStmt => self.eval_switch(node),
            NodeKind::BreakStmt => Err(Signal::Break(node.jump_label())),
            NodeKind::ContinueStmt => Err(Signal::Continue(node.jump_label())),
            NodeKind::FallthroughStmt => Err(Signal::Fallthrough),
            NodeKind::TupleExpr => self.eval_tuple(node),
            NodeKind::LetDecl => self.eval_decl(node, false),
            NodeKind::VarDecl => self.eval_decl(node, true),
            NodeKind::CallExpr => self.eval_call(node),
            NodeKind::BinaryExpr => self.eval_binary(node),
            NodeKind::PrefixExpr => self.eval_unary(node),
            NodeKind::AssignExpr => self.eval_assign(node),
            NodeKind::TernaryExpr => self.eval_ternary(node),
            NodeKind::MemberExpr => self.eval_member(node),
            NodeKind::KeyPathExpr => self.eval_keypath(node),
            NodeKind::IdentExpr => self.eval_ident(node),
            NodeKind::IntegerLiteral => Ok(self.eval_int_literal(node)),
            NodeKind::BoolLiteral => Ok(SwiftValue::Bool(node.bool().unwrap_or(false))),
            NodeKind::FloatLiteral => Ok(SwiftValue::Double(node.float().unwrap_or(0.0))),
            NodeKind::StringLiteral => self.eval_string_literal(node),
            NodeKind::RegexLiteral => self.eval_regex_literal(node),
            NodeKind::NilLiteral => Ok(SwiftValue::Nil),
            NodeKind::CompilerDirective => self.eval_macro(node),
            NodeKind::PostfixExpr => self.eval_force_unwrap(node),
            NodeKind::SubscriptExpr => self.eval_subscript(node),
            NodeKind::ArrayLiteral => self.eval_array_literal(node),
            NodeKind::DictLiteral => self.eval_dict_literal(node),
            other => Err(EvalError::Unsupported(format!("{other:?}")).into()),
        }
    }

    /// A source file: hoist function declarations first so forward references
    /// and mutual recursion resolve, then run statements in the global scope.
    fn eval_block(&mut self, node: &Node<'static>) -> Eval {
        self.hoist(node);
        self.eval_seq(node)
    }

    /// A `{ … }` block: same as [`eval_block`] but in a fresh nested scope so
    /// its local bindings do not leak outward.
    fn eval_scoped_block(&mut self, node: &Node<'static>) -> Eval {
        self.env.push();
        self.defer_stack.push(Vec::new());
        self.hoist(node);
        let r = self.eval_seq(node);
        // Run `defer` blocks LIFO on every exit path (normal, return, throw),
        // while the scope's bindings are still in scope.
        let defers = self.defer_stack.pop().unwrap_or_default();
        for d in defers.iter().rev() {
            let _ = self.eval(d);
        }
        // Run `deinit` for class instances released as the scope exits (LIFO).
        let mut released = self.env.pop_owned();
        released.reverse();
        for v in released {
            self.run_deinit(&v);
        }
        r
    }

    /// `do { … } catch <pattern> { … } …` — run the body; on a thrown error,
    /// dispatch to the first matching catch clause.
    fn eval_do(&mut self, node: &Node<'static>) -> Eval {
        let children: Vec<Node<'static>> = node.children().collect();
        let body = children
            .iter()
            .find(|c| c.kind() == NodeKind::Block)
            .ok_or_else(|| EvalError::Unsupported("do without a body".into()))?;
        let catches: Vec<Node<'static>> = children
            .iter()
            .copied()
            .filter(|c| c.kind() == NodeKind::CatchClause)
            .collect();

        match self.eval(body) {
            Err(Signal::Throw(err)) => {
                for catch in &catches {
                    if let Some(binds) = self.match_catch(catch, &err)? {
                        let cbody = catch
                            .children()
                            .find(|c| c.kind() == NodeKind::Block)
                            .ok_or_else(|| EvalError::Unsupported("catch without a body".into()))?;
                        self.env.push();
                        for (n, v) in &binds {
                            self.env.declare(n, v.clone(), false);
                        }
                        let r = self.eval(&cbody);
                        self.env.pop();
                        return r;
                    }
                }
                Err(Signal::Throw(err)) // unhandled — re-propagate
            }
            other => other,
        }
    }

    /// Whether a catch clause matches `err`, returning any bound names. A
    /// pattern-less `catch` matches everything and binds `error`.
    fn match_catch(
        &mut self,
        catch: &Node<'static>,
        err: &SwiftValue,
    ) -> Result<Option<Vec<(String, SwiftValue)>>, Signal> {
        let pattern = catch.children().find(|c| c.kind() != NodeKind::Block);
        match pattern {
            None => Ok(Some(vec![("error".into(), err.clone())])),
            Some(p) => self.match_pattern(&p, err),
        }
    }

    /// `try expr` / `try? expr` / `try! expr`.
    fn eval_try(&mut self, node: &Node<'static>) -> Eval {
        let kind = node.text();
        let inner = node
            .children()
            .next()
            .ok_or_else(|| EvalError::Unsupported("try without an expression".into()))?;
        let result = self.eval(&inner);
        match kind.as_deref() {
            // `try?` turns a thrown error into nil.
            Some("?") => match result {
                Ok(v) => Ok(v),
                Err(Signal::Throw(_)) => Ok(SwiftValue::Nil),
                Err(other) => Err(other),
            },
            // `try!` traps on a thrown error.
            Some("!") => match result {
                Ok(v) => Ok(v),
                Err(Signal::Throw(e)) => Err(trap(format!("unexpected error: {e}"))),
                Err(other) => Err(other),
            },
            // Plain `try` is transparent; the error propagates.
            _ => result,
        }
    }

    /// Pre-declare function and struct declarations in `node` so forward
    /// references resolve.
    fn hoist(&mut self, node: &Node<'static>) {
        // First pass: type and protocol declarations.
        for child in expand_directives(node) {
            match child.kind() {
                NodeKind::FuncDecl => self.declare_func(&child),
                NodeKind::StructDecl => {
                    self.register_struct(&child);
                    self.register_nested_types(&child);
                }
                NodeKind::EnumDecl => {
                    self.register_enum(&child);
                    self.register_nested_types(&child);
                }
                // An `actor` is a reference type whose isolation is provided
                // for free by our single-threaded executor (ADR-0005), so it is
                // registered exactly like a class.
                NodeKind::ClassDecl | NodeKind::ActorDecl => {
                    self.register_class(&child);
                    self.register_nested_types(&child);
                }
                NodeKind::ProtocolDecl => self.register_protocol(&child),
                NodeKind::TypeAliasDecl => self.register_typealias(&child),
                _ => {}
            }
        }
        // Second pass: extensions (they add to already-registered types).
        for child in expand_directives(node) {
            if child.kind() == NodeKind::ExtensionDecl {
                self.register_extension(&child);
            }
        }
    }

    /// Register type declarations nested inside a type body so they resolve by
    /// their simple name (e.g. `B` referenced inside `A`, or `A.B` qualified).
    fn register_nested_types(&mut self, node: &Node<'static>) {
        // Members are the nominal's direct children; there is no synthesized
        // body block. Non-member children (inherited types, attributes, generic
        // params) fall through each loop's `_ => {}` arm.
        let body = node;
        for member in expand_directives(body) {
            match member.kind() {
                NodeKind::StructDecl => {
                    self.register_struct(&member);
                    self.register_nested_types(&member);
                }
                NodeKind::EnumDecl => {
                    self.register_enum(&member);
                    self.register_nested_types(&member);
                }
                NodeKind::ClassDecl | NodeKind::ActorDecl => {
                    self.register_class(&member);
                    self.register_nested_types(&member);
                }
                _ => {}
            }
        }
    }

    /// Register a `typealias X = A & B` whose right-hand side is a protocol
    /// composition, so conformance to `X` can be expanded to its components.
    fn register_typealias(&mut self, node: &Node<'static>) {
        let Some(name) = node.text() else { return };
        let Some(rhs) = node
            .children()
            .find(|c| c.kind() == NodeKind::TypeRef)
            .and_then(|c| c.text())
        else {
            return;
        };
        if !rhs.contains('&') {
            return;
        }
        let components: Vec<String> = rhs
            .split('&')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !components.is_empty() {
            self.protocol_aliases.insert(name, components);
        }
    }

    /// Record the protocols a type conforms to from its inherited-type
    /// (`TypeRef`) children.
    fn record_conformances(&mut self, type_name: &str, node: &Node<'static>) {
        let conf: Vec<String> = node
            .children()
            .filter(|c| c.kind() == NodeKind::TypeRef)
            .filter_map(|c| c.text())
            .collect();
        if !conf.is_empty() {
            self.conformances
                .entry(type_name.to_string())
                .or_default()
                .extend(conf);
        }
    }

    /// Register a protocol declaration (name + inherited protocols).
    fn register_protocol(&mut self, node: &Node<'static>) {
        let Some(name) = node.text() else { return };
        let inherited: Vec<String> = node
            .children()
            .filter(|c| c.kind() == NodeKind::TypeRef)
            .filter_map(|c| c.text())
            .collect();
        self.protocols.entry(name).or_insert_with(|| ProtoDef {
            inherited,
            methods: std::collections::HashMap::new(),
            computed: std::collections::HashMap::new(),
        });
    }

    /// Register an extension: add its members to the extended type, or — when the
    /// extension targets a protocol — to that protocol's default members. Any
    /// conformances the extension adds are recorded too.
    fn register_extension(&mut self, node: &Node<'static>) {
        let Some(target) = node.text() else { return };
        self.record_conformances(&target, node);
        // Members are the nominal's direct children; there is no synthesized
        // body block. Non-member children (inherited types, attributes, generic
        // params) fall through each loop's `_ => {}` arm.
        let body = node;
        let mut methods = std::collections::HashMap::new();
        let mut computed = std::collections::HashMap::new();
        for member in expand_directives(body) {
            match member.kind() {
                NodeKind::FuncDecl => {
                    if let Some(mname) = member.text() {
                        methods.insert(
                            mname,
                            MethodDef {
                                params: parse_params(&member),
                                body: member.children().find(|c| c.kind() == NodeKind::Block),
                                mutating: member.modifiers() & MOD_MUTATING != 0,
                                generic_params: generic_param_names(&member),
                                is_static: member.modifiers() & MOD_STATIC != 0,
                            },
                        );
                    }
                }
                NodeKind::VarDecl | NodeKind::LetDecl => {
                    if let Some(pname) = member.decl_name() {
                        let acc = member.var_accessors();
                        if acc.is_computed {
                            computed.insert(
                                pname,
                                ComputedProp {
                                    getter: acc.getter_body,
                                    setter: acc.setter_body,
                                    setter_param: acc.setter_param,
                                    setter_nonmutating: acc.setter_nonmutating,
                                    is_static: member.modifiers() & MOD_STATIC != 0,
                                },
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(proto) = self.protocols.get_mut(&target) {
            proto.methods.extend(methods);
            proto.computed.extend(computed);
        } else if let Some(def) = self.structs.get_mut(&target) {
            def.methods.extend(methods);
            def.computed.extend(computed);
        } else if let Some(def) = self.enums.get_mut(&target) {
            def.methods.extend(methods);
            def.computed.extend(computed);
        } else if let Some(def) = self.classes.get_mut(&target) {
            def.methods.extend(methods);
            def.computed.extend(computed);
        } else {
            // Extension on a builtin type (`extension Int`, `extension Array`,
            // `extension String`, …). Store the members so value-typed
            // receivers can dispatch to them.
            self.builtin_ext_methods
                .entry(target.clone())
                .or_default()
                .extend(methods);
            self.builtin_ext_computed
                .entry(target)
                .or_default()
                .extend(computed);
        }
    }

    /// Run a user extension method declared on a builtin type, binding `self`
    /// to the receiver and writing it back through `place` for a `mutating`
    /// method.
    fn call_builtin_ext_method(
        &mut self,
        receiver: SwiftValue,
        type_name: &str,
        method: &str,
        args: Vec<CallArg>,
        place: Option<Place>,
    ) -> Option<Eval> {
        let def = self.builtin_ext_methods.get(type_name)?.get(method)?;
        let params = clone_params(&def.params);
        let body = def.body;
        let mutating = def.mutating;
        if mutating && place.is_none() {
            return Some(Err(EvalError::Type(format!(
                "mutating method `{method}` requires an lvalue receiver"
            ))
            .into()));
        }
        let saved_env = self.env.enter_isolated();
        self.env.declare("self", receiver, true);
        let outcome = match self.bind_params(&params, args) {
            Ok(_) => match body {
                Some(b) => self.eval(&b),
                None => Ok(SwiftValue::Void),
            },
            Err(e) => Err(e),
        };
        let updated_self = self.env.get("self").unwrap_or(SwiftValue::Void);
        self.env.restore(saved_env);
        let result = match outcome {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => Err(e),
        };
        // A `mutating` method copies the updated receiver back, including when
        // it throws (Swift writes `inout self` back on a caught error); only a
        // fatal interpreter trap skips the copy-out.
        if mutating && !matches!(result, Err(Signal::Error(_))) {
            if let Some(place) = place {
                if let Err(e) = self.write_place(&place, updated_self) {
                    return Some(Err(e));
                }
            }
        }
        Some(result)
    }

    /// All protocols a type conforms to, transitively (including protocol
    /// inheritance), for default-implementation lookup.
    fn all_protocols(&self, type_name: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut stack: Vec<String> = self
            .conformances
            .get(type_name)
            .cloned()
            .unwrap_or_default();
        while let Some(p) = stack.pop() {
            // Expand a protocol-composition typealias (`typealias X = A & B`)
            // into its component protocols.
            if let Some(components) = self.protocol_aliases.get(&p) {
                stack.extend(components.iter().cloned());
                continue;
            }
            if result.contains(&p) {
                continue;
            }
            if let Some(def) = self.protocols.get(&p) {
                stack.extend(def.inherited.iter().cloned());
            }
            result.push(p);
        }
        result
    }

    /// A protocol default method for `type_name`'s `method`, if any.
    fn protocol_default_method(
        &self,
        type_name: &str,
        method: &str,
    ) -> Option<(Vec<Param>, Option<Node<'static>>, bool, Vec<String>)> {
        for proto in self.all_protocols(type_name) {
            if let Some(m) = self
                .protocols
                .get(&proto)
                .and_then(|d| d.methods.get(method))
            {
                return Some((
                    clone_params(&m.params),
                    m.body,
                    m.mutating,
                    m.generic_params.clone(),
                ));
            }
        }
        None
    }

    /// Render a value honouring `CustomStringConvertible.description` when the
    /// value's type provides one; otherwise fall back to the plain rendering.
    fn render_description(&mut self, value: &SwiftValue) -> String {
        let described = match value {
            SwiftValue::Struct(o) => self
                .structs
                .get(&o.type_name)
                .is_some_and(|d| d.computed.contains_key("description"))
                .then(|| self.read_struct_member(value, "description").ok())
                .flatten(),
            SwiftValue::Object(o) => {
                let cn = o.borrow().class_name.clone();
                self.class_computed_getter(&cn, "description")
                    .is_some()
                    .then(|| self.read_object_member(value, "description").ok())
                    .flatten()
            }
            SwiftValue::Enum(e) => self
                .enums
                .get(&e.type_name)
                .is_some_and(|d| d.computed.contains_key("description"))
                .then(|| self.read_enum_computed(value, "description").ok().flatten())
                .flatten(),
            _ => None,
        };
        match described {
            Some(SwiftValue::Str(s)) => s,
            _ => value.to_string(),
        }
    }

    /// A protocol default computed getter for `type_name`'s `name`, if any.
    fn protocol_default_getter(&self, type_name: &str, name: &str) -> Option<Node<'static>> {
        for proto in self.all_protocols(type_name) {
            if let Some(c) = self
                .protocols
                .get(&proto)
                .and_then(|d| d.computed.get(name))
            {
                return c.getter;
            }
        }
        None
    }

    /// Register an enum type from its declaration.
    fn register_enum(&mut self, node: &Node<'static>) {
        let Some(name) = node.text() else { return };
        if self.enums.contains_key(&name) {
            return;
        }
        self.record_conformances(&name, node);
        // Members are the nominal's direct children; there is no synthesized
        // body block. Non-member children (inherited types, attributes, generic
        // params) fall through each loop's `_ => {}` arm.
        let body = node;
        // Determine the raw-value backing type from the inherited-type list.
        let raw_kind = node
            .children()
            .filter(|c| c.kind() == NodeKind::TypeRef)
            .find_map(|c| match c.text().as_deref() {
                Some("String") => Some(RawKind::Str),
                Some(t) if IntWidth::from_type_name(t).is_some() => Some(RawKind::Int),
                _ => None,
            });
        let mut next_int: i128 = 0;
        let mut cases = Vec::new();
        let mut methods = std::collections::HashMap::new();
        let mut computed = std::collections::HashMap::new();
        for member in expand_directives(body) {
            match member.kind() {
                // Each `case` element is a flat `EnumCaseDecl(name)`: its
                // expression child (if any) is the raw value (`case c = 1`);
                // its `TypeIdent` children are associated-value types.
                NodeKind::EnumCaseDecl => {
                    let Some(cname) = member.text() else {
                        continue;
                    };
                    let explicit = member
                        .children()
                        .find(|ec| is_expr(ec))
                        .and_then(|n| self.eval(&n).ok());
                    let raw = match raw_kind {
                        Some(RawKind::Int) => {
                            let v = match &explicit {
                                Some(SwiftValue::Int(i)) => i.raw,
                                _ => next_int,
                            };
                            next_int = v + 1;
                            Some(SwiftValue::int(v))
                        }
                        Some(RawKind::Str) => {
                            Some(explicit.unwrap_or_else(|| SwiftValue::Str(cname.clone())))
                        }
                        None => explicit,
                    };
                    let payload_types: Vec<Option<String>> = member
                        .children()
                        .filter(|ec| ec.kind() == NodeKind::TypeRef)
                        .map(|c| c.text())
                        .collect();
                    cases.push(EnumCaseDef {
                        name: cname,
                        raw,
                        payload_types,
                    });
                }
                NodeKind::FuncDecl => {
                    if let Some(mname) = member.text() {
                        methods.insert(
                            mname,
                            MethodDef {
                                params: parse_params(&member),
                                body: member.children().find(|c| c.kind() == NodeKind::Block),
                                mutating: member.modifiers() & MOD_MUTATING != 0,
                                generic_params: generic_param_names(&member),
                                is_static: member.modifiers() & MOD_STATIC != 0,
                            },
                        );
                    }
                }
                NodeKind::VarDecl | NodeKind::LetDecl => {
                    if let Some(pname) = member.decl_name() {
                        let acc = member.var_accessors();
                        if acc.is_computed {
                            computed.insert(
                                pname,
                                ComputedProp {
                                    getter: acc.getter_body,
                                    setter: acc.setter_body,
                                    setter_param: acc.setter_param,
                                    setter_nonmutating: acc.setter_nonmutating,
                                    is_static: member.modifiers() & MOD_STATIC != 0,
                                },
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        self.enums.insert(
            name,
            EnumDef {
                cases,
                methods,
                computed,
            },
        );
    }

    /// Register a class type from its declaration.
    fn register_class(&mut self, node: &Node<'static>) {
        let Some(name) = node.text() else { return };
        if self.classes.contains_key(&name) {
            return;
        }
        self.record_conformances(&name, node);
        let superclass = node
            .children()
            .find(|c| c.kind() == NodeKind::TypeRef)
            .and_then(|c| c.text());
        // Members are the nominal's direct children; there is no synthesized
        // body block. Non-member children (inherited types, attributes, generic
        // params) fall through each loop's `_ => {}` arm.
        let body = node;
        let mut stored = Vec::new();
        let mut weak_fields = Vec::new();
        let mut computed = std::collections::HashMap::new();
        let mut methods = std::collections::HashMap::new();
        let mut init = None;
        let mut init_overloads = Vec::new();
        let mut deinit = None;
        let mut static_subscript = None;
        let mut static_inits: Vec<(String, Node<'static>)> = Vec::new();

        for member in expand_directives(body) {
            match member.kind() {
                NodeKind::InitDecl => {
                    let def = MethodDef {
                        params: parse_params(&member),
                        body: member.children().find(|c| c.kind() == NodeKind::Block),
                        mutating: false,
                        generic_params: generic_param_names(&member),
                        is_static: false,
                    };
                    init_overloads.push(clone_method(&def));
                    init = Some(def);
                }
                NodeKind::SubscriptDecl if member.modifiers() & MOD_STATIC != 0 => {
                    let acc = member.var_accessors();
                    let sbody = acc
                        .getter_body
                        .or_else(|| member.children().find(|c| c.kind() == NodeKind::Block));
                    static_subscript = Some(MethodDef {
                        params: parse_params(&member),
                        body: sbody,
                        mutating: false,
                        generic_params: generic_param_names(&member),
                        is_static: true,
                    });
                }
                NodeKind::DeinitDecl => {
                    deinit = member.children().find(|c| c.kind() == NodeKind::Block);
                }
                NodeKind::FuncDecl => {
                    if let Some(mname) = member.text() {
                        methods.insert(
                            mname,
                            MethodDef {
                                params: parse_params(&member),
                                body: member.children().find(|c| c.kind() == NodeKind::Block),
                                mutating: false,
                                generic_params: generic_param_names(&member),
                                is_static: member.modifiers() & MOD_STATIC != 0,
                            },
                        );
                    }
                }
                NodeKind::VarDecl | NodeKind::LetDecl => {
                    let Some(pname) = member.decl_name() else {
                        continue;
                    };
                    let acc = member.var_accessors();
                    if acc.is_computed {
                        computed.insert(
                            pname,
                            ComputedProp {
                                getter: acc.getter_body,
                                setter: acc.setter_body,
                                setter_param: acc.setter_param,
                                setter_nonmutating: acc.setter_nonmutating,
                                is_static: member.modifiers() & MOD_STATIC != 0,
                            },
                        );
                    } else if member.modifiers() & MOD_STATIC != 0 {
                        // A `static` stored property is type-level storage; defer
                        // its initializer until the class is registered so it can
                        // reference its own type.
                        if let Some(def) = member.children().find(|c| is_value_node(c)) {
                            static_inits.push((pname.clone(), def));
                        }
                    } else {
                        if member.modifiers() & MOD_WEAK != 0
                            || member.ownership().as_deref() == Some("weak")
                        {
                            weak_fields.push(pname.clone());
                        }
                        let default = member.children().find(|c| is_value_node(c));
                        let will_set = acc.will_set_body.map(|b| {
                            (
                                acc.will_set_param
                                    .clone()
                                    .unwrap_or_else(|| "newValue".into()),
                                b,
                            )
                        });
                        let did_set = acc.did_set_body.map(|b| {
                            (
                                acc.did_set_param
                                    .clone()
                                    .unwrap_or_else(|| "oldValue".into()),
                                b,
                            )
                        });
                        stored.push(StoredProp {
                            name: pname,
                            ty: field_type_name(&member),
                            default,
                            lazy: member.modifiers() & MOD_LAZY != 0,
                            will_set,
                            did_set,
                        });
                    }
                }
                _ => {}
            }
        }
        self.classes.insert(
            name.clone(),
            ClassDef {
                superclass,
                stored,
                weak_fields,
                computed,
                methods,
                init,
                init_overloads,
                deinit,
                static_subscript,
            },
        );
        // Evaluate static stored-property initializers now the class exists.
        for (pname, def) in static_inits {
            if let Ok(v) = self.eval(&def) {
                self.statics.insert(format!("{name}.{pname}"), v);
            }
        }
    }

    /// Register a struct type from its declaration.
    fn register_struct(&mut self, node: &Node<'static>) {
        let Some(name) = node.text() else { return };
        if self.structs.contains_key(&name) {
            return;
        }
        self.record_conformances(&name, node);
        // `@main` attribute marks the program entry point.
        if node
            .children()
            .any(|c| c.kind() == NodeKind::Attribute && c.text().as_deref() == Some("main"))
        {
            self.main_type = Some(name.clone());
        }
        // `@dynamicMemberLookup` routes unresolved member access through the
        // type's `subscript(dynamicMember:)`.
        let dynamic_member_lookup = node.children().any(|c| {
            c.kind() == NodeKind::Attribute && c.text().as_deref() == Some("dynamicMemberLookup")
        });
        // `@dynamicCallable` routes call syntax through the type's
        // `dynamicallyCall(...)` method.
        let dynamic_callable = node.children().any(|c| {
            c.kind() == NodeKind::Attribute && c.text().as_deref() == Some("dynamicCallable")
        });
        // Members are the nominal's direct children; there is no synthesized
        // body block. Non-member children (inherited types, attributes, generic
        // params) fall through each loop's `_ => {}` arm.
        let body = node;
        let mut stored = Vec::new();
        let mut computed = std::collections::HashMap::new();
        let mut methods = std::collections::HashMap::new();
        let mut method_overloads: std::collections::HashMap<String, Vec<MethodDef>> =
            std::collections::HashMap::new();
        let mut wrappers = std::collections::HashMap::new();
        let mut subscripts: Vec<SubscriptDef> = Vec::new();
        let mut static_subscript = None;
        let mut init = None;
        let mut init_overloads = Vec::new();
        let mut static_inits: Vec<(String, Node<'static>)> = Vec::new();

        for member in expand_directives(body) {
            match member.kind() {
                NodeKind::InitDecl => {
                    let def = MethodDef {
                        params: parse_params(&member),
                        body: member.children().find(|c| c.kind() == NodeKind::Block),
                        mutating: true,
                        generic_params: generic_param_names(&member),
                        is_static: false,
                    };
                    init_overloads.push(clone_method(&def));
                    init = Some(def);
                }
                NodeKind::SubscriptDecl => {
                    let acc = member.var_accessors();
                    let getter = acc
                        .getter_body
                        .or_else(|| member.children().find(|c| c.kind() == NodeKind::Block));
                    if member.modifiers() & MOD_STATIC != 0 {
                        static_subscript = Some(MethodDef {
                            params: parse_params(&member),
                            body: getter,
                            mutating: false,
                            generic_params: generic_param_names(&member),
                            is_static: true,
                        });
                    } else {
                        subscripts.push(SubscriptDef {
                            params: parse_params(&member),
                            getter,
                            setter: acc.setter_body,
                            setter_param: acc
                                .setter_param
                                .unwrap_or_else(|| "newValue".to_string()),
                        });
                    }
                }
                NodeKind::FuncDecl => {
                    if let Some(mname) = member.text() {
                        let mods = member.modifiers();
                        let params = parse_params(&member);
                        let body = member.children().find(|c| c.kind() == NodeKind::Block);
                        let mutating = mods & MOD_MUTATING != 0;
                        let is_static = mods & MOD_STATIC != 0;
                        let def = MethodDef {
                            params,
                            body,
                            mutating,
                            generic_params: generic_param_names(&member),
                            is_static,
                        };
                        method_overloads
                            .entry(mname.clone())
                            .or_default()
                            .push(clone_method(&def));
                        methods.insert(mname, def);
                    }
                }
                NodeKind::VarDecl | NodeKind::LetDecl => {
                    let Some(pname) = member.decl_name() else {
                        continue;
                    };
                    let mods = member.modifiers();
                    let is_static = mods & MOD_STATIC != 0;
                    let acc = member.var_accessors();
                    if acc.is_computed {
                        computed.insert(
                            pname,
                            ComputedProp {
                                getter: acc.getter_body,
                                setter: acc.setter_body,
                                setter_param: acc.setter_param,
                                setter_nonmutating: acc.setter_nonmutating,
                                is_static,
                            },
                        );
                    } else {
                        if let Some(attr) = member
                            .children()
                            .find(|c| c.kind() == NodeKind::Attribute)
                            .and_then(|a| a.text())
                        {
                            wrappers.insert(pname.clone(), attr);
                        }
                        let default = member.children().find(|c| is_value_node(c));
                        let will_set = acc.will_set_body.map(|b| {
                            (
                                acc.will_set_param
                                    .clone()
                                    .unwrap_or_else(|| "newValue".into()),
                                b,
                            )
                        });
                        let did_set = acc.did_set_body.map(|b| {
                            (
                                acc.did_set_param
                                    .clone()
                                    .unwrap_or_else(|| "oldValue".into()),
                                b,
                            )
                        });
                        if is_static {
                            // Defer evaluation until after the type is
                            // registered so a static like `static let red =
                            // Color(...)` can reference its own type.
                            if let Some(def) = default {
                                static_inits.push((pname.clone(), def));
                            }
                        } else {
                            stored.push(StoredProp {
                                name: pname,
                                ty: field_type_name(&member),
                                default,
                                lazy: mods & MOD_LAZY != 0,
                                will_set,
                                did_set,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        self.structs.insert(
            name.clone(),
            StructDef {
                stored,
                computed,
                methods,
                method_overloads,
                subscripts,
                static_subscript,
                init,
                init_overloads,
                wrappers,
                dynamic_member_lookup,
                dynamic_callable,
            },
        );
        // Now that the struct is registered, evaluate its static initializers
        // (which may construct instances of the type itself).
        for (pname, def) in static_inits {
            if let Ok(v) = self.eval(&def) {
                self.statics.insert(format!("{name}.{pname}"), v);
            }
        }
    }

    /// A tuple expression `(a, b, …)`.
    fn eval_tuple(&mut self, node: &Node<'static>) -> Eval {
        let mut items = Vec::new();
        let mut labels = Vec::new();
        for child in node.children() {
            labels.push(child.arg_label());
            items.push(self.eval(&child)?);
        }
        Ok(SwiftValue::Tuple(items, labels))
    }

    /// Evaluate each child in order, yielding the last value.
    fn eval_seq(&mut self, node: &Node<'static>) -> Eval {
        let mut last = SwiftValue::Void;
        for child in expand_directives(node) {
            last = self.eval(&child)?;
        }
        Ok(last)
    }

    /// Register a function declaration as a first-class value in the current
    /// scope, capturing the enclosing scope chain.
    fn declare_func(&mut self, node: &Node<'static>) {
        let Some(name) = node.text() else {
            return;
        };
        // Avoid double-hoisting if eval_block runs twice on the same node.
        if matches!(self.env.get(&name), Some(SwiftValue::Function(_))) {
            return;
        }
        let params = parse_params(node);
        let body = node.children().find(|c| c.kind() == NodeKind::Block);
        let ret_tuple_labels = node
            .children()
            .find(|c| c.kind() == NodeKind::TypeRef)
            .and_then(|t| t.text())
            .and_then(|t| tuple_type_labels(&t));
        let captured = self.env.capture();
        let generic_params = generic_param_names(node);
        let id = self.funcs.len();
        self.funcs.push(FuncDef {
            params,
            body,
            captured,
            generic_params,
            ret_tuple_labels,
        });
        self.env.declare(&name, SwiftValue::Function(id), false);
    }

    /// `let`/`var name [= init]`, including tuple decomposition
    /// `let (a, b) = pair`.
    fn eval_decl(&mut self, node: &Node<'static>, mutable: bool) -> Eval {
        let children: Vec<Node<'static>> = node.children().collect();

        // The initializer is the last value child. A trailing declaration
        // `Attribute` (e.g. `@usableFromInline let g = 5`) is not a value, so
        // search from the end for the actual expression.
        let init_expr = children.iter().rev().find(|c| is_expr(c)).copied();

        // Tuple-pattern binding: `let (a, b) = expr`.
        if let Some(pat) = children.iter().find(|c| c.kind() == NodeKind::TuplePattern) {
            let init = init_expr.ok_or_else(|| {
                EvalError::Unsupported("tuple binding without initializer".into())
            })?;
            let value = self.eval(&init)?;
            self.bind_tuple_pattern(pat, &value, mutable)?;
            return Ok(SwiftValue::Void);
        }

        let name = node
            .decl_name()
            .ok_or_else(|| EvalError::Unsupported("declaration without a name".into()))?;

        // `async let name = expr` spawns a child task; the binding holds its
        // handle and `await name` later retrieves the result (ADR-0005).
        if node.is_async_let() {
            if let Some(init) = init_expr {
                let id = self.spawn_expr_task(init);
                self.env.declare(&name, SwiftValue::Task(id), mutable);
                return Ok(SwiftValue::Void);
            }
        }

        let value = match init_expr {
            Some(init) => {
                let v = self.eval(&init)?;
                let v = self.coerce_to_literal_type(node, v)?;
                self.coerce_to_decl_type(node, v)
            }
            None => SwiftValue::Void,
        };
        self.env.declare(&name, value, mutable);
        Ok(SwiftValue::Void)
    }

    /// If the declaration is annotated with a user type that conforms to an
    /// `ExpressibleBy*Literal` protocol and the initializer is the matching
    /// literal kind, build the value through that type's literal initializer
    /// (`let s: Stack = [1, 2, 3]` → `Stack(arrayLiteral: 1, 2, 3)`).
    fn coerce_to_literal_type(&mut self, node: &Node<'static>, value: SwiftValue) -> Eval {
        let Some(ty) = node
            .children()
            .find(|c| c.kind() == NodeKind::TypeRef)
            .and_then(|c| c.text())
        else {
            return Ok(value);
        };
        // Conversion only applies to *literal syntax*, never to an arbitrary
        // expression that happens to evaluate to a matching value kind.
        let Some(init_kind) = node
            .children()
            .filter(|c| is_expr(c))
            .last()
            .and_then(|c| literal_syntax_kind(&c))
        else {
            return Ok(value);
        };
        let ty = ty.trim();
        // An optional annotation (`T?`): a `nil` literal stays the absent
        // optional rather than constructing `T(nilLiteral:)`.
        let optional = ty.ends_with('?');
        if optional && init_kind == NodeKind::NilLiteral {
            return Ok(value);
        }
        let ty = ty.trim_end_matches('?').trim();
        let is_user_type = self.structs.contains_key(ty) || self.classes.contains_key(ty);
        if !is_user_type {
            return Ok(value);
        }
        self.coerce_literal_value(ty, init_kind, value)
    }

    /// Convert a literal value into the contextual user type named by `ty` when
    /// that type declares the matching `ExpressibleBy*Literal` conformance.
    fn coerce_literal_value(&mut self, ty: &str, init_kind: NodeKind, value: SwiftValue) -> Eval {
        let is_user_type = self.structs.contains_key(ty) || self.classes.contains_key(ty);
        if !is_user_type {
            return Ok(value);
        }
        // The literal protocol implied by the initializer's syntax, plus the
        // argument label and positional argument(s) its initializer takes.
        let (proto, label, args): (&str, &str, Vec<SwiftValue>) = match init_kind {
            NodeKind::ArrayLiteral => match &value {
                SwiftValue::Array(items) => (
                    "ExpressibleByArrayLiteral",
                    "arrayLiteral",
                    items.as_ref().clone(),
                ),
                _ => return Ok(value),
            },
            NodeKind::DictLiteral => match &value {
                SwiftValue::Dict(pairs) => (
                    "ExpressibleByDictionaryLiteral",
                    "dictionaryLiteral",
                    pairs
                        .iter()
                        .map(|(k, v)| {
                            SwiftValue::Tuple(vec![k.clone(), v.clone()], vec![None, None])
                        })
                        .collect(),
                ),
                _ => return Ok(value),
            },
            NodeKind::StringLiteral => (
                "ExpressibleByStringLiteral",
                "stringLiteral",
                vec![value.clone()],
            ),
            NodeKind::IntegerLiteral => (
                "ExpressibleByIntegerLiteral",
                "integerLiteral",
                vec![value.clone()],
            ),
            NodeKind::FloatLiteral => (
                "ExpressibleByFloatLiteral",
                "floatLiteral",
                vec![value.clone()],
            ),
            NodeKind::BoolLiteral => (
                "ExpressibleByBooleanLiteral",
                "booleanLiteral",
                vec![value.clone()],
            ),
            NodeKind::NilLiteral => (
                "ExpressibleByNilLiteral",
                "nilLiteral",
                vec![SwiftValue::Void],
            ),
            _ => return Ok(value),
        };
        if !self.all_protocols(ty).iter().any(|p| p == proto) {
            return Ok(value);
        }
        let call_args: Vec<(Option<String>, SwiftValue)> = args
            .into_iter()
            .map(|v| (Some(label.to_string()), v))
            .collect();
        if self.structs.contains_key(ty) {
            self.instantiate_struct(ty, &call_args)
        } else {
            let call_args = call_args
                .into_iter()
                .map(|(label, value)| CallArg {
                    label,
                    value,
                    place: None,
                })
                .collect();
            self.instantiate_class(ty, call_args)
        }
    }

    /// Bind the names in a tuple pattern to the elements of a tuple value.
    fn bind_tuple_pattern(
        &mut self,
        pattern: &Node<'static>,
        value: &SwiftValue,
        mutable: bool,
    ) -> Result<(), Signal> {
        let SwiftValue::Tuple(items, _) = value else {
            return Err(EvalError::Type(format!(
                "cannot destructure {} as a tuple",
                value.type_name()
            ))
            .into());
        };
        let elems: Vec<Node<'static>> = pattern.children().collect();
        for (sub, item) in elems.iter().zip(items.iter()) {
            match sub.kind() {
                NodeKind::WildcardPattern => {}
                NodeKind::TuplePattern => self.bind_tuple_pattern(sub, item, mutable)?,
                _ => {
                    if let Some(name) = sub.text() {
                        if name != "_" {
                            self.env.declare(&name, item.clone(), mutable);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// If the declaration carries an explicit integer type annotation, retag the
    /// initializer's width to match it. (msf collapses fixed-width ints to
    /// `Int`, so the `TYPE_IDENT` node is the only reliable source.)
    fn coerce_to_decl_type(&self, node: &Node<'static>, value: SwiftValue) -> SwiftValue {
        // A `Set<…>`-annotated array literal becomes a deduplicated set.
        if let SwiftValue::Array(items) = &value {
            for child in node.children() {
                if child.kind() == NodeKind::TypeRef
                    && child
                        .text()
                        .as_deref()
                        .is_some_and(|t| t.starts_with("Set<") || t == "Set")
                {
                    return SwiftValue::Set(StdRc::new(dedup_preserving_order(
                        items.as_ref().clone(),
                    )));
                }
            }
            return value;
        }
        let SwiftValue::Int(i) = &value else {
            return value;
        };
        for child in node.children() {
            if child.kind() == NodeKind::TypeRef {
                let ty = child.text();
                // An integer literal in a `Double`/`Float` context coerces to
                // floating point (`let r: Double = 5`).
                if matches!(ty.as_deref(), Some("Double") | Some("Float")) {
                    return SwiftValue::Double(i.raw as f64);
                }
                if let Some(w) = ty.as_deref().and_then(IntWidth::from_type_name) {
                    return SwiftValue::Int(IntValue::new(i.raw, w));
                }
            }
        }
        value
    }

    /// An identifier reference: look up a binding, falling back to an implicit
    /// `self.<name>` member when evaluating inside a method.
    fn eval_ident(&mut self, node: &Node<'static>) -> Eval {
        let name = node
            .text()
            .ok_or_else(|| EvalError::Unsupported("unnamed identifier".into()))?;
        // Swift resolution order inside a method: local variables/parameters,
        // then the enclosing type's members (which shadow module globals),
        // then the module/global scope.
        if let Some(v) = self.env.get_local(&name) {
            return Ok(v);
        }
        if let Some(v) = self.implicit_self_member(&name)? {
            return Ok(v);
        }
        // Inside a `static` method, an unqualified name may be a static property
        // of the enclosing type (stored, then computed).
        if let Some(v) = self.implicit_static_member(&name) {
            return Ok(v);
        }
        if let Some(ty) = self.static_ctx.last().cloned() {
            if let Some(v) = self.read_static_computed(&ty, &name)? {
                return Ok(v);
            }
        }
        if let Some(v) = self.env.get_global(&name) {
            return Ok(v);
        }
        // A bare operator used as a value (`reduce(0, +)`, `sorted(by: >)`):
        // synthesize an operator-function closure the standard-library
        // algorithms can call back into.
        if is_operator_name(&name) {
            let id = self.closures.len();
            self.closures.push((ClosureDef::Operator(name), Vec::new()));
            return Ok(SwiftValue::Closure(id));
        }
        Err(EvalError::UnknownVariable(name).into())
    }

    /// If executing inside a `static` method, read `name` as a static property
    /// of the enclosing type (`Type.name`), if such a static exists.
    fn implicit_static_member(&self, name: &str) -> Option<SwiftValue> {
        let ty = self.static_ctx.last()?;
        self.statics.get(&format!("{ty}.{name}")).cloned()
    }

    /// The `statics` key for an unqualified `name` referencing a static property
    /// of the enclosing `static` method's type, when one exists.
    fn implicit_static_key(&self, name: &str) -> Option<String> {
        let ty = self.static_ctx.last()?;
        let key = format!("{ty}.{name}");
        self.statics.contains_key(&key).then_some(key)
    }

    /// If `name` is a property of the current `self`, read it. Covers struct
    /// stored/computed members and enum `rawValue`/computed members.
    fn implicit_self_member(&mut self, name: &str) -> Result<Option<SwiftValue>, Signal> {
        let Some(this) = self.env.get("self") else {
            return Ok(None);
        };
        match &this {
            SwiftValue::Struct(obj) => {
                // A bare projected reference `$name` inside a method resolves to
                // the wrapped property's `projectedValue` (e.g. `$flag` for a
                // `@State var flag`).
                if let Some(stripped) = name.strip_prefix('$') {
                    if self.wrapped_field(&obj.type_name, stripped) {
                        return Ok(Some(self.read_struct_member(&this, name)?));
                    }
                }
                if obj.get(name).is_some() || self.struct_has_member(&obj.type_name, name) {
                    Ok(Some(self.read_struct_member(&this, name)?))
                } else {
                    Ok(None)
                }
            }
            SwiftValue::Object(obj) => {
                let class_name = obj.borrow().class_name.clone();
                if obj.borrow().get(name).is_some() || self.class_has_member(&class_name, name) {
                    Ok(Some(self.read_object_member(&this, name)?))
                } else {
                    Ok(None)
                }
            }
            SwiftValue::Enum(e) => {
                if name == "rawValue" {
                    return Ok(Some(self.enum_raw_value(&e.type_name, &e.case)?));
                }
                self.read_enum_computed(&this, name)
            }
            // A builtin self (`extension Array { … self.count … }`): resolve an
            // unqualified property name through the builtin property registry,
            // then any user extension computed property on that type.
            _ => {
                if let Some(kind) = BuiltinReceiver::of(&this) {
                    if let Some(func) = self.properties.get(&(kind, name.to_string())).copied() {
                        return func(this).map(Some).map_err(Self::std_error_to_signal);
                    }
                }
                let tn = this.type_name();
                if let Some(body) = self
                    .builtin_ext_computed
                    .get(&tn)
                    .and_then(|m| m.get(name))
                    .and_then(|c| c.getter)
                {
                    return self
                        .run_with_self(this.clone(), |me| me.eval(&body))
                        .map(|(v, _)| Some(v));
                }
                Ok(None)
            }
        }
    }

    /// Whether `name` spells a known type — a user struct/class/enum/protocol
    /// or a builtin value type — so `Type.self` resolves to a metatype.
    fn is_type_name(&self, name: &str) -> bool {
        self.structs.contains_key(name)
            || self.classes.contains_key(name)
            || self.enums.contains_key(name)
            || self.protocols.contains_key(name)
            || IntWidth::from_type_name(name).is_some()
            || matches!(
                name,
                "Int"
                    | "UInt"
                    | "Double"
                    | "Float"
                    | "Bool"
                    | "String"
                    | "Character"
                    | "Array"
                    | "Dictionary"
                    | "Set"
                    | "Data"
                    | "UUID"
            )
    }

    /// Whether `name` names a stored or computed member of the enclosing
    /// `self`, used to resolve implicit `self.<name>` references.
    fn is_self_member(&self, name: &str) -> bool {
        match self.env.get("self") {
            Some(SwiftValue::Struct(obj)) => {
                obj.get(name).is_some() || self.struct_has_member(&obj.type_name, name)
            }
            Some(SwiftValue::Object(obj)) => {
                let class_name = obj.borrow().class_name.clone();
                obj.borrow().get(name).is_some() || self.class_has_member(&class_name, name)
            }
            _ => false,
        }
    }

    /// Construct an enum case value if `case` names a case of `type_name`.
    /// Returns `Ok(None)` if the name is not a case.
    fn make_enum_case(
        &mut self,
        type_name: &str,
        case: &str,
        payload: Vec<SwiftValue>,
    ) -> Result<Option<SwiftValue>, Signal> {
        let payload_types = self.enums.get(type_name).and_then(|d| {
            d.cases
                .iter()
                .find(|c| c.name == case)
                .map(|c| c.payload_types.clone())
        });
        let Some(payload_types) = payload_types else {
            return Ok(None);
        };
        // Coerce integer literals into floating associated-value slots
        // (`.circle(radius: 5)` where `radius: Double`).
        let payload = payload
            .into_iter()
            .enumerate()
            .map(|(i, v)| coerce_numeric(v, payload_types.get(i).and_then(|t| t.as_deref())))
            .collect();
        Ok(Some(SwiftValue::Enum(Rc::new(EnumObj {
            type_name: type_name.to_string(),
            case: case.to_string(),
            payload,
        }))))
    }

    /// The active call-site contextual type, if an argument is currently being
    /// evaluated under a known parameter type (top of the hint stack).
    fn contextual_type(&self) -> Option<&str> {
        self.type_hint.last().and_then(|o| o.as_deref())
    }

    /// Resolve the enum type for a shorthand `.case` member from the resolved
    /// type or call-site contextual type, falling back to the unique enum
    /// declaring that case.
    fn resolve_member_enum(&self, member: &Node<'static>, case: &str) -> Option<String> {
        // The member's resolved type (the enum or a function returning it), then
        // the call-site contextual type; match a registered enum name within.
        for ty in member
            .type_name()
            .into_iter()
            .chain(self.contextual_type().map(String::from))
        {
            for name in self.enums.keys() {
                if ty
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|t| t == name)
                    && self.enum_has_case(name, case)
                {
                    return Some(name.clone());
                }
            }
        }
        // Fall back: a single enum declaring this case name.
        let mut found = None;
        for (name, def) in &self.enums {
            if def.cases.iter().any(|c| c.name == case) {
                if found.is_some() {
                    return None; // ambiguous
                }
                found = Some(name.clone());
            }
        }
        found
    }

    /// Resolve an implicit-member `.name` to a static property. Prefers the
    /// member node's inferred contextual type; otherwise accepts a unique
    /// registered static whose member name matches.
    fn resolve_implicit_static(&self, node: &Node<'static>, name: &str) -> Option<SwiftValue> {
        // The node's inferred type, then the call-site contextual type.
        for ty in node
            .type_name()
            .into_iter()
            .chain(self.contextual_type().map(String::from))
        {
            for type_name in ty.split(|c: char| !c.is_alphanumeric() && c != '_') {
                if let Some(v) = self.statics.get(&format!("{type_name}.{name}")) {
                    return Some(v.clone());
                }
            }
        }
        // Otherwise, a unique `Type.name` static across all registered types.
        let suffix = format!(".{name}");
        let mut found: Option<&SwiftValue> = None;
        for (key, value) in &self.statics {
            if key.ends_with(&suffix) {
                if found.is_some() {
                    return None; // ambiguous
                }
                found = Some(value);
            }
        }
        found.cloned()
    }

    /// Resolve an implicit-member call `.m(...)` to the contextual type that
    /// declares a `static`/`class` method named `m`. Prefers the node's
    /// inferred type; otherwise accepts a unique struct/class/enum that
    /// declares such a static method.
    fn resolve_implicit_static_method(&self, node: &Node<'static>, method: &str) -> Option<String> {
        let declares = |type_name: &str| -> bool {
            let m = self
                .structs
                .get(type_name)
                .map(|d| &d.methods)
                .or_else(|| self.classes.get(type_name).map(|d| &d.methods))
                .or_else(|| self.enums.get(type_name).map(|d| &d.methods));
            m.and_then(|methods| methods.get(method))
                .is_some_and(|def| def.is_static)
        };
        // The node's inferred type, then the call-site contextual type.
        for ty in node
            .type_name()
            .into_iter()
            .chain(self.contextual_type().map(String::from))
        {
            for type_name in ty.split(|c: char| !c.is_alphanumeric() && c != '_') {
                if declares(type_name) {
                    return Some(type_name.to_string());
                }
            }
        }
        // Otherwise, a unique type declaring this static method.
        let mut found: Option<String> = None;
        let names = self
            .structs
            .keys()
            .chain(self.classes.keys())
            .chain(self.enums.keys());
        for name in names {
            if declares(name) {
                if found.is_some() {
                    return None; // ambiguous
                }
                found = Some(name.clone());
            }
        }
        found
    }

    /// Whether `name` is a case of enum `type_name`.
    fn enum_has_case(&self, type_name: &str, name: &str) -> bool {
        self.enums
            .get(type_name)
            .is_some_and(|d| d.cases.iter().any(|c| c.name == name))
    }

    /// The `rawValue` of an enum case (precomputed at registration).
    fn enum_raw_value(&mut self, type_name: &str, case: &str) -> Eval {
        let raw = self
            .enums
            .get(type_name)
            .and_then(|d| d.cases.iter().find(|c| c.name == case))
            .and_then(|c| c.raw.clone());
        raw.ok_or_else(|| EvalError::Type(format!("{type_name}.{case} has no raw value")).into())
    }

    /// All cases of an enum as an array (`CaseIterable.allCases`).
    fn enum_all_cases(&mut self, type_name: &str) -> Eval {
        let names: Vec<String> = self
            .enums
            .get(type_name)
            .map(|d| d.cases.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default();
        let items = names
            .into_iter()
            .map(|name| {
                SwiftValue::Enum(Rc::new(EnumObj {
                    type_name: type_name.to_string(),
                    case: name,
                    payload: Vec::new(),
                }))
            })
            .collect();
        Ok(SwiftValue::Array(Rc::new(items)))
    }

    /// Read a `static`/`class` computed property `Type.prop`, running its getter
    /// with no instance `self` and the type recorded as the static context.
    fn read_static_computed(
        &mut self,
        type_name: &str,
        member: &str,
    ) -> Result<Option<SwiftValue>, Signal> {
        let getter = self
            .structs
            .get(type_name)
            .map(|d| &d.computed)
            .or_else(|| self.classes.get(type_name).map(|d| &d.computed))
            .or_else(|| self.enums.get(type_name).map(|d| &d.computed))
            .and_then(|c| c.get(member))
            .filter(|c| c.is_static)
            .and_then(|c| c.getter);
        let Some(body) = getter else {
            return Ok(None);
        };
        // Guard against unbounded recursion (e.g. `static var x: Int { x }`),
        // which would otherwise overflow the native stack with no interpreter
        // trap.
        self.depth += 1;
        if self.depth > MAX_CALL_DEPTH {
            self.depth -= 1;
            return Err(trap(
                "stack overflow: recursion exceeded the maximum call depth".into(),
            ));
        }
        self.static_ctx.push(type_name.to_string());
        let saved_env = self.env.enter_isolated();
        // A type-level getter has no instance `self`; shadow any enclosing one
        // so unqualified names resolve against the type, not a caller instance.
        self.env.declare("self", SwiftValue::Void, false);
        let result = self.eval(&body);
        self.env.restore(saved_env);
        self.static_ctx.pop();
        self.depth -= 1;
        match result {
            Ok(v) => Ok(Some(v)),
            Err(Signal::Return(v)) => Ok(Some(v)),
            Err(e) => Err(e),
        }
    }

    /// Read a computed property off an enum value, if declared.
    fn read_enum_computed(
        &mut self,
        value: &SwiftValue,
        name: &str,
    ) -> Result<Option<SwiftValue>, Signal> {
        let SwiftValue::Enum(e) = value else {
            return Ok(None);
        };
        let getter = self
            .enums
            .get(&e.type_name)
            .and_then(|d| d.computed.get(name))
            .filter(|c| !c.is_static)
            .and_then(|c| c.getter);
        match getter {
            Some(body) => self
                .run_with_self(value.clone(), |me| me.eval(&body))
                .map(|(v, _)| Some(v)),
            None => Ok(None),
        }
    }

    // ----- Classes (reference semantics + ARC) -----

    /// The class inheritance chain, root superclass first.
    fn class_chain(&self, class_name: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut current = Some(class_name.to_string());
        while let Some(name) = current {
            if !self.classes.contains_key(&name) {
                break;
            }
            current = self.classes[&name].superclass.clone();
            chain.push(name);
        }
        chain.reverse();
        chain
    }

    /// Whether `sub` is `super` or a descendant of it.
    fn class_is(&self, sub: &str, super_: &str) -> bool {
        self.class_chain(sub).iter().any(|c| c == super_)
    }

    /// Find the most-derived method `name` for an object of `class_name`,
    /// returning the method and the class that declares it.
    fn lookup_method(
        &self,
        class_name: &str,
        name: &str,
    ) -> Option<(Vec<Param>, Option<Node<'static>>, String, Vec<String>)> {
        let mut current = Some(class_name.to_string());
        while let Some(cls) = current {
            let def = self.classes.get(&cls)?;
            if let Some(m) = def.methods.get(name) {
                return Some((
                    clone_params(&m.params),
                    m.body,
                    cls,
                    m.generic_params.clone(),
                ));
            }
            current = def.superclass.clone();
        }
        None
    }

    /// Select a class initializer overload by labels/types, falling back to the
    /// last declared initializer when no overload is uniquely selected.
    fn select_class_init(
        &self,
        class_name: &str,
        args: &[CallArg],
    ) -> Option<(Vec<Param>, Option<Node<'static>>)> {
        let def = self.classes.get(class_name)?;
        if def.init_overloads.len() > 1 {
            if let Some(init) = select_labeled_overload(&def.init_overloads, args) {
                return Some((clone_params(&init.params), init.body));
            }
        }
        def.init
            .as_ref()
            .map(|init| (clone_params(&init.params), init.body))
    }

    /// Instantiate a class: lay out fields from the whole chain, then run init.
    fn instantiate_class(&mut self, class_name: &str, args: Vec<CallArg>) -> Eval {
        let chain = self.class_chain(class_name);
        let mut fields: Vec<(String, SwiftValue)> = Vec::new();
        for cls in &chain {
            let props: Vec<(String, Option<Node<'static>>)> = self.classes[cls]
                .stored
                .iter()
                .map(|p| (p.name.clone(), p.default))
                .collect();
            for (pname, default) in props {
                let value = match default {
                    Some(def) => self.eval(&def)?,
                    None => SwiftValue::Nil,
                };
                fields.push((pname, value));
            }
        }
        let obj = StdRc::new(RefCell::new(ClassObj {
            class_name: class_name.to_string(),
            fields,
        }));
        let value = SwiftValue::Object(obj);

        // Run the most-derived initializer (walk up for an inherited one).
        let init_owner = chain
            .iter()
            .rev()
            .find(|c| self.classes[*c].init.is_some())
            .cloned();
        if let Some(owner) = init_owner {
            let (params, body) = self.select_class_init(&owner, &args).unwrap_or_else(|| {
                let m = self.classes[&owner].init.as_ref().unwrap();
                (clone_params(&m.params), m.body)
            });
            self.class_ctx.push(owner);
            let saved_env = self.env.enter_isolated();
            self.env.declare("self", value.clone(), false);
            let bound = self.bind_params(&params, args);
            let result = match bound {
                Ok(_) => match body {
                    Some(b) => self.eval(&b),
                    None => Ok(SwiftValue::Void),
                },
                Err(e) => Err(e),
            };
            self.env.restore(saved_env);
            self.class_ctx.pop();
            match result {
                // A failable initializer that runs `return nil` produces the
                // absent optional rather than the half-built instance.
                Err(Signal::Return(SwiftValue::Nil)) => return Ok(SwiftValue::Nil),
                Ok(_) | Err(Signal::Return(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(value)
    }

    /// Read a member off a class instance: a stored field (upgrading weak
    /// references), or a computed getter.
    fn read_object_member(&mut self, value: &SwiftValue, name: &str) -> Eval {
        let SwiftValue::Object(obj) = value else {
            return Err(EvalError::Type(format!("`{name}` is not a member")).into());
        };
        let class_name = obj.borrow().class_name.clone();
        if let Some(field) = obj.borrow().get(name).cloned() {
            return Ok(match field {
                SwiftValue::Weak(w) => w
                    .upgrade()
                    .map(SwiftValue::Object)
                    .unwrap_or(SwiftValue::Nil),
                v => v,
            });
        }
        // Computed getter somewhere in the chain.
        let getter = self.class_computed_getter(&class_name, name);
        if let Some(body) = getter {
            self.class_ctx.push(class_name);
            let r = self
                .run_with_self(value.clone(), |me| me.eval(&body))
                .map(|(v, _)| v);
            self.class_ctx.pop();
            return r;
        }
        Err(EvalError::Type(format!("{class_name} has no member `{name}`")).into())
    }

    /// Find a computed getter for `name` walking up the class chain.
    fn class_computed_getter(&self, class_name: &str, name: &str) -> Option<Node<'static>> {
        let mut current = Some(class_name.to_string());
        while let Some(cls) = current {
            let def = self.classes.get(&cls)?;
            if let Some(c) = def.computed.get(name).filter(|c| !c.is_static) {
                return c.getter;
            }
            current = def.superclass.clone();
        }
        None
    }

    /// Set a stored field on a class instance in place, downgrading values
    /// assigned to `weak` fields.
    fn set_object_field(&mut self, obj: &StdRc<RefCell<ClassObj>>, name: &str, value: SwiftValue) {
        let class_name = obj.borrow().class_name.clone();
        let is_weak = self.field_is_weak(&class_name, name);
        let stored = if is_weak {
            match value {
                SwiftValue::Object(o) => SwiftValue::Weak(StdRc::downgrade(&o)),
                other => other,
            }
        } else {
            value
        };
        obj.borrow_mut().set(name, stored);
    }

    /// Whether `name` is a `weak` field anywhere in the class chain.
    fn field_is_weak(&self, class_name: &str, name: &str) -> bool {
        let mut current = Some(class_name.to_string());
        while let Some(cls) = current {
            let Some(def) = self.classes.get(&cls) else {
                break;
            };
            if def.weak_fields.iter().any(|f| f == name) {
                return true;
            }
            current = def.superclass.clone();
        }
        false
    }

    /// Whether a class (or any ancestor) declares a stored/computed member.
    fn class_has_member(&self, class_name: &str, name: &str) -> bool {
        let mut current = Some(class_name.to_string());
        while let Some(cls) = current {
            let Some(def) = self.classes.get(&cls) else {
                break;
            };
            if def.stored.iter().any(|p| p.name == name) || def.computed.contains_key(name) {
                return true;
            }
            current = def.superclass.clone();
        }
        false
    }

    /// Run a deinit chain for an object whose last strong reference is dropping.
    fn run_deinit(&mut self, value: &SwiftValue) {
        let SwiftValue::Object(obj) = value else {
            return;
        };
        if StdRc::strong_count(obj) != 1 {
            return; // still referenced elsewhere
        }
        let class_name = obj.borrow().class_name.clone();
        // Run deinit bodies from the most-derived class up to the root.
        let mut chain = self.class_chain(&class_name);
        chain.reverse();
        for cls in chain {
            if let Some(body) = self.classes.get(&cls).and_then(|d| d.deinit) {
                self.class_ctx.push(cls);
                let _ = self.run_with_self(value.clone(), |me| me.eval(&body));
                self.class_ctx.pop();
            }
        }
    }

    // ----- Closures -----

    /// Build a closure value capturing the current scope chain.
    ///
    /// msf nests an untyped-parameter closure's body under its last `Param`
    /// node, while a typed closure keeps the body as sibling statements; this
    /// handles both.
    fn eval_closure(&mut self, node: &Node<'static>) -> Eval {
        let mut params = Vec::new();
        let mut body = Vec::new();
        let mut last_param: Option<Node<'static>> = None;
        let mut captured_overrides: Vec<(String, SwiftValue)> = Vec::new();
        for child in node.children() {
            match child.kind() {
                NodeKind::Param => {
                    // Closure params have no external label; the name is the
                    // first token (param_info's two-name heuristic mistakes the
                    // trailing `in` keyword for an internal name).
                    let name = child.text().unwrap_or_default();
                    let info = child.param_info();
                    params.push(Param {
                        label: None,
                        name,
                        ty: None,
                        variadic: info.variadic,
                        inout_: info.is_inout,
                        autoclosure: false,
                        default: None,
                    });
                    last_param = Some(child);
                }
                NodeKind::ClosureCapture => {
                    if let Some(name) = child.text() {
                        let v = match child.children().next() {
                            Some(expr) => self.eval(&expr)?,
                            None => self.env.get(&name).unwrap_or(SwiftValue::Nil),
                        };
                        captured_overrides.push((name, v));
                    }
                }
                NodeKind::TypeRef => {}
                _ => body.push(child),
            }
        }
        // Untyped parameters: the body lives under the last `Param` node.
        if body.is_empty() {
            if let Some(p) = last_param {
                for c in p.children() {
                    if c.kind() != NodeKind::TypeRef {
                        body.push(c);
                    }
                }
            }
        }

        // A closure body is collected here rather than executed through
        // `eval_block`, so expand any `#if` wrappers in it now.
        let body = expand_directive_list(body);

        let mut captured = self.env.capture();
        if !captured_overrides.is_empty() {
            let scope: Scope = Default::default();
            for (name, v) in captured_overrides {
                scope.borrow_mut().insert(
                    name,
                    crate::env::Binding {
                        value: v,
                        mutable: false,
                    },
                );
            }
            captured.push(scope);
        }
        let id = self.closures.len();
        self.closures
            .push((ClosureDef::User { params, body }, captured));
        Ok(SwiftValue::Closure(id))
    }

    /// Invoke a closure value with call arguments, writing back any `inout`
    /// parameters to their caller locations (`f(&x)` over a closure whose
    /// parameter is `inout`). Falls back to the value-only path when the
    /// closure has no `inout` parameters.
    fn call_closure_with_args(&mut self, id: usize, args: Vec<CallArg>) -> Eval {
        // A closure participates in the `inout` write-back path when it either
        // declares an explicit `inout` parameter or is being called with an
        // `&`-prefixed argument. The latter covers shorthand closures
        // (`{ $0 += 1 }`), whose parameters carry no explicit `inout` marker —
        // the caller's `&x` is the contextual signal that position is `inout`.
        let has_inout = match self.closures.get(id) {
            Some((ClosureDef::User { params, .. }, _)) => {
                params.iter().any(|p| p.inout_) || args.iter().any(|a| a.place.is_some())
            }
            _ => false,
        };
        if !has_inout {
            let plain = args.into_iter().map(|a| a.value).collect();
            return self.call_closure(id, plain);
        }

        self.depth += 1;
        if self.depth > MAX_CALL_DEPTH {
            self.depth -= 1;
            return Err(trap("stack overflow: recursion too deep".into()));
        }
        let (params, body, captured) = match &self.closures[id] {
            (ClosureDef::User { params, body }, cap) => {
                (clone_params(params), body.clone(), cap.clone())
            }
            _ => unreachable!("operator/non-user closure has no inout params"),
        };
        let call_env = Env::with_captured(captured);
        let saved = std::mem::replace(&mut self.env, call_env);

        let mut writebacks: Vec<(String, Place)> = Vec::new();
        for (i, p) in params.iter().enumerate() {
            // Swift rejects passing a value to an `inout` parameter without an
            // explicit `&`; an `inout` parameter therefore requires its caller
            // argument to carry a write-back `Place`.
            if p.inout_ && args.get(i).and_then(|a| a.place.as_ref()).is_none() {
                self.env = saved;
                self.depth -= 1;
                return Err(trap(format!(
                    "passing value to 'inout' parameter '{}' requires '&'",
                    p.name
                )));
            }
            let v = args
                .get(i)
                .map(|a| a.value.clone())
                .unwrap_or(SwiftValue::Nil);
            let place = args.get(i).and_then(|a| a.place.clone());
            self.env.declare(&p.name, v, p.inout_ || place.is_some());
            if let Some(place) = place {
                writebacks.push((p.name.clone(), place));
            }
        }
        for (i, a) in args.iter().enumerate() {
            // Shorthand closures (`{ $0 += 1 }`) expose no named `Param`; an
            // `&`-passed argument makes the `$i` binding the mutable `inout`
            // target and schedules its final value for write-back.
            let is_shorthand_inout = i >= params.len() && a.place.is_some();
            self.env
                .declare(&format!("${i}"), a.value.clone(), is_shorthand_inout);
            if is_shorthand_inout {
                if let Some(place) = a.place.clone() {
                    writebacks.push((format!("${i}"), place));
                }
            }
        }

        let mut result = Ok(SwiftValue::Void);
        for stmt in &body {
            match self.eval(stmt) {
                Ok(v) => result = Ok(v),
                Err(Signal::Return(v)) => {
                    result = Ok(v);
                    break;
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        // Capture the final value of each `inout` parameter before unwinding.
        let finals: Vec<(Place, SwiftValue)> = writebacks
            .iter()
            .filter_map(|(name, place)| self.env.get(name).map(|v| (place.clone(), v)))
            .collect();
        self.env = saved;
        self.depth -= 1;
        // Swift copies `inout` arguments back even when the callee throws, so
        // mutations are visible after a caught/`try?`-converted error. Only a
        // fatal interpreter trap (`Signal::Error`) skips the copy-out.
        if !matches!(result, Err(Signal::Error(_))) {
            for (place, v) in finals {
                self.write_place(&place, v)?;
            }
        }
        result
    }

    /// Evaluate the body of user closure `id` statement-by-statement, returning
    /// each statement's value (Void results dropped) — the result-builder block
    /// evaluation backing `@ViewBuilder`. Non-user closures yield their single
    /// call result.
    fn eval_builder_block(&mut self, id: usize) -> Result<Vec<SwiftValue>, Signal> {
        if id >= self.closures.len() {
            return Err(EvalError::UnknownFunction("<closure>".into()).into());
        }
        let (body, captured) = match &self.closures[id] {
            (ClosureDef::User { body, .. }, cap) => (body.clone(), cap.clone()),
            _ => {
                // Operator/key-path closures carry no multi-statement body.
                let v = self.call_closure(id, Vec::new())?;
                return Ok(if matches!(v, SwiftValue::Void) {
                    Vec::new()
                } else {
                    vec![v]
                });
            }
        };
        self.depth += 1;
        if self.depth > MAX_CALL_DEPTH {
            self.depth -= 1;
            return Err(trap("stack overflow: recursion too deep".into()));
        }
        let call_env = Env::with_captured(captured);
        let saved = std::mem::replace(&mut self.env, call_env);
        let mut values = Vec::new();
        let mut error = None;
        for stmt in &body {
            match self.eval(stmt) {
                Ok(SwiftValue::Void) => {}
                Ok(v) => values.push(v),
                Err(Signal::Return(v)) => {
                    if !matches!(v, SwiftValue::Void) {
                        values.push(v);
                    }
                    break;
                }
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
        }
        self.env = saved;
        self.depth -= 1;
        match error {
            Some(e) => Err(e),
            None => Ok(values),
        }
    }

    /// Evaluate a `@ViewBuilder`-style closure body with bound arguments,
    /// collecting *every* view-valued statement (not just the last). This is the
    /// builder-block analogue of [`Interpreter::call_closure`]: `ForEach`'s
    /// per-element content closure takes the element as an argument yet may emit
    /// several sibling views.
    fn eval_builder_block_with_args(
        &mut self,
        id: usize,
        args: Vec<SwiftValue>,
    ) -> Result<Vec<SwiftValue>, Signal> {
        let (params, body, captured) = match &self.closures[id] {
            (ClosureDef::User { params, body }, cap) => {
                (clone_params(params), body.clone(), cap.clone())
            }
            // Operator/key-path closures carry no multi-statement body; fall
            // back to a single applied value.
            _ => {
                let v = self.call_closure(id, args)?;
                return Ok(if matches!(v, SwiftValue::Void) {
                    Vec::new()
                } else {
                    vec![v]
                });
            }
        };
        self.depth += 1;
        if self.depth > MAX_CALL_DEPTH {
            self.depth -= 1;
            return Err(trap("stack overflow: recursion too deep".into()));
        }
        let call_env = Env::with_captured(captured);
        let saved = std::mem::replace(&mut self.env, call_env);
        for (i, p) in params.iter().enumerate() {
            let v = args.get(i).cloned().unwrap_or(SwiftValue::Nil);
            self.env.declare(&p.name, v, false);
        }
        for (i, v) in args.iter().enumerate() {
            self.env.declare(&format!("${i}"), v.clone(), false);
        }
        let mut values = Vec::new();
        let mut error = None;
        for stmt in &body {
            match self.eval(stmt) {
                Ok(SwiftValue::Void) => {}
                Ok(v) => values.push(v),
                Err(Signal::Return(v)) => {
                    if !matches!(v, SwiftValue::Void) {
                        values.push(v);
                    }
                    break;
                }
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
        }
        self.env = saved;
        self.depth -= 1;
        match error {
            Some(e) => Err(e),
            None => Ok(values),
        }
    }

    /// Invoke a closure value with already-evaluated arguments.
    fn call_closure(&mut self, id: usize, args: Vec<SwiftValue>) -> Eval {
        if id >= self.closures.len() {
            return Err(EvalError::UnknownFunction("<closure>".into()).into());
        }
        self.depth += 1;
        if self.depth > MAX_CALL_DEPTH {
            self.depth -= 1;
            return Err(trap("stack overflow: recursion too deep".into()));
        }
        // An operator-function reference applies its operator directly to the
        // arguments (binary for two, unary for one) without a call frame.
        if let (ClosureDef::Operator(op), _) = &self.closures[id] {
            let op = op.clone();
            self.depth -= 1;
            return match args.as_slice() {
                [a, b] => ops::binary(&op, a, b).map_err(trap),
                [a] => ops::unary(&op, a).map_err(trap),
                _ => Err(EvalError::Unsupported(format!(
                    "operator `{op}` reference expects 1 or 2 arguments"
                ))
                .into()),
            };
        }
        // A key-path value used as a function: walk the path from its single
        // argument (`names.map(\.count)`).
        if let (ClosureDef::KeyPath(components), _) = &self.closures[id] {
            let components = components.clone();
            self.depth -= 1;
            let [root] = <[SwiftValue; 1]>::try_from(args).map_err(|args| {
                EvalError::Unsupported(format!(
                    "key-path function expects exactly one argument, got {}",
                    args.len()
                ))
            })?;
            return self.apply_keypath(root, &components);
        }
        let (params, body, captured) = {
            let (def, cap) = &self.closures[id];
            match def {
                ClosureDef::User { params, body } => {
                    (clone_params(params), body.clone(), cap.clone())
                }
                ClosureDef::Operator(_) | ClosureDef::KeyPath(_) => {
                    unreachable!("operator/key-path handled above")
                }
            }
        };
        let call_env = Env::with_captured(captured);
        let saved = std::mem::replace(&mut self.env, call_env);

        // Bind named parameters, and always expose `$0`, `$1`, … shorthands.
        for (i, p) in params.iter().enumerate() {
            let v = args.get(i).cloned().unwrap_or(SwiftValue::Nil);
            self.env.declare(&p.name, v, false);
        }
        for (i, v) in args.iter().enumerate() {
            self.env.declare(&format!("${i}"), v.clone(), false);
        }

        // Evaluate the closure body statements, yielding the last value.
        let mut result = Ok(SwiftValue::Void);
        for stmt in &body {
            match self.eval(stmt) {
                Ok(v) => result = Ok(v),
                Err(Signal::Return(v)) => {
                    result = Ok(v);
                    break;
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        self.env = saved;
        self.depth -= 1;
        result
    }

    // ----- Structured concurrency (ADR-0005) -----

    /// `await <expr>`: evaluate the operand, then, if it is a task handle, drive
    /// that task to completion and yield its result. Awaiting any other value is
    /// the identity (an `await f()` on an inline `async` call already ran).
    fn eval_await(&mut self, node: &Node<'static>) -> Eval {
        let inner = node
            .children()
            .next()
            .ok_or_else(|| EvalError::Unsupported("await without an operand".into()))?;
        let value = self.eval(&inner)?;
        self.await_value(value)
    }

    /// Resolve a value an `await` produced: drive a task handle, pass anything
    /// else through unchanged.
    fn await_value(&mut self, value: SwiftValue) -> Eval {
        match value {
            SwiftValue::Task(id) => self.run_task(id),
            other => Ok(other),
        }
    }

    /// Register a 0-argument closure body as a task and return its handle index.
    fn spawn_task_closure(&mut self, closure_id: usize) -> usize {
        let id = self.tasks.len();
        self.tasks.push(TaskSlot {
            closure: closure_id,
            class_ctx: self.class_ctx.clone(),
            state: TaskState::Pending,
            cancelled: false,
        });
        id
    }

    /// Spawn a task whose body is a single expression (used by `async let`),
    /// capturing the current lexical scope so the expression sees local state.
    fn spawn_expr_task(&mut self, expr: Node<'static>) -> usize {
        let captured = self.env.capture();
        let closure_id = self.closures.len();
        self.closures.push((
            ClosureDef::User {
                params: Vec::new(),
                body: vec![expr],
            },
            captured,
        ));
        self.spawn_task_closure(closure_id)
    }

    /// Drive task `id` to completion (cooperatively, on the current stack) and
    /// return its memoized outcome. Re-awaiting a finished task returns the
    /// stored result; awaiting a task that is mid-flight is a deadlock trap.
    fn run_task(&mut self, id: usize) -> Eval {
        match &self.tasks[id].state {
            TaskState::Done(result) => return result.clone(),
            TaskState::Running => {
                return Err(trap("await on a task awaiting itself (deadlock)".into()));
            }
            TaskState::Pending => {}
        }
        let closure = self.tasks[id].closure;
        let ctx = self.tasks[id].class_ctx.clone();
        self.tasks[id].state = TaskState::Running;
        let saved_ctx = std::mem::replace(&mut self.class_ctx, ctx);
        let result = self.call_closure(closure, Vec::new());
        self.class_ctx = saved_ctx;
        self.tasks[id].state = TaskState::Done(result.clone());
        result
    }

    /// Run every spawned-but-unawaited task to completion. Called at the end of
    /// the program so detached `Task { }` side effects still happen (structured
    /// concurrency guarantees a child finishes before its scope exits; here the
    /// whole program is the outermost scope).
    fn drain_pending_tasks(&mut self) -> Result<(), Signal> {
        let mut i = 0;
        while i < self.tasks.len() {
            if matches!(self.tasks[i].state, TaskState::Pending) {
                if let Err(sig @ Signal::Error(_)) = self.run_task(i) {
                    return Err(sig);
                }
            }
            i += 1;
        }
        Ok(())
    }

    /// The first closure handle among already-evaluated call arguments (the
    /// trailing `{ ... }` of `group.addTask { }`).
    fn first_closure(args: &[CallArg]) -> Option<usize> {
        args.iter().find_map(|a| match a.value {
            SwiftValue::Closure(id) => Some(id),
            _ => None,
        })
    }

    /// Evaluate the trailing body closure of a concurrency call to a closure
    /// handle, ignoring non-closure arguments (e.g. `of: Int.self`).
    fn eval_body_closure(&mut self, arg_nodes: &[Node<'static>]) -> Result<Option<usize>, Signal> {
        for arg in arg_nodes {
            if arg.kind() == NodeKind::ClosureExpr {
                if let SwiftValue::Closure(id) = self.eval(arg)? {
                    return Ok(Some(id));
                }
            }
        }
        Ok(None)
    }

    /// Dispatch the free-function concurrency entry points (`Task { }`,
    /// `withTaskGroup { }`). Returns `None` if `name` is not one of them so
    /// normal call resolution continues.
    fn try_concurrency_builtin(
        &mut self,
        name: &str,
        arg_nodes: &[Node<'static>],
    ) -> Result<Option<SwiftValue>, Signal> {
        match name {
            "Task" => {
                let closure = self
                    .eval_body_closure(arg_nodes)?
                    .ok_or_else(|| EvalError::Unsupported("Task without a body closure".into()))?;
                Ok(Some(SwiftValue::Task(self.spawn_task_closure(closure))))
            }
            "withTaskGroup" | "withThrowingTaskGroup" => {
                let body = self.eval_body_closure(arg_nodes)?.ok_or_else(|| {
                    EvalError::Unsupported("withTaskGroup without a body closure".into())
                })?;
                let gid = self.groups.len();
                self.groups.push(Vec::new());
                let result = self.call_closure(body, vec![SwiftValue::TaskGroup(gid)]);
                // The group's children are structured: drain any not consumed by
                // a `for await` so they complete before the group returns.
                self.drain_group(gid)?;
                result.map(Some)
            }
            "withCheckedContinuation"
            | "withUnsafeContinuation"
            | "withCheckedThrowingContinuation"
            | "withUnsafeThrowingContinuation" => {
                self.eval_with_continuation(name, arg_nodes).map(Some)
            }
            _ => Ok(None),
        }
    }

    /// `await with*Continuation { continuation in ... }`: hand the body a
    /// continuation handle, run it, then read back whatever `resume(...)` stored.
    ///
    /// Our executor runs to completion at each `await` (ADR-0005), so the body
    /// either resumes the continuation inline or hands it to a spawned `Task`.
    /// If it is not resumed inline we drive only the tasks *this body spawned*
    /// (not unrelated earlier pending tasks) until the continuation is resumed.
    /// An unresumed continuation traps, mirroring `CheckedContinuation`'s misuse
    /// diagnostic.
    fn eval_with_continuation(&mut self, name: &str, arg_nodes: &[Node<'static>]) -> Eval {
        let body = self
            .eval_body_closure(arg_nodes)?
            .ok_or_else(|| EvalError::Unsupported(format!("{name} without a body closure")))?;
        let cid = self.continuations.len();
        self.continuations.push(ContinuationState::Pending);
        let body_tasks_start = self.tasks.len();
        self.call_closure(body, vec![SwiftValue::Continuation(cid)])?;
        if matches!(self.continuations[cid], ContinuationState::Pending) {
            // The body parked the continuation in a task it spawned; drive only
            // those tasks (in spawn order) until one resumes the continuation.
            self.drive_tasks_until_resumed(cid, body_tasks_start)?;
        }
        // Read the value and mark the slot consumed so a later resume traps.
        match std::mem::replace(&mut self.continuations[cid], ContinuationState::Consumed) {
            ContinuationState::Resumed(result) => result,
            _ => Err(trap(
                "continuation was not resumed before with*Continuation returned".into(),
            )),
        }
    }

    /// Drive the tasks spawned at or after `start` (in spawn order) until the
    /// continuation `cid` is resumed, leaving any unrelated/earlier pending
    /// tasks for normal program-end draining. A spawned task that fails with a
    /// genuine interpreter error propagates; an uncaught Swift `throw` from a
    /// detached task is dropped, matching [`drain_pending_tasks`].
    fn drive_tasks_until_resumed(&mut self, cid: usize, start: usize) -> Result<(), Signal> {
        let mut i = start;
        while i < self.tasks.len() {
            if matches!(self.continuations[cid], ContinuationState::Resumed(_)) {
                break;
            }
            if matches!(self.tasks[i].state, TaskState::Pending) {
                if let Err(sig @ Signal::Error(_)) = self.run_task(i) {
                    return Err(sig);
                }
            }
            i += 1;
        }
        Ok(())
    }

    /// Dispatch instance methods on a task handle or task group. Returns `None`
    /// when `base` is neither, so normal method resolution continues.
    fn try_concurrency_method(
        &mut self,
        base: &SwiftValue,
        method: &str,
        arg_nodes: &[Node<'static>],
    ) -> Result<Option<SwiftValue>, Signal> {
        match base {
            SwiftValue::TaskGroup(gid) => {
                let gid = *gid;
                match method {
                    "addTask" | "addTaskUnlessCancelled" => {
                        let args = self.eval_args(arg_nodes)?;
                        let closure = Self::first_closure(&args).ok_or_else(|| {
                            EvalError::Unsupported("addTask without a body closure".into())
                        })?;
                        let tid = self.spawn_task_closure(closure);
                        self.groups[gid].push(tid);
                        Ok(Some(SwiftValue::Bool(true)))
                    }
                    "cancelAll" => {
                        for &tid in &self.groups[gid].clone() {
                            self.tasks[tid].cancelled = true;
                        }
                        Ok(Some(SwiftValue::Void))
                    }
                    "waitForAll" => {
                        self.drain_group(gid)?;
                        Ok(Some(SwiftValue::Void))
                    }
                    _ => Ok(None),
                }
            }
            SwiftValue::Task(tid) => {
                let tid = *tid;
                match method {
                    "cancel" => {
                        self.tasks[tid].cancelled = true;
                        Ok(Some(SwiftValue::Void))
                    }
                    _ => Ok(None),
                }
            }
            SwiftValue::Continuation(cid) => {
                let cid = *cid;
                if method != "resume" {
                    return Ok(None);
                }
                let args = self.eval_args(arg_nodes)?;
                let outcome = self.continuation_outcome(&args)?;
                // `CheckedContinuation` traps on a second *or late* resume: only
                // a still-`Pending` slot accepts the outcome.
                if !matches!(self.continuations[cid], ContinuationState::Pending) {
                    return Err(trap("continuation resumed more than once".into()));
                }
                self.continuations[cid] = ContinuationState::Resumed(outcome);
                Ok(Some(SwiftValue::Void))
            }
            _ => Ok(None),
        }
    }

    /// Decode a continuation `resume(...)` call's arguments into the outcome to
    /// store: `resume()` / `resume(returning:)` yield a value; `resume(throwing:)`
    /// a thrown error; `resume(with: .success/.failure)` either, per the `Result`.
    fn continuation_outcome(&self, args: &[CallArg]) -> Result<Eval, Signal> {
        match args.first() {
            // `resume()` — Void continuation.
            None => Ok(Ok(SwiftValue::Void)),
            Some(arg) => match arg.label.as_deref() {
                Some("throwing") => Ok(Err(Signal::Throw(arg.value.clone()))),
                Some("with") => match &arg.value {
                    SwiftValue::Enum(e) if e.case == "success" => {
                        Ok(Ok(e.payload.first().cloned().unwrap_or(SwiftValue::Void)))
                    }
                    SwiftValue::Enum(e) if e.case == "failure" => Ok(Err(Signal::Throw(
                        e.payload.first().cloned().unwrap_or(SwiftValue::Void),
                    ))),
                    other => Err(trap(format!(
                        "resume(with:) expects a Result, got {}",
                        other.type_name()
                    ))),
                },
                // `resume(returning:)` or an unlabeled value.
                _ => Ok(Ok(arg.value.clone())),
            },
        }
    }

    /// Run any still-pending child tasks of group `gid` (structured-concurrency
    /// guarantee: the group does not return until its children finish).
    fn drain_group(&mut self, gid: usize) -> Result<(), Signal> {
        let ids = std::mem::take(&mut self.groups[gid]);
        for id in ids {
            if let Err(sig @ Signal::Error(_)) = self.run_task(id) {
                return Err(sig);
            }
        }
        Ok(())
    }

    /// Consume a group's children for `for await`, returning their results in
    /// completion order (our cooperative executor runs them in add order).
    fn drain_group_results(&mut self, gid: usize) -> Result<Vec<SwiftValue>, Signal> {
        let ids = std::mem::take(&mut self.groups[gid]);
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            out.push(self.run_task(id)?);
        }
        Ok(out)
    }

    // ----- Casting -----

    /// `expr is T`, `expr as? T`, `expr as! T`, `expr as T`.
    fn eval_cast(&mut self, node: &Node<'static>) -> Eval {
        let op = node.op_text().unwrap_or_default();
        let mut kids = node.children();
        let expr = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("cast without an expression".into()))?;
        let ty = kids.next().and_then(|t| t.text()).unwrap_or_default();
        let value = self.eval(&expr)?;
        let matches = self.value_is_type(&value, &ty);
        // `as?` carries the optional-cast modifier (0x800); `is` yields Bool.
        let optional = node.modifiers() & 0x800 != 0;
        if op == "is" {
            return Ok(SwiftValue::Bool(matches));
        }
        if optional {
            return Ok(if matches { value } else { SwiftValue::Nil });
        }
        // `as!` / `as`
        if matches {
            Ok(value)
        } else {
            Err(trap(format!("could not cast value to {ty}")))
        }
    }

    /// Whether `value`'s dynamic type is (or descends from) `type_name`.
    fn value_is_type(&self, value: &SwiftValue, type_name: &str) -> bool {
        match value {
            SwiftValue::Object(o) => {
                let cls = o.borrow().class_name.clone();
                self.class_is(&cls, type_name)
            }
            SwiftValue::Int(_) => {
                matches!(type_name, "Int" | "Int64")
                    || IntWidth::from_type_name(type_name).is_some()
            }
            SwiftValue::Double(_) => type_name == "Double" || type_name == "Float",
            SwiftValue::Bool(_) => type_name == "Bool",
            SwiftValue::Str(_) => type_name == "String",
            SwiftValue::Struct(s) => s.type_name == type_name,
            SwiftValue::Enum(e) => e.type_name == type_name,
            // Array cast `[Element]`: every element must match the element
            // type. A covariant optional element (`[T]` as `[T?]`) succeeds
            // because each `T` is a valid `T?` and `nil` qualifies too.
            SwiftValue::Array(items) => match array_element_type(type_name) {
                Some(elem) => {
                    let (base, optional) = match elem.strip_suffix('?') {
                        Some(b) => (b.trim(), true),
                        None => (elem, false),
                    };
                    items.iter().all(|v| {
                        (optional && matches!(v, SwiftValue::Nil)) || self.value_is_type(v, base)
                    })
                }
                None => false,
            },
            // A `nil` is any optional type (`T?`, `[T]?`, …).
            SwiftValue::Nil => type_name.ends_with('?'),
            _ => false,
        }
    }

    /// The member chain of a `#selector`/`#keyPath` operand, dropping the
    /// leading type root: `C.a.b` → `["a", "b"]`. The names are collected
    /// outer-to-inner then reversed into source order.
    fn member_chain(node: &Node<'static>) -> Vec<String> {
        let mut names = Vec::new();
        let mut cur = Some(*node);
        while let Some(n) = cur {
            if n.kind() == NodeKind::MemberExpr {
                if let Some(name) = n.text() {
                    names.push(name);
                }
                cur = n.children().next();
            } else {
                break;
            }
        }
        names.reverse();
        names
    }

    /// Magic literals: `#file`, `#line`, `#function`, `#column`.
    fn eval_macro(&mut self, node: &Node<'static>) -> Eval {
        let which = node.text().unwrap_or_default();
        match which.as_str() {
            "file" | "filePath" | "fileID" => Ok(SwiftValue::Str(self.filename.clone())),
            "line" => Ok(SwiftValue::int(node.line() as i128)),
            "column" => Ok(SwiftValue::int(0)),
            // `#selector(Type.method)` yields the method name (Swift prints a
            // selector as its name); `#keyPath(Type.a.b)` yields the dotted key
            // path string relative to the root type.
            "selector" => {
                let chain = node.children().next().map(|c| Self::member_chain(&c));
                Ok(SwiftValue::Str(
                    chain.and_then(|c| c.last().cloned()).unwrap_or_default(),
                ))
            }
            "keyPath" => {
                let chain = node
                    .children()
                    .next()
                    .map(|c| Self::member_chain(&c))
                    .unwrap_or_default();
                Ok(SwiftValue::Str(chain.join(".")))
            }
            "function" => Ok(SwiftValue::Str(
                self.class_ctx.last().cloned().unwrap_or_default(),
            )),
            // Availability conditions. The runtime targets one current platform,
            // so a required version is always met: `#available(...)` is `true`
            // and `#unavailable(...)` is `false`.
            "available" => Ok(SwiftValue::Bool(true)),
            "unavailable" => Ok(SwiftValue::Bool(false)),
            // `#warning`/`#error` are diagnosed by the frontend; no-op at runtime.
            _ => Ok(SwiftValue::Void),
        }
    }

    /// Serialize a `Codable` value to its `JSONEncoder` representation.
    fn json_encode(&self, value: &SwiftValue) -> Result<crate::json::Json, Signal> {
        use crate::json::Json;
        Ok(match value {
            SwiftValue::Nil => Json::Null,
            SwiftValue::Bool(b) => Json::Bool(*b),
            SwiftValue::Int(i) => Json::Int(i.raw as i64),
            SwiftValue::Double(d) => Json::Double(*d),
            SwiftValue::Str(s) => Json::Str(s.clone()),
            SwiftValue::Array(items) => Json::Array(
                items
                    .iter()
                    .map(|v| self.json_encode(v))
                    .collect::<Result<_, _>>()?,
            ),
            SwiftValue::Struct(o) => Json::Object(
                o.fields
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.json_encode(v)?)))
                    .collect::<Result<_, Signal>>()?,
            ),
            // A `Codable` enum: a `RawRepresentable` enum encodes its raw value;
            // a payload-free enum encodes its bare case name.
            SwiftValue::Enum(e) => {
                let raw = self
                    .enums
                    .get(&e.type_name)
                    .and_then(|d| d.cases.iter().find(|c| c.name == e.case))
                    .and_then(|c| c.raw.clone());
                match raw {
                    Some(r) => self.json_encode(&r)?,
                    None if e.payload.is_empty() => Json::Str(e.case.clone()),
                    None => {
                        return Err(EvalError::Type(format!(
                            "cannot encode enum '{}' with associated values",
                            e.type_name
                        ))
                        .into())
                    }
                }
            }
            other => {
                return Err(EvalError::Type(format!("cannot encode {}", other.type_name())).into())
            }
        })
    }

    /// Build a runtime value from JSON for the given target type (a registered
    /// struct, else inferred from the JSON shape).
    fn json_decode(&self, type_name: &str, json: &crate::json::Json) -> SwiftValue {
        use crate::json::Json;
        // A `Codable` enum decodes from its raw value, or — for a payload-free
        // case — its bare case name. A case with associated values never matches
        // here (we would have to synthesize a payload), so it is skipped.
        if let Some(def) = self.enums.get(type_name) {
            let decoded = self.json_value(json);
            if let Some(case) = def.cases.iter().find(|c| {
                let raw_matches = c.raw.as_ref().is_some_and(|r| r == &decoded);
                let name_matches = c.payload_types.is_empty()
                    && matches!(&decoded, SwiftValue::Str(s) if s == &c.name);
                raw_matches || name_matches
            }) {
                return SwiftValue::Enum(Rc::new(EnumObj {
                    type_name: type_name.to_string(),
                    case: case.name.clone(),
                    payload: Vec::new(),
                }));
            }
        }
        if let (Json::Object(_), Some(def)) = (json, self.structs.get(type_name)) {
            let fields: Vec<(String, SwiftValue)> = def
                .stored
                .iter()
                .map(|p| {
                    let v = json
                        .get(&p.name)
                        .map(|j| {
                            // Decode typed nested fields (structs/enums) by their
                            // declared element type so they round-trip; fall back
                            // to a shape-inferred value otherwise.
                            match p.ty.as_deref() {
                                Some(full)
                                    if self.structs.contains_key(decode_element_type(full))
                                        || self.enums.contains_key(decode_element_type(full)) =>
                                {
                                    self.json_decode_field(decode_element_type(full), full, j)
                                }
                                _ => self.json_value(j),
                            }
                        })
                        .unwrap_or(SwiftValue::Nil);
                    (p.name.clone(), v)
                })
                .collect();
            return SwiftValue::Struct(Rc::new(StructObj {
                type_name: type_name.to_string(),
                fields,
            }));
        }
        self.json_value(json)
    }

    /// Decode a struct field whose declared type is `inner` (the element type)
    /// and full spelling `full` (e.g. `[User]`, `User?`). Handles arrays and
    /// optionals of a registered struct/enum element.
    fn json_decode_field(&self, inner: &str, full: &str, json: &crate::json::Json) -> SwiftValue {
        use crate::json::Json;
        match json {
            // `nil` for an absent optional.
            Json::Null => SwiftValue::Nil,
            // `[Element]` decodes each item by the element type.
            Json::Array(items) if full.trim_start().starts_with('[') => SwiftValue::Array(Rc::new(
                items.iter().map(|j| self.json_decode(inner, j)).collect(),
            )),
            _ => self.json_decode(inner, json),
        }
    }

    /// Map a JSON value to a runtime value without target-type context.
    fn json_value(&self, json: &crate::json::Json) -> SwiftValue {
        use crate::json::Json;
        match json {
            Json::Null => SwiftValue::Nil,
            Json::Bool(b) => SwiftValue::Bool(*b),
            Json::Int(i) => SwiftValue::int(*i as i128),
            Json::Double(d) => SwiftValue::Double(*d),
            Json::Str(s) => SwiftValue::Str(s.clone()),
            Json::Array(items) => {
                SwiftValue::Array(Rc::new(items.iter().map(|j| self.json_value(j)).collect()))
            }
            Json::Object(entries) => SwiftValue::Struct(Rc::new(StructObj {
                type_name: "JSON".into(),
                fields: entries
                    .iter()
                    .map(|(k, v)| (k.clone(), self.json_value(v)))
                    .collect(),
            })),
        }
    }

    /// `value!` — force-unwrap an optional, trapping on nil.
    fn eval_force_unwrap(&mut self, node: &Node<'static>) -> Eval {
        let inner = node
            .children()
            .next()
            .ok_or_else(|| EvalError::Unsupported("force-unwrap without operand".into()))?;
        let v = self.eval(&inner)?;
        if matches!(v, SwiftValue::Nil) {
            Err(trap(
                "unexpectedly found nil while unwrapping an Optional".into(),
            ))
        } else {
            Ok(v)
        }
    }

    /// An array literal `[a, b, …]`.
    fn eval_array_literal(&mut self, node: &Node<'static>) -> Eval {
        let mut items = Vec::new();
        for child in node.children() {
            items.push(self.eval(&child)?);
        }
        Ok(SwiftValue::Array(Rc::new(items)))
    }

    /// A dictionary literal `[k: v, …]` — children alternate key, value. An
    /// empty dictionary is written `[:]`.
    fn eval_dict_literal(&mut self, node: &Node<'static>) -> Eval {
        let children: Vec<Node<'static>> = node.children().collect();
        let mut pairs: Vec<(SwiftValue, SwiftValue)> = Vec::new();
        let mut i = 0;
        while i + 1 < children.len() {
            let key = self.eval(&children[i])?;
            let value = self.eval(&children[i + 1])?;
            // Later duplicate keys overwrite earlier ones.
            if let Some(slot) = pairs.iter_mut().find(|(k, _)| *k == key) {
                slot.1 = value;
            } else {
                pairs.push((key, value));
            }
            i += 2;
        }
        Ok(SwiftValue::Dict(Rc::new(pairs)))
    }

    /// A subscript read `base[index]` over arrays, strings, or a user
    /// `subscript` getter.
    fn eval_subscript(&mut self, node: &Node<'static>) -> Eval {
        let mut kids = node.children();
        let base = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("subscript without a base".into()))?;
        // `Type[index]`: a `static subscript` addressed through the type name.
        if base.kind() == NodeKind::IdentExpr {
            if let Some(type_name) = base.text() {
                let has_static = self
                    .structs
                    .get(&type_name)
                    .is_some_and(|d| d.static_subscript.is_some())
                    || self
                        .classes
                        .get(&type_name)
                        .is_some_and(|d| d.static_subscript.is_some());
                if self.env.get(&type_name).is_none() && has_static {
                    let indices: Vec<SwiftValue> =
                        kids.map(|n| self.eval(&n)).collect::<Result<_, _>>()?;
                    return self.read_static_subscript(&type_name, &indices);
                }
            }
        }
        let base_value = self.eval(&base)?;
        let index_nodes: Vec<Node<'static>> = kids.collect();
        // A single one-sided range index (`a[2...]`, `a[..<2]`, `a[...2]`) is
        // resolved against the base collection's length into a concrete
        // `Range` before the generic index evaluation, which has no notion of
        // partial ranges.
        if let [only] = index_nodes.as_slice() {
            if let Some(range) = self.eval_partial_range_index(only, &base_value)? {
                return self.read_subscript(&base_value, &[range]);
            }
        }
        let indices: Vec<SwiftValue> = index_nodes
            .iter()
            .map(|n| self.eval(n))
            .collect::<Result<_, _>>()?;
        self.read_subscript(&base_value, &indices)
    }

    /// If `node` is a one-sided range form (`..<n` / `...n` prefix or `n...`
    /// postfix), resolve it to a concrete `Range` over `base`'s length; else
    /// `None`. The lower bound of an up-to/through range is `0`; the upper
    /// bound of a from range is the collection's element count.
    fn eval_partial_range_index(
        &mut self,
        node: &Node<'static>,
        base: &SwiftValue,
    ) -> Result<Option<SwiftValue>, Signal> {
        let len = match base {
            SwiftValue::Array(items) => items.len() as i128,
            SwiftValue::Str(s) => crate::graphemes(s).len() as i128,
            _ => return Ok(None),
        };
        let op = node.op_text();
        let bound_int = |this: &mut Self, n: &Node<'static>| -> Result<i128, Signal> {
            match this.eval(n)? {
                SwiftValue::Int(i) => Ok(i.raw),
                other => Err(EvalError::Type(format!(
                    "range bound must be an integer, found {}",
                    other.type_name()
                ))
                .into()),
            }
        };
        let child = node.children().next();
        match (node.kind(), op.as_deref()) {
            (NodeKind::PrefixExpr, Some("..<")) => {
                let hi = bound_int(self, &child.unwrap())?;
                Ok(Some(SwiftValue::Range {
                    lo: 0,
                    hi,
                    inclusive: false,
                }))
            }
            (NodeKind::PrefixExpr, Some("...")) => {
                let hi = bound_int(self, &child.unwrap())?;
                Ok(Some(SwiftValue::Range {
                    lo: 0,
                    hi,
                    inclusive: true,
                }))
            }
            (NodeKind::PostfixExpr, Some("...")) => {
                let lo = bound_int(self, &child.unwrap())?;
                Ok(Some(SwiftValue::Range {
                    lo,
                    hi: len,
                    inclusive: false,
                }))
            }
            _ => Ok(None),
        }
    }

    /// Evaluate a `static subscript` declared on `type_name`, addressed as
    /// `Type[index]`. No `self` is bound; only the index parameters are.
    fn read_static_subscript(&mut self, type_name: &str, indices: &[SwiftValue]) -> Eval {
        let (params, body) = {
            let m = self
                .structs
                .get(type_name)
                .and_then(|d| d.static_subscript.as_ref())
                .or_else(|| {
                    self.classes
                        .get(type_name)
                        .and_then(|d| d.static_subscript.as_ref())
                })
                .expect("static subscript exists");
            (clone_params(&m.params), m.body)
        };
        let args: Vec<CallArg> = indices
            .iter()
            .map(|v| CallArg {
                label: None,
                value: v.clone(),
                place: None,
            })
            .collect();
        self.env.push();
        let bound = self.bind_params(&params, args);
        let result = match bound {
            Ok(_) => match body {
                Some(b) => self.eval(&b),
                None => Ok(SwiftValue::Void),
            },
            Err(e) => Err(e),
        };
        self.env.pop();
        match result {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
    }

    /// Assign `base[index] = value` (compound ops supported) over arrays,
    /// dictionaries, and user `subscript { set }`s. A nested subscript base
    /// (`m[i][j] = v`) is handled by read-modify-write through `base`.
    fn assign_subscript(&mut self, target: &Node<'static>, rhs: &Node<'static>, op: &str) -> Eval {
        let mut kids = target.children();
        let base = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("subscript without a base".into()))?;
        let index_nodes: Vec<Node<'static>> = kids.collect();
        if index_nodes.is_empty() {
            return Err(EvalError::Unsupported("subscript without an index".into()).into());
        }
        let index_values: Vec<SwiftValue> = index_nodes
            .iter()
            .map(|n| self.eval(n))
            .collect::<Result<_, _>>()?;
        let current = self.eval(&base)?;

        // The leaf value to store: for a compound op, fold against the element
        // currently at that index.
        let new_value = if op == "=" {
            self.eval(rhs)?
        } else {
            let cur_elem = self.read_subscript(&current, &index_values)?;
            let r = self.eval(rhs)?;
            ops::binary(op.trim_end_matches('='), &cur_elem, &r).map_err(trap)?
        };

        // Remember the original class identity (if any) so an in-place mutation
        // can be told apart from a whole-value replacement below.
        let prev_ref = match &current {
            SwiftValue::Object(o) => Some(o.clone()),
            _ => None,
        };
        let updated = self.set_subscript_element(current, &index_values, new_value)?;
        // A class instance is a reference: when the write mutated *the same*
        // instance in place, there is nothing to rebind (and re-assigning to a
        // `let` binding would be illegal). A whole-value replacement
        // (`obj[keyPath: \.self] = other`) yields a different instance and still
        // writes back.
        if let (Some(prev), SwiftValue::Object(now)) = (&prev_ref, &updated) {
            if StdRc::ptr_eq(prev, now) {
                return Ok(SwiftValue::Void);
            }
        }
        self.assign_value_to(&base, updated)
    }

    /// Return a copy of `container` with `container[indices]` set to `value`,
    /// dispatching over arrays, dictionaries, and user struct subscript setters.
    fn set_subscript_element(
        &mut self,
        container: SwiftValue,
        indices: &[SwiftValue],
        value: SwiftValue,
    ) -> Eval {
        debug_assert!(
            !indices.is_empty(),
            "set_subscript_element requires at least one index"
        );
        // `container[keyPath: kp] = value` — write through a (writable) key path.
        if let [idx] = indices {
            if let Some(components) = self.keypath_components(idx) {
                return self.set_keypath(container, &components, value);
            }
        }
        // A user `subscript { set }` on a struct runs the setter with `self`
        // mutable, the index parameters, and the `newValue` binding.
        if let SwiftValue::Struct(obj) = &container {
            let type_name = obj.type_name.clone();
            let selected = self.structs.get(&type_name).and_then(|d| {
                d.subscripts
                    .iter()
                    .find(|s| s.params.len() == indices.len())
                    .map(|s| (clone_params(&s.params), s.setter, s.setter_param.clone()))
            });
            if let Some((params, setter, setter_param)) = selected {
                let setter_body = setter.ok_or_else(|| {
                    EvalError::Type(format!("{type_name} subscript is read-only"))
                })?;
                let args: Vec<CallArg> = indices
                    .iter()
                    .map(|v| CallArg {
                        label: None,
                        value: v.clone(),
                        place: None,
                    })
                    .collect();
                let saved_env = self.env.enter_isolated();
                self.env.declare("self", container.clone(), true);
                let bound = self.bind_params(&params, args);
                let outcome = match bound {
                    Ok(_) => {
                        self.env.declare(&setter_param, value, false);
                        self.eval(&setter_body)
                    }
                    Err(e) => Err(e),
                };
                let updated_self = self.env.get("self").unwrap_or_else(|| container.clone());
                self.env.restore(saved_env);
                match outcome {
                    Ok(_) | Err(Signal::Return(_)) => {}
                    Err(e) => return Err(e),
                }
                return Ok(updated_self);
            }
            return Err(EvalError::Type(format!(
                "{type_name} has no subscript taking {} index argument(s)",
                indices.len()
            ))
            .into());
        }

        let index_value = indices
            .first()
            .cloned()
            .expect("at least one index checked by caller");
        // `dict[key] = value` inserts/updates; `dict[key] = nil` removes.
        // When `indices.len() > 1` (e.g. `dict[k, default:]`), only
        // `indices[0]` is the key; the compound-op read already folded the
        // `default:` in via `read_subscript`, so extra indices are ignored here.
        if let SwiftValue::Dict(pairs) = &container {
            let mut new_pairs = pairs.as_ref().clone();
            let existing = new_pairs.iter().position(|(k, _)| *k == index_value);
            match (existing, matches!(value, SwiftValue::Nil)) {
                (Some(i), true) => {
                    new_pairs.remove(i);
                }
                (Some(i), false) => new_pairs[i].1 = value,
                (None, true) => {}
                (None, false) => new_pairs.push((index_value, value)),
            }
            return Ok(SwiftValue::Dict(StdRc::new(new_pairs)));
        }
        let idx = subscript_index(&[index_value])?;
        let SwiftValue::Array(items) = &container else {
            return Err(EvalError::Type("subscript assignment requires an array".into()).into());
        };
        if idx >= items.len() {
            return Err(trap(format!("index {idx} out of range")));
        }
        let mut new_items = items.as_ref().clone();
        new_items[idx] = value;
        Ok(SwiftValue::Array(StdRc::new(new_items)))
    }

    /// Write `value` back to the storage named by an lvalue `node`. A subscript
    /// lvalue recurses (so `m[i][j] = v` updates the inner container, then
    /// stores it back into the outer one); recursion terminates when the base is
    /// a variable/member rather than another subscript, which resolves to a
    /// place.
    fn assign_value_to(&mut self, node: &Node<'static>, value: SwiftValue) -> Eval {
        if node.kind() == NodeKind::SubscriptExpr {
            let mut kids = node.children();
            let inner_base = kids
                .next()
                .ok_or_else(|| EvalError::Unsupported("subscript without a base".into()))?;
            let idx_values: Vec<SwiftValue> = kids
                .collect::<Vec<_>>()
                .iter()
                .map(|n| self.eval(n))
                .collect::<Result<_, _>>()?;
            let container = self.eval(&inner_base)?;
            let updated = self.set_subscript_element(container, &idx_values, value)?;
            return self.assign_value_to(&inner_base, updated);
        }
        let place = self
            .resolve_place(node)
            .ok_or_else(|| EvalError::Unsupported("subscript target is not assignable".into()))?;
        self.write_place(&place, value)?;
        Ok(SwiftValue::Void)
    }

    /// Read `base[indices]`.
    fn read_subscript(&mut self, base: &SwiftValue, indices: &[SwiftValue]) -> Eval {
        // `base[keyPath: kp]` — a key-path subscript walks the path from `base`.
        if let [idx] = indices {
            if let Some(components) = self.keypath_components(idx) {
                return self.apply_keypath(base.clone(), &components);
            }
        }
        // `base[range]` — slice an array or string by an integer range
        // (two-sided `a..<b`/`a...b` or a one-sided partial range resolved
        // by `eval_subscript` against the collection length).
        if let [SwiftValue::Range { lo, hi, inclusive }] = indices {
            let (lo, hi, inclusive) = (*lo, *hi, *inclusive);
            match base {
                SwiftValue::Array(items) => {
                    let (start, end) = slice_bounds(lo, hi, inclusive, items.len())?;
                    return Ok(SwiftValue::Array(Rc::new(items[start..end].to_vec())));
                }
                SwiftValue::Str(s) => {
                    let chars = crate::graphemes(s);
                    let (start, end) = slice_bounds(lo, hi, inclusive, chars.len())?;
                    return Ok(SwiftValue::Str(chars[start..end].concat()));
                }
                _ => {}
            }
        }
        match base {
            SwiftValue::Array(items) => {
                let i = subscript_index(indices)?;
                items
                    .get(i)
                    .cloned()
                    .ok_or_else(|| trap(format!("index {i} out of range")))
            }
            // `dict[key]` → the value, or `nil` when absent. `dict[key, default:]`
            // returns the default instead of `nil` when the key is missing.
            SwiftValue::Dict(pairs) => {
                let key = indices
                    .first()
                    .ok_or_else(|| EvalError::Type("dictionary subscript needs a key".into()))?;
                Ok(pairs
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| indices.get(1).cloned().unwrap_or(SwiftValue::Nil)))
            }
            SwiftValue::Str(s) => {
                let i = subscript_index(indices)?;
                // Index by extended grapheme cluster (Swift `Character`), so
                // string indexing agrees with `count` and iteration.
                crate::graphemes(s)
                    .into_iter()
                    .nth(i)
                    .map(SwiftValue::Str)
                    .ok_or_else(|| trap(format!("string index {i} out of range")))
            }
            SwiftValue::Struct(obj) => {
                let type_name = obj.type_name.clone();
                // `IndexPath[i]` reads its `i`th element (a Foundation builtin
                // backed by a `_indexes` array, with no user `subscript`).
                if type_name == "IndexPath" {
                    if let Some(SwiftValue::Array(items)) = obj.get("_indexes") {
                        let i = subscript_index(indices)?;
                        return items
                            .get(i)
                            .cloned()
                            .ok_or_else(|| trap(format!("index {i} out of range")));
                    }
                }
                // Select the overload whose arity matches the index count.
                let getter = self.structs.get(&type_name).and_then(|d| {
                    d.subscripts
                        .iter()
                        .find(|s| s.params.len() == indices.len())
                        .map(|s| (clone_params(&s.params), s.getter))
                });
                if let Some((params, body)) = getter {
                    let args: Vec<CallArg> = indices
                        .iter()
                        .map(|v| CallArg {
                            label: None,
                            value: v.clone(),
                            place: None,
                        })
                        .collect();
                    let saved_env = self.env.enter_isolated();
                    self.env.declare("self", base.clone(), false);
                    let bound = self.bind_params(&params, args);
                    let result = match bound {
                        Ok(_) => match body {
                            Some(b) => self.eval(&b),
                            None => Ok(SwiftValue::Void),
                        },
                        Err(e) => Err(e),
                    };
                    self.env.restore(saved_env);
                    return match result {
                        Ok(v) => Ok(v),
                        Err(Signal::Return(v)) => Ok(v),
                        Err(e) => Err(e),
                    };
                }
                Err(EvalError::Type(format!("{type_name} has no subscript")).into())
            }
            other => Err(EvalError::Type(format!("cannot subscript {}", other.type_name())).into()),
        }
    }

    /// The default initializer of a lazy stored property, if `name` names one.
    fn lazy_default(&self, type_name: &str, name: &str) -> Option<Node<'static>> {
        self.structs.get(type_name).and_then(|d| {
            d.stored
                .iter()
                .find(|p| p.name == name && p.lazy)
                .and_then(|p| p.default)
        })
    }

    /// Whether a struct type declares a stored/computed property or method.
    fn struct_has_member(&self, type_name: &str, name: &str) -> bool {
        self.structs.get(type_name).is_some_and(|d| {
            d.computed.contains_key(name)
                || d.methods.contains_key(name)
                || d.stored.iter().any(|p| p.name == name)
        })
    }

    /// Read a property off a struct value: a stored field, or a computed
    /// getter run with `self` bound.
    fn read_struct_member(&mut self, value: &SwiftValue, name: &str) -> Eval {
        let SwiftValue::Struct(obj) = value else {
            return Err(EvalError::Type(format!(
                "`{name}` is not a member of {}",
                value.type_name()
            ))
            .into());
        };
        // Projected value `$name` reads the wrapper's `projectedValue`.
        if let Some(stripped) = name.strip_prefix('$') {
            if self.wrapped_field(&obj.type_name, stripped) {
                if let Some(wrapper) = obj.get(stripped).cloned() {
                    return self.read_struct_member(&wrapper, "projectedValue");
                }
            }
        }
        if let Some(v) = obj.get(name) {
            // A wrapped stored property exposes its wrapper's `wrappedValue`.
            if self.wrapped_field(&obj.type_name, name) {
                return self.read_struct_member(&v.clone(), "wrappedValue");
            }
            return Ok(v.clone());
        }
        let getter = self
            .structs
            .get(&obj.type_name)
            .and_then(|d| d.computed.get(name))
            .filter(|c| !c.is_static)
            .and_then(|c| c.getter)
            .or_else(|| self.protocol_default_getter(&obj.type_name, name));
        if let Some(body) = getter {
            return self
                .run_with_self(value.clone(), |me| me.eval(&body))
                .map(|(v, _)| v);
        }
        // `@dynamicMemberLookup`: an unresolved member name routes through a
        // `subscript(dynamicMember:)` getter, passing the name as a string.
        if let Some(v) = self.dynamic_member_read(value, &obj.type_name, name)? {
            return Ok(v);
        }
        Err(EvalError::Type(format!("struct {} has no member `{name}`", obj.type_name)).into())
    }

    /// Find a `subscript(dynamicMember:)` getter on `type_name` and invoke it
    /// with `member` as a string key. Returns `None` when the type declares no
    /// dynamic-member subscript, so the caller can fall through to its error.
    fn dynamic_member_read(
        &mut self,
        receiver: &SwiftValue,
        type_name: &str,
        member: &str,
    ) -> Result<Option<SwiftValue>, Signal> {
        let getter = self.structs.get(type_name).and_then(|d| {
            if !d.dynamic_member_lookup {
                return None;
            }
            // The dynamic-member subscript is the single `String`-keyed overload
            // (its `dynamicMember` argument label is not retained in the AST, so
            // the `@dynamicMemberLookup` attribute plus a one-`String`-parameter
            // signature identifies it — and disambiguates it from an ordinary
            // single-parameter subscript such as `subscript(_ i: Int)`).
            d.subscripts
                .iter()
                .find(|s| s.params.len() == 1 && s.params[0].ty.as_deref() == Some("String"))
                .map(|s| (clone_params(&s.params), s.getter))
        });
        let Some((params, body)) = getter else {
            return Ok(None);
        };
        let args = vec![CallArg {
            label: None,
            value: SwiftValue::Str(member.to_string()),
            place: None,
        }];
        let saved_env = self.env.enter_isolated();
        self.env.declare("self", receiver.clone(), false);
        let bound = self.bind_params(&params, args);
        let result = match bound {
            Ok(_) => match body {
                Some(b) => self.eval(&b),
                None => Ok(SwiftValue::Void),
            },
            Err(e) => Err(e),
        };
        self.env.restore(saved_env);
        match result {
            Ok(v) | Err(Signal::Return(v)) => Ok(Some(v)),
            Err(e) => Err(e),
        }
    }

    /// Whether `value` is an instance of a `@dynamicCallable` struct type.
    fn is_dynamic_callable(&self, value: &SwiftValue) -> bool {
        matches!(value, SwiftValue::Struct(obj)
            if self.structs.get(&obj.type_name).is_some_and(|d| d.dynamic_callable))
    }

    /// `@dynamicCallable`: route call syntax on a struct instance through its
    /// `dynamicallyCall(withArguments:)` (all positional) or
    /// `dynamicallyCall(withKeywordArguments:)` (any labelled) method.
    fn dynamic_call(&mut self, receiver: SwiftValue, args: Vec<CallArg>) -> Eval {
        let SwiftValue::Struct(obj) = &receiver else {
            return Err(EvalError::Type("dynamicallyCall on a non-struct".into()).into());
        };
        let type_name = obj.type_name.clone();
        // The declared `dynamicallyCall` parameter type decides the packing
        // form: a dictionary parameter (`[Key: Value]`) is the keyword form,
        // anything else is the positional array form. Fall back to the call
        // site's labels when the type is not introspectable.
        let keyword_param = self
            .structs
            .get(&type_name)
            .and_then(|d| d.methods.get("dynamicallyCall"))
            .and_then(|m| m.params.first())
            .and_then(|p| p.ty.as_deref())
            .map(|ty| ty.contains(':'));
        let use_keyword = keyword_param.unwrap_or_else(|| args.iter().any(|a| a.label.is_some()));
        // `withKeywordArguments:` receives the call's (label, value) pairs as a
        // dictionary keyed by the argument label (unlabelled → empty string);
        // `withArguments:` receives the positional values as an array.
        let (label, packed) = if use_keyword {
            let pairs: Vec<(SwiftValue, SwiftValue)> = args
                .into_iter()
                .map(|a| (SwiftValue::Str(a.label.unwrap_or_default()), a.value))
                .collect();
            ("withKeywordArguments", SwiftValue::Dict(Rc::new(pairs)))
        } else {
            let items: Vec<SwiftValue> = args.into_iter().map(|a| a.value).collect();
            ("withArguments", SwiftValue::Array(Rc::new(items)))
        };
        let call_args = vec![CallArg {
            label: Some(label.to_string()),
            value: packed,
            place: None,
        }];
        self.call_struct_method(receiver, &type_name, "dynamicallyCall", call_args, None)
    }

    /// Select a struct initializer overload by the call's labels and runtime
    /// value types, falling back to the last declared initializer when the
    /// overload set is ambiguous for compatibility with existing programs.
    fn select_struct_init(
        &self,
        type_name: &str,
        args: &[(Option<String>, SwiftValue)],
    ) -> Option<(Vec<Param>, Option<Node<'static>>)> {
        let def = self.structs.get(type_name)?;
        let call_args: Vec<CallArg> = args
            .iter()
            .map(|(label, value)| CallArg {
                label: label.clone(),
                value: value.clone(),
                place: None,
            })
            .collect();
        if def.init_overloads.len() > 1 {
            if let Some(init) = select_labeled_overload(&def.init_overloads, &call_args) {
                return Some((clone_params(&init.params), init.body));
            }
        }
        def.init
            .as_ref()
            .map(|init| (clone_params(&init.params), init.body))
    }

    /// Build a struct instance from a memberwise initializer call.
    fn instantiate_struct(
        &mut self,
        type_name: &str,
        args: &[(Option<String>, SwiftValue)],
    ) -> Eval {
        // A custom initializer runs against a fresh empty value, binding `self`.
        let custom_init = self.select_struct_init(type_name, args);
        if let Some((params, body)) = custom_init {
            // Stored properties with a default are initialized before the
            // initializer body runs (Swift gives each such property its default
            // value first; the body may then reassign it).
            let defaults: Vec<(String, Node<'static>)> = self
                .structs
                .get(type_name)
                .map(|d| {
                    d.stored
                        .iter()
                        .filter(|p| !p.lazy)
                        .filter_map(|p| p.default.map(|def| (p.name.clone(), def)))
                        .collect()
                })
                .unwrap_or_default();
            let mut fields: Vec<(String, SwiftValue)> = Vec::new();
            for (pname, def) in defaults {
                let value = self.eval(&def)?;
                // Wrap `@propertyWrapper` fields in their wrapper instance, the
                // same way the memberwise initializer does.
                let wrapper = self
                    .structs
                    .get(type_name)
                    .and_then(|d| d.wrappers.get(&pname))
                    .cloned();
                let value = match wrapper {
                    Some(wt) => {
                        self.instantiate_struct(&wt, &[(Some("wrappedValue".into()), value)])?
                    }
                    None => value,
                };
                fields.push((pname, value));
            }
            let this = SwiftValue::Struct(Rc::new(StructObj {
                type_name: type_name.to_string(),
                fields,
            }));
            let call_args: Vec<CallArg> = args
                .iter()
                .map(|(label, value)| CallArg {
                    label: label.clone(),
                    value: value.clone(),
                    place: None,
                })
                .collect();
            let saved_env = self.env.enter_isolated();
            self.env.declare("self", this, true);
            let bound = self.bind_params(&params, call_args);
            let result = match bound {
                Ok(_) => match body {
                    Some(b) => self.eval(&b),
                    None => Ok(SwiftValue::Void),
                },
                Err(e) => Err(e),
            };
            let built = self.env.get("self").unwrap_or(SwiftValue::Void);
            self.env.restore(saved_env);
            return match result {
                // A failable initializer that runs `return nil` produces the
                // absent optional rather than the half-built value.
                Err(Signal::Return(SwiftValue::Nil)) => Ok(SwiftValue::Nil),
                Ok(_) | Err(Signal::Return(_)) => Ok(built),
                Err(e) => Err(e),
            };
        }

        let plan: Vec<(String, Option<String>, bool, Option<Node<'static>>)> = self
            .structs
            .get(type_name)
            .map(|d| {
                d.stored
                    .iter()
                    .map(|p| (p.name.clone(), p.ty.clone(), p.lazy, p.default))
                    .collect()
            })
            .unwrap_or_default();

        let mut fields: Vec<(String, SwiftValue)> = Vec::new();
        let mut positional = args.iter().filter(|(l, _)| l.is_none());
        for (pname, field_ty, lazy, default) in plan {
            let labeled = args
                .iter()
                .find(|(l, _)| l.as_deref() == Some(pname.as_str()))
                .map(|(_, v)| v.clone());
            // The `@propertyWrapper` type backing this field, if any.
            let wrapper = self
                .structs
                .get(type_name)
                .and_then(|d| d.wrappers.get(&pname))
                .cloned();
            let value = if let Some(v) = labeled {
                coerce_numeric(v, field_ty.as_deref())
            } else if let Some((_, v)) = positional.next() {
                coerce_numeric(v.clone(), field_ty.as_deref())
            } else if lazy {
                // Lazy properties are materialized on first access, not here.
                continue;
            } else if let Some(def) = default {
                self.eval(&def)?
            } else if let Some(wt) = &wrapper {
                // A wrapped property with no provided value and no default (e.g.
                // `@EnvironmentObject var x: T`) is synthesized via the
                // wrapper's own no-argument `init()` — its value is injected
                // later (by the environment) rather than supplied here.
                let synthesized = self.instantiate_struct(wt, &[])?;
                fields.push((pname, synthesized));
                continue;
            } else {
                return Err(EvalError::Type(format!(
                    "missing value for property `{pname}` of {type_name}"
                ))
                .into());
            };
            // Wrap `@propertyWrapper` fields in their wrapper instance.
            let value = match &wrapper {
                Some(wt) => self.instantiate_struct(wt, &[(Some("wrappedValue".into()), value)])?,
                None => value,
            };
            fields.push((pname, value));
        }
        Ok(SwiftValue::Struct(Rc::new(StructObj {
            type_name: type_name.to_string(),
            fields,
        })))
    }

    /// The `@propertyWrapper` type of `field` on struct `type_name`, if any.
    fn wrapped_field(&self, type_name: &str, field: &str) -> bool {
        self.structs
            .get(type_name)
            .is_some_and(|d| d.wrappers.contains_key(field))
    }

    /// Run `body` with `self` bound to `this` in a fresh scope, returning the
    /// body's value and the (possibly mutated) `self`.
    fn run_with_self(
        &mut self,
        this: SwiftValue,
        body: impl FnOnce(&mut Self) -> Eval,
    ) -> Result<(SwiftValue, SwiftValue), Signal> {
        // Isolated from caller locals: a computed property/method body sees
        // globals, `self`, and its members — not enclosing variables.
        let saved_env = self.env.enter_isolated();
        self.env.declare("self", this, true);
        let result = body(self);
        let updated = self.env.get("self").unwrap_or(SwiftValue::Void);
        self.env.restore(saved_env);
        let value = match result {
            Ok(v) => v,
            Err(Signal::Return(v)) => v,
            Err(e) => return Err(e),
        };
        Ok((value, updated))
    }

    /// Set a property on a struct value, honoring computed setters and
    /// `willSet`/`didSet` observers. Returns the updated struct value.
    fn set_struct_field(
        &mut self,
        value: SwiftValue,
        name: &str,
        new_value: SwiftValue,
    ) -> Result<SwiftValue, Signal> {
        let type_name = match &value {
            SwiftValue::Struct(o) => o.type_name.clone(),
            _ => return Err(EvalError::Type("cannot set a member on a non-struct".into()).into()),
        };

        // A wrapped property's set goes through its wrapper's `wrappedValue`.
        if self.wrapped_field(&type_name, name) {
            let current = match &value {
                SwiftValue::Struct(o) => o.get(name).cloned(),
                _ => None,
            };
            if let Some(wrapper) = current {
                let updated = self.set_struct_field(wrapper, "wrappedValue", new_value)?;
                let mut value = value;
                if let SwiftValue::Struct(obj) = &mut value {
                    Rc::make_mut(obj).set(name, updated);
                }
                return Ok(value);
            }
        }

        let setter = self
            .structs
            .get(&type_name)
            .and_then(|d| d.computed.get(name))
            .map(|c| (c.setter, c.setter_param.clone()));
        if let Some((Some(body), param)) = setter {
            let param = param.unwrap_or_else(|| "newValue".into());
            let nv = new_value.clone();
            let (_, updated) = self.run_with_self(value, |me| {
                me.env.declare(&param, nv, false);
                me.eval(&body)
            })?;
            return Ok(updated);
        }

        let observers = self.structs.get(&type_name).and_then(|d| {
            d.stored
                .iter()
                .find(|p| p.name == name)
                .map(|p| (p.will_set.clone(), p.did_set.clone()))
        });
        let (will_set, did_set) = observers.unwrap_or((None, None));
        let old_value = match &value {
            SwiftValue::Struct(o) => o.get(name).cloned(),
            _ => None,
        };

        let mut value = value;
        if let Some((param, body)) = will_set {
            let nv = new_value.clone();
            let (_, updated) = self.run_with_self(value, |me| {
                me.env.declare(&param, nv, false);
                me.eval(&body)
            })?;
            value = updated;
        }
        if let SwiftValue::Struct(obj) = &mut value {
            Rc::make_mut(obj).set(name, new_value);
        }
        if let Some((param, body)) = did_set {
            let old = old_value.unwrap_or(SwiftValue::Void);
            let (_, updated) = self.run_with_self(value, |me| {
                me.env.declare(&param, old, false);
                me.eval(&body)
            })?;
            value = updated;
        }
        Ok(value)
    }

    /// An integer literal, widened to its msf-resolved type when known.
    fn eval_int_literal(&self, node: &Node<'static>) -> SwiftValue {
        let raw = node.int().unwrap_or(0) as i128;
        let width = node
            .type_name()
            .and_then(|n| IntWidth::from_type_name(&n))
            .unwrap_or(IntWidth::I64);
        SwiftValue::Int(IntValue::new(raw, width))
    }

    /// A binary operation, with short-circuiting `&&`/`||`.
    fn eval_binary(&mut self, node: &Node<'static>) -> Eval {
        let op = node
            .op_text()
            .ok_or_else(|| EvalError::Unsupported("binary without operator".into()))?;
        let mut kids = node.children();
        let lhs = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("binary without lhs".into()))?;
        let rhs = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("binary without rhs".into()))?;

        // Identity operators compare class instances by reference.
        if op == "===" || op == "!==" {
            let l = self.eval(&lhs)?;
            let r = self.eval(&rhs)?;
            let same = l == r;
            return Ok(SwiftValue::Bool(if op == "===" { same } else { !same }));
        }

        // Nil-coalescing: `lhs ?? rhs` evaluates `rhs` only when `lhs` is nil.
        if op == "??" {
            let l = self.eval(&lhs)?;
            return if matches!(l, SwiftValue::Nil) {
                self.eval(&rhs)
            } else {
                Ok(l)
            };
        }

        if op == "&&" || op == "||" {
            let l = self.eval(&lhs)?;
            let lb = l.as_bool().ok_or_else(|| {
                EvalError::Type(format!("`{op}` needs Bool, got {}", l.type_name()))
            })?;
            if op == "&&" && !lb {
                return Ok(SwiftValue::Bool(false));
            }
            if op == "||" && lb {
                return Ok(SwiftValue::Bool(true));
            }
            let r = self.eval(&rhs)?;
            let rb = r.as_bool().ok_or_else(|| {
                EvalError::Type(format!("`{op}` needs Bool, got {}", r.type_name()))
            })?;
            return Ok(SwiftValue::Bool(rb));
        }

        let l = self.eval(&lhs)?;
        let r = self.eval(&rhs)?;
        // Equality against nil / reference / compound values goes through the
        // structural comparison rather than the scalar operator table.
        if (op == "==" || op == "!=")
            && matches!(
                (&l, &r),
                (SwiftValue::Nil, _)
                    | (_, SwiftValue::Nil)
                    | (SwiftValue::Object(_), _)
                    | (_, SwiftValue::Object(_))
                    | (SwiftValue::Enum(_), _)
                    | (SwiftValue::Struct(_), _)
            )
        {
            let same = l == r;
            return Ok(SwiftValue::Bool(if op == "==" { same } else { !same }));
        }
        match ops::binary(&op, &l, &r) {
            Ok(v) => Ok(v),
            Err(e) => {
                // A static operator method (`Comparable`'s `<`, etc.) declared on
                // the operand's type: `static func < (a:T, b:T) -> Bool`.
                if let Some(tn) = self.value_type_name(&l) {
                    if self.type_has_method(&tn, &op) {
                        let args = vec![
                            CallArg {
                                label: None,
                                value: l.clone(),
                                place: None,
                            },
                            CallArg {
                                label: None,
                                value: r.clone(),
                                place: None,
                            },
                        ];
                        return self.call_struct_method(SwiftValue::Void, &tn, &op, args, None);
                    }
                    // Comparable derives `>`, `<=`, `>=` (and `<`) from a single
                    // `static func <`, so a type that defines only `<` still
                    // supports the other ordering operators.
                    if matches!(op.as_str(), "<" | ">" | "<=" | ">=") {
                        let derived = match op.as_str() {
                            "<" => self.value_less_than(&l, &r),
                            ">" => self.value_less_than(&r, &l),
                            "<=" => self.value_less_than(&r, &l).map(|gt| !gt),
                            ">=" => self.value_less_than(&l, &r).map(|lt| !lt),
                            _ => None,
                        };
                        if let Some(b) = derived {
                            return Ok(SwiftValue::Bool(b));
                        }
                    }
                }
                // A user-defined (custom) operator is a function named after it.
                if let Some(SwiftValue::Function(id)) = self.env.get(&op) {
                    let call_args = vec![
                        CallArg {
                            label: None,
                            value: l,
                            place: None,
                        },
                        CallArg {
                            label: None,
                            value: r,
                            place: None,
                        },
                    ];
                    self.call_function(id, call_args)
                } else {
                    Err(trap(e))
                }
            }
        }
    }

    /// `if cond { … } [else if …] [else { … }]`, including `if let`/`if case`
    /// bindings. Also serves `if` expressions: the taken branch's value.
    fn eval_if(&mut self, node: &Node<'static>) -> Eval {
        let kids: Vec<Node<'static>> = node.children().collect();
        let then_idx = kids
            .iter()
            .position(|c| c.kind() == NodeKind::Block)
            .ok_or_else(|| EvalError::Unsupported("if without a body".into()))?;
        let conds = &kids[..then_idx];
        let then = &kids[then_idx];
        let els = kids.get(then_idx + 1);

        // Bindings from `if let` live only inside the then-branch scope.
        self.env.push();
        let passed = self.eval_cond_list(conds);
        let result = match passed {
            Ok(true) => self.eval(then),
            Ok(false) => {
                self.env.pop();
                return match els {
                    Some(e) => self.eval(e),
                    None => Ok(SwiftValue::Void),
                };
            }
            Err(e) => Err(e),
        };
        self.env.pop();
        result
    }

    /// `guard <conds> else { … }`. Bindings persist in the enclosing scope; the
    /// else block (which must transfer control) runs when a condition fails.
    fn eval_guard(&mut self, node: &Node<'static>) -> Eval {
        let kids: Vec<Node<'static>> = node.children().collect();
        let els_idx = kids
            .iter()
            .rposition(|c| c.kind() == NodeKind::Block)
            .ok_or_else(|| EvalError::Unsupported("guard without else".into()))?;
        let conds = &kids[..els_idx];
        let els = &kids[els_idx];
        if self.eval_cond_list(conds)? {
            Ok(SwiftValue::Void)
        } else {
            self.eval(els)
        }
    }

    /// Evaluate a comma-separated condition list, binding any `if let`/`guard
    /// let` optionals into the current scope. Returns whether all passed.
    fn eval_cond_list(&mut self, conds: &[Node<'static>]) -> Result<bool, Signal> {
        for cond in conds {
            match cond.kind() {
                // A `let`/`var` binding condition. The binding pattern is the
                // first child; an optional type annotation and the
                // initializer/subject follow.
                //
                // - simple optional binding (`if let x = e`, `if let x`): a
                //   `NamePattern` pattern — unwrap the optional (fail on
                //   nil) and bind the name.
                // - refutable match (`if case .a(let v) = e`): any other
                //   pattern — match it against the subject and bind on success.
                NodeKind::LetDecl | NodeKind::VarDecl => {
                    let pattern = cond.children().next().ok_or_else(|| {
                        EvalError::Unsupported("condition binding without a pattern".into())
                    })?;
                    // The subject/initializer follows the pattern (and any type
                    // annotation). Search *past* the first child so a value
                    // pattern (`if case 1 = x`) is never mistaken for the subject.
                    let init = cond.children().skip(1).find(|c| is_expr(c));
                    match pattern.kind() {
                        // Simple optional binding: `if let x = e`, `if let x`
                        // (shorthand), `if let _ = e`. Unwrap the optional
                        // (fail on nil); a value binding binds its name, a
                        // wildcard binds nothing.
                        NodeKind::NamePattern | NodeKind::WildcardPattern => {
                            let value = match init {
                                Some(expr) => self.eval(&expr)?,
                                None => {
                                    let name = pattern.text().ok_or_else(|| {
                                        EvalError::Unsupported("binding without a name".into())
                                    })?;
                                    self.env
                                        .get(&name)
                                        .ok_or_else(|| EvalError::UnknownVariable(name))?
                                }
                            };
                            if matches!(value, SwiftValue::Nil) {
                                return Ok(false);
                            }
                            if pattern.kind() == NodeKind::NamePattern {
                                if let Some(name) = pattern.text() {
                                    self.env.declare(&name, value, false);
                                }
                            }
                        }
                        // Refutable match: `if case .a(let v) = e`.
                        _ => {
                            let subject = match init {
                                Some(expr) => self.eval(&expr)?,
                                None => {
                                    return Err(EvalError::Unsupported(
                                        "case condition without a subject".into(),
                                    )
                                    .into())
                                }
                            };
                            match self.match_pattern(&pattern, &subject)? {
                                Some(binds) => {
                                    for (name, value) in binds {
                                        self.env.declare(&name, value, false);
                                    }
                                }
                                None => return Ok(false),
                            }
                        }
                    }
                }
                _ => {
                    if !self.eval_condition(cond)? {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    /// `while <conds> { … }`. The condition list may bind optionals (`while let
    /// x = …`) or match patterns (`while case …`), re-evaluated each iteration
    /// in a fresh scope that also holds the loop body's bindings.
    fn eval_while(&mut self, node: &Node<'static>) -> Eval {
        let kids: Vec<Node<'static>> = node.children().collect();
        let body_idx = kids
            .iter()
            .position(|c| c.kind() == NodeKind::Block)
            .ok_or_else(|| EvalError::Unsupported("while without body".into()))?;
        let conds = &kids[..body_idx];
        let body = &kids[body_idx];
        let label = node.loop_label();
        loop {
            // Each iteration's `while let` bindings live only for that pass.
            self.env.push();
            match self.eval_cond_list(conds) {
                Ok(true) => {}
                Ok(false) => {
                    self.env.pop();
                    break;
                }
                Err(e) => {
                    self.env.pop();
                    return Err(e);
                }
            }
            let flow = self.run_loop_body(body, &label);
            self.env.pop();
            match flow? {
                LoopFlow::Continue => {}
                LoopFlow::Break => break,
            }
        }
        Ok(SwiftValue::Void)
    }

    /// `repeat { … } while cond`.
    fn eval_repeat(&mut self, node: &Node<'static>) -> Eval {
        let kids: Vec<Node<'static>> = node.children().collect();
        let body = kids
            .first()
            .ok_or_else(|| EvalError::Unsupported("repeat without body".into()))?;
        let cond = kids
            .last()
            .ok_or_else(|| EvalError::Unsupported("repeat without condition".into()))?;
        let label = node.loop_label();
        loop {
            if let LoopFlow::Break = self.run_loop_body(body, &label)? {
                break;
            }
            if !self.eval_condition(cond)? {
                break;
            }
        }
        Ok(SwiftValue::Void)
    }

    /// `for v in seq [where cond] { … }` over an integer range or array.
    fn eval_for(&mut self, node: &Node<'static>) -> Eval {
        // `for await r in seq`: the loop carries the `async` effect modifier.
        let is_for_await = node.is_async();
        // The binding is the first pattern child, before the iterable. A simple
        // `for x in` / `for _ in` yields a name/wildcard binding read directly
        // into `var_name`; a refutable `for case <pattern> in` keeps the pattern
        // child so it can filter and destructure each element.
        let mut var_name = None;
        let mut pattern = None;
        let mut iterable = None;
        let mut where_clause = None;
        let mut body = None;
        for child in node.children() {
            match child.kind() {
                NodeKind::Param => {}
                NodeKind::Block => body = Some(child),
                NodeKind::NamePattern
                    if var_name.is_none() && pattern.is_none() && iterable.is_none() =>
                {
                    var_name = Some(child.text().unwrap_or_else(|| "_".to_string()));
                }
                NodeKind::WildcardPattern
                    if var_name.is_none() && pattern.is_none() && iterable.is_none() =>
                {
                    var_name = Some("_".to_string());
                }
                k if is_pattern_node(k)
                    && pattern.is_none()
                    && var_name.is_none()
                    && iterable.is_none() =>
                {
                    pattern = Some(child);
                }
                _ => {
                    if iterable.is_none() {
                        iterable = Some(child);
                    } else {
                        where_clause = Some(child);
                    }
                }
            }
        }
        if var_name.is_none() && pattern.is_none() {
            return Err(EvalError::Unsupported("for-loop without a binding".into()).into());
        }
        let iterable =
            iterable.ok_or_else(|| EvalError::Unsupported("for-loop without a sequence".into()))?;
        let body = body.ok_or_else(|| EvalError::Unsupported("for-loop without a body".into()))?;
        let label = node.loop_label();

        let seq = self.eval(&iterable)?;
        // `for await r in group`: each iteration consumes one finished child.
        // `for await r in customSequence`: drive its async iterator protocol.
        let items = match &seq {
            SwiftValue::TaskGroup(gid) => self.drain_group_results(*gid)?,
            _ if is_for_await && !is_builtin_iterable(&seq) => {
                let name = var_name.as_deref().unwrap_or("_");
                return self.run_async_sequence(&seq, name, where_clause, &body, &label);
            }
            // A user type conforming to `Sequence`/`IteratorProtocol`: drive its
            // iterator lazily so infinite sequences with `break` terminate.
            _ if !is_builtin_iterable(&seq) && self.is_custom_sequence(&seq) => {
                return self.run_sync_sequence(
                    &seq,
                    var_name.as_deref(),
                    pattern,
                    where_clause,
                    &body,
                    &label,
                );
            }
            _ => self.iterate(&seq)?,
        };

        for item in items {
            // A fresh scope per iteration so a closure/task created in the body
            // captures *this* iteration's binding (Swift's per-iteration `let`),
            // not a single shared, mutated slot.
            self.env.push();
            // A `for case` pattern that fails to match skips the element.
            if let Some(pat) = pattern {
                match self.match_pattern(&pat, &item)? {
                    Some(binds) => {
                        for (name, value) in binds {
                            self.env.declare(&name, value, false);
                        }
                    }
                    None => {
                        self.env.pop();
                        continue;
                    }
                }
            } else if let Some(name) = &var_name {
                self.env.declare(name, item, false);
            }
            if let Some(w) = where_clause {
                match self.eval_condition(&w) {
                    Ok(true) => {}
                    Ok(false) => {
                        self.env.pop();
                        continue;
                    }
                    Err(s) => {
                        self.env.pop();
                        return Err(s);
                    }
                }
            }
            let flow = self.run_loop_body(&body, &label);
            self.env.pop();
            match flow {
                Ok(LoopFlow::Continue) => {}
                Ok(LoopFlow::Break) => break,
                Err(s) => return Err(s),
            }
        }
        Ok(SwiftValue::Void)
    }

    /// `for await x in seq` over a custom `AsyncSequence`: obtain its async
    /// iterator, then drive `next()` (which may itself `await`) until it yields
    /// `nil`. The iterator is `mutating`, so it lives in a temp binding that the
    /// method call writes back to between iterations (ADR-0005).
    fn run_async_sequence(
        &mut self,
        seq: &SwiftValue,
        var_name: &str,
        where_clause: Option<Node<'static>>,
        body: &Node<'static>,
        label: &Option<String>,
    ) -> Eval {
        const ITER: &str = "$asynciter";
        // A type that *is* its own iterator (conforms to AsyncIteratorProtocol)
        // skips `makeAsyncIterator`; otherwise we ask the sequence for one.
        let seq_ty = self.value_type_name(seq);
        let iter = if seq_ty
            .as_deref()
            .is_some_and(|t| self.type_has_method(t, "next"))
        {
            seq.clone()
        } else if seq_ty
            .as_deref()
            .is_some_and(|t| self.type_has_method(t, "makeAsyncIterator"))
        {
            let ty = seq_ty.clone().unwrap();
            self.call_struct_method(seq.clone(), &ty, "makeAsyncIterator", Vec::new(), None)?
        } else {
            return Err(EvalError::Type(format!(
                "cannot iterate over {} (not an AsyncSequence)",
                seq.type_name()
            ))
            .into());
        };
        let iter_ty = self
            .value_type_name(&iter)
            .ok_or_else(|| EvalError::Type("async iterator has no type".into()))?;

        self.env.push();
        self.env.declare(ITER, iter, true);
        let outcome = loop {
            let current = self.env.get(ITER).unwrap_or(SwiftValue::Nil);
            let place = Place {
                root: ITER.into(),
                path: Vec::new(),
            };
            let next =
                match self.call_struct_method(current, &iter_ty, "next", Vec::new(), Some(place)) {
                    Ok(v) => v,
                    Err(e) => break Err(e),
                };
            // `next()` returns `Element?`: `nil` ends the sequence.
            if matches!(next, SwiftValue::Nil) {
                break Ok(SwiftValue::Void);
            }
            self.env.push();
            self.env.declare(var_name, next, false);
            if let Some(w) = where_clause {
                match self.eval_condition(&w) {
                    Ok(true) => {}
                    Ok(false) => {
                        self.env.pop();
                        continue;
                    }
                    Err(s) => {
                        self.env.pop();
                        break Err(s);
                    }
                }
            }
            let flow = self.run_loop_body(body, label);
            self.env.pop();
            match flow {
                Ok(LoopFlow::Continue) => {}
                Ok(LoopFlow::Break) => break Ok(SwiftValue::Void),
                Err(s) => break Err(s),
            }
        };
        self.env.pop();
        outcome
    }

    /// Whether `value` declares `Sequence`/`IteratorProtocol` conformance and
    /// exposes the corresponding iteration method.
    fn is_custom_sequence(&self, value: &SwiftValue) -> bool {
        self.value_type_name(value).is_some_and(|t| {
            (self.is_sequence_conformer(&t) && self.seq_type_has_method(&t, "makeIterator"))
                || (self.is_iterator_conformer(&t) && self.seq_type_has_method(&t, "next"))
        })
    }

    fn is_sequence_conformer(&self, type_name: &str) -> bool {
        self.all_protocols(type_name)
            .iter()
            .any(|p| p == "Sequence")
    }

    fn is_iterator_conformer(&self, type_name: &str) -> bool {
        self.all_protocols(type_name)
            .iter()
            .any(|p| p == "IteratorProtocol")
    }

    /// `type_has_method` extended to class declarations (walking the chain), for
    /// custom-sequence detection over struct/enum/class conformers.
    fn seq_type_has_method(&self, type_name: &str, method: &str) -> bool {
        self.type_has_method(type_name, method)
            || (self.classes.contains_key(type_name)
                && self.lookup_method(type_name, method).is_some())
    }

    /// Dispatch `next()`/`makeIterator()` on a sequence/iterator value, routing
    /// a class receiver through dynamic dispatch and a struct/enum receiver
    /// through the value-method path (writing the mutated iterator back to
    /// `place`).
    fn call_sequence_method(
        &mut self,
        receiver: SwiftValue,
        type_name: &str,
        method: &str,
        place: Option<Place>,
    ) -> Eval {
        if self.classes.contains_key(type_name) {
            // A class iterator mutates through its reference; no write-back.
            self.dispatch_class_method(receiver, type_name, method, Vec::new())
        } else {
            self.call_struct_method(receiver, type_name, method, Vec::new(), place)
        }
    }

    /// `for x in seq` over a custom `Sequence`/`IteratorProtocol`: obtain the
    /// iterator (the value itself if it has `next()`, else `makeIterator()`),
    /// then drive the mutating `next()` until it yields `nil`, running the loop
    /// body for each element. Supports a binding name or a `for case` pattern.
    fn run_sync_sequence(
        &mut self,
        seq: &SwiftValue,
        var_name: Option<&str>,
        pattern: Option<Node<'static>>,
        where_clause: Option<Node<'static>>,
        body: &Node<'static>,
        label: &Option<String>,
    ) -> Eval {
        const ITER: &str = "$synciter";
        let seq_ty = self
            .value_type_name(seq)
            .ok_or_else(|| EvalError::Type("sequence has no type".into()))?;
        // A Sequence with a makeIterator() method is driven through that method
        // even if it also happens to expose a helper named next(). A type that
        // only conforms as an IteratorProtocol is its own iterator.
        let iter = if self.is_sequence_conformer(&seq_ty)
            && self.seq_type_has_method(&seq_ty, "makeIterator")
        {
            self.call_sequence_method(seq.clone(), &seq_ty, "makeIterator", None)?
        } else {
            seq.clone()
        };
        let iter_ty = self
            .value_type_name(&iter)
            .ok_or_else(|| EvalError::Type("iterator has no type".into()))?;

        self.env.push();
        self.env.declare(ITER, iter, true);
        let outcome = loop {
            let current = self.env.get(ITER).unwrap_or(SwiftValue::Nil);
            let place = Place {
                root: ITER.into(),
                path: Vec::new(),
            };
            let next = match self.call_sequence_method(current, &iter_ty, "next", Some(place)) {
                Ok(v) => v,
                Err(e) => break Err(e),
            };
            // `next()` returns `Element?`: `nil` ends the sequence.
            if matches!(next, SwiftValue::Nil) {
                break Ok(SwiftValue::Void);
            }
            self.env.push();
            // A `for case` pattern that fails to match skips the element.
            if let Some(pat) = pattern {
                match self.match_pattern(&pat, &next) {
                    Ok(Some(binds)) => {
                        for (name, value) in binds {
                            self.env.declare(&name, value, false);
                        }
                    }
                    Ok(None) => {
                        self.env.pop();
                        continue;
                    }
                    Err(s) => {
                        self.env.pop();
                        break Err(s);
                    }
                }
            } else if let Some(name) = var_name {
                self.env.declare(name, next, false);
            }
            if let Some(w) = where_clause {
                match self.eval_condition(&w) {
                    Ok(true) => {}
                    Ok(false) => {
                        self.env.pop();
                        continue;
                    }
                    Err(s) => {
                        self.env.pop();
                        break Err(s);
                    }
                }
            }
            let flow = self.run_loop_body(body, label);
            self.env.pop();
            match flow {
                Ok(LoopFlow::Continue) => {}
                Ok(LoopFlow::Break) => break Ok(SwiftValue::Void),
                Err(s) => break Err(s),
            }
        };
        self.env.pop();
        outcome
    }

    /// Eagerly drive a custom `Sequence`/`IteratorProtocol` into an array of
    /// elements for standard-library sequence algorithms.
    fn materialize_custom_sequence(&mut self, seq: SwiftValue) -> Result<Vec<SwiftValue>, Signal> {
        const ITER: &str = "$algoiter";
        let seq_ty = self
            .value_type_name(&seq)
            .ok_or_else(|| EvalError::Type("sequence has no type".into()))?;
        let iter = if self.is_sequence_conformer(&seq_ty)
            && self.seq_type_has_method(&seq_ty, "makeIterator")
        {
            self.call_sequence_method(seq, &seq_ty, "makeIterator", None)?
        } else {
            seq
        };
        let iter_ty = self
            .value_type_name(&iter)
            .ok_or_else(|| EvalError::Type("iterator has no type".into()))?;
        self.env.push();
        self.env.declare(ITER, iter, true);
        let mut items = Vec::new();
        let result = loop {
            if items.len() >= MAX_SEQUENCE_MATERIALIZE {
                break Err(trap(format!(
                    "custom sequence algorithm exceeded {MAX_SEQUENCE_MATERIALIZE} elements"
                )));
            }
            let current = self.env.get(ITER).unwrap_or(SwiftValue::Nil);
            let place = Place {
                root: ITER.into(),
                path: Vec::new(),
            };
            let next = match self.call_sequence_method(current, &iter_ty, "next", Some(place)) {
                Ok(next) => next,
                Err(err) => break Err(err),
            };
            if matches!(next, SwiftValue::Nil) {
                break Ok(items);
            }
            items.push(next);
        };
        self.env.pop();
        result
    }

    /// The nominal type name backing a value, for protocol/method lookup.
    fn value_type_name(&self, value: &SwiftValue) -> Option<String> {
        match value {
            SwiftValue::Struct(o) => Some(o.type_name.clone()),
            SwiftValue::Enum(e) => Some(e.type_name.clone()),
            SwiftValue::Object(o) => Some(o.borrow().class_name.clone()),
            _ => None,
        }
    }

    /// Expand a sequence value (range or array) into the values to iterate.
    fn iterate(&self, seq: &SwiftValue) -> Result<Vec<SwiftValue>, Signal> {
        match seq {
            SwiftValue::Range { lo, hi, inclusive } => {
                let end = if *inclusive { *hi + 1 } else { *hi };
                Ok((*lo..end).map(SwiftValue::int).collect())
            }
            SwiftValue::Array(items) => Ok(items.as_ref().clone()),
            // Iterating a dictionary yields `(key:, value:)` tuples.
            SwiftValue::Dict(pairs) => Ok(pairs
                .iter()
                .map(|(k, v)| dict_element_tuple(k.clone(), v.clone()))
                .collect()),
            SwiftValue::Set(items) => Ok(items.as_ref().clone()),
            SwiftValue::Str(s) => Ok(crate::graphemes(s)
                .into_iter()
                .map(SwiftValue::Str)
                .collect()),
            other => {
                Err(EvalError::Type(format!("cannot iterate over {}", other.type_name())).into())
            }
        }
    }

    /// Evaluate a loop body, mapping `break`/`continue` (with optional labels) to
    /// the corresponding [`LoopFlow`]; other signals propagate.
    fn run_loop_body(
        &mut self,
        body: &Node<'static>,
        label: &Option<String>,
    ) -> Result<LoopFlow, Signal> {
        match self.eval(body) {
            Ok(_) => Ok(LoopFlow::Continue),
            Err(Signal::Break(None)) => Ok(LoopFlow::Break),
            Err(Signal::Break(Some(l))) if Some(&l) == label.as_ref() => Ok(LoopFlow::Break),
            Err(Signal::Continue(None)) => Ok(LoopFlow::Continue),
            Err(Signal::Continue(Some(l))) if Some(&l) == label.as_ref() => Ok(LoopFlow::Continue),
            Err(other) => Err(other),
        }
    }

    /// `switch subject { case …: … default: … }`.
    fn eval_switch(&mut self, node: &Node<'static>) -> Eval {
        let kids: Vec<Node<'static>> = node.children().collect();
        let subject_node = kids
            .first()
            .ok_or_else(|| EvalError::Unsupported("switch without a subject".into()))?;
        let subject = self.eval(subject_node)?;
        let cases: Vec<Node<'static>> = kids[1..]
            .iter()
            .copied()
            .filter(|c| c.kind() == NodeKind::CaseClause)
            .collect();

        // Find the first matching case.
        let mut chosen = None;
        for (i, case) in cases.iter().enumerate() {
            if let Some(binds) = self.case_matches(case, &subject)? {
                chosen = Some((i, binds));
                break;
            }
        }

        let Some((start, mut binds)) = chosen else {
            return Ok(SwiftValue::Void);
        };
        let mut idx = start;
        loop {
            let (_, body) = case_parts(&cases[idx]);
            self.env.push();
            for (name, value) in &binds {
                self.env.declare(name, value.clone(), false);
            }
            let mut fell_through = false;
            let mut propagate = None;
            for stmt in &body {
                match self.eval(stmt) {
                    Ok(_) => {}
                    Err(Signal::Fallthrough) => {
                        fell_through = true;
                        break;
                    }
                    Err(Signal::Break(None)) => break,
                    Err(other) => {
                        propagate = Some(other);
                        break;
                    }
                }
            }
            self.env.pop();
            if let Some(sig) = propagate {
                return Err(sig);
            }
            if fell_through && idx + 1 < cases.len() {
                idx += 1;
                binds = Vec::new();
                continue;
            }
            break;
        }
        Ok(SwiftValue::Void)
    }

    /// Whether `case` matches `subject`, returning the names it binds.
    fn case_matches(
        &mut self,
        case: &Node<'static>,
        subject: &SwiftValue,
    ) -> Result<Option<Vec<(String, SwiftValue)>>, Signal> {
        let info = case.case_info();
        if info.is_default {
            return Ok(Some(Vec::new()));
        }
        let (patterns, _) = case_parts(case);
        for pattern in patterns {
            if let Some(binds) = self.match_pattern(&pattern, subject)? {
                if let Some(guard) = info.where_expr {
                    self.env.push();
                    for (name, value) in &binds {
                        self.env.declare(name, value.clone(), false);
                    }
                    let pass = self.eval_condition(&guard);
                    self.env.pop();
                    if !pass? {
                        continue;
                    }
                }
                return Ok(Some(binds));
            }
        }
        Ok(None)
    }

    /// Try to match a single pattern against `subject`. `Ok(Some(binds))` on a
    /// match (with any bound names), `Ok(None)` on a non-match.
    fn match_pattern(
        &mut self,
        pattern: &Node<'static>,
        subject: &SwiftValue,
    ) -> Result<Option<Vec<(String, SwiftValue)>>, Signal> {
        match pattern.kind() {
            NodeKind::WildcardPattern => Ok(Some(Vec::new())),
            NodeKind::NamePattern => {
                let name = pattern.text().unwrap_or_default();
                Ok(Some(vec![(name, subject.clone())]))
            }
            NodeKind::RangePattern => {
                let bounds: Vec<Node<'static>> = pattern.children().collect();
                let marker = pattern.text();
                // One-sided range patterns carry a single bound tagged by
                // direction: `..<n` (upTo), `...n` (through), `n...` (from).
                if bounds.len() == 1 {
                    let bound = self.eval(&bounds[0])?;
                    let within = match (subject, &bound) {
                        (SwiftValue::Int(s), SwiftValue::Int(b)) => match marker.as_deref() {
                            Some("from") => s.raw >= b.raw,
                            Some("through") => s.raw <= b.raw,
                            Some("upTo") => s.raw < b.raw,
                            _ => return Ok(None),
                        },
                        _ => return Ok(None),
                    };
                    return Ok(if within { Some(Vec::new()) } else { None });
                }
                if bounds.len() != 2 {
                    return Ok(None);
                }
                let lo = self.eval(&bounds[0])?;
                let hi = self.eval(&bounds[1])?;
                let inclusive = marker.as_deref() == Some("...");
                if let (SwiftValue::Int(s), SwiftValue::Int(a), SwiftValue::Int(b)) =
                    (subject, &lo, &hi)
                {
                    let within = s.raw >= a.raw
                        && (if inclusive {
                            s.raw <= b.raw
                        } else {
                            s.raw < b.raw
                        });
                    return Ok(if within { Some(Vec::new()) } else { None });
                }
                Ok(None)
            }
            NodeKind::EnumCasePattern => {
                let case_name = pattern.op_text().unwrap_or_default();
                // The leading `TypeIdent` (e.g. the `E` in `E.bad`) is not a
                // sub-pattern; only payload bindings are.
                let subs: Vec<Node<'static>> = pattern
                    .children()
                    .filter(|c| c.kind() != NodeKind::TypeRef)
                    .collect();
                // Optional patterns desugar to `.some`/`.none`.
                if case_name == "some" {
                    if matches!(subject, SwiftValue::Nil) {
                        return Ok(None);
                    }
                    return match subs.first() {
                        Some(p) => self.match_pattern(p, subject),
                        None => Ok(Some(Vec::new())),
                    };
                }
                if case_name == "none" {
                    return Ok(if matches!(subject, SwiftValue::Nil) {
                        Some(Vec::new())
                    } else {
                        None
                    });
                }
                let SwiftValue::Enum(e) = subject else {
                    return Ok(None);
                };
                if e.case != case_name {
                    return Ok(None);
                }
                if !subs.is_empty() && subs.len() != e.payload.len() {
                    return Ok(None);
                }
                let mut all = Vec::new();
                for (sub, item) in subs.iter().zip(e.payload.iter()) {
                    match self.match_pattern(sub, item)? {
                        Some(b) => all.extend(b),
                        None => return Ok(None),
                    }
                }
                Ok(Some(all))
            }
            // `<pattern> as Type` — a cast pattern matches only when the
            // subject's dynamic type is `Type`, then binds the inner pattern.
            NodeKind::CastExpr => {
                let kids: Vec<Node<'static>> = pattern.children().collect();
                let Some(inner) = kids.first() else {
                    return Ok(None);
                };
                let ty = kids.get(1).and_then(|t| t.text()).unwrap_or_default();
                if self.value_is_type(subject, &ty) {
                    self.match_pattern(inner, subject)
                } else {
                    Ok(None)
                }
            }
            NodeKind::TuplePattern => {
                let SwiftValue::Tuple(items, _) = subject else {
                    return Ok(None);
                };
                let subs: Vec<Node<'static>> = pattern.children().collect();
                if subs.len() != items.len() {
                    return Ok(None);
                }
                let mut all = Vec::new();
                for (sub, item) in subs.iter().zip(items.iter()) {
                    match self.match_pattern(sub, item)? {
                        Some(b) => all.extend(b),
                        None => return Ok(None),
                    }
                }
                Ok(Some(all))
            }
            // An expression pattern: match by equality.
            _ => {
                let v = self.eval(pattern)?;
                Ok(if values_equal(&v, subject) {
                    Some(Vec::new())
                } else {
                    None
                })
            }
        }
    }

    /// Evaluate a node expected to yield a `Bool`.
    fn eval_condition(&mut self, node: &Node<'static>) -> Result<bool, Signal> {
        let v = self.eval(node)?;
        v.as_bool().ok_or_else(|| {
            EvalError::Type(format!("condition is not Bool: {}", v.type_name())).into()
        })
    }

    /// A ternary `cond ? a : b`, evaluating only the taken branch.
    fn eval_ternary(&mut self, node: &Node<'static>) -> Eval {
        let mut kids = node.children();
        let cond = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("ternary without condition".into()))?;
        let then = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("ternary without then-branch".into()))?;
        let els = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("ternary without else-branch".into()))?;
        let c = self.eval(&cond)?;
        let taken = c
            .as_bool()
            .ok_or_else(|| EvalError::Type(format!("ternary needs Bool, got {}", c.type_name())))?;
        if taken {
            self.eval(&then)
        } else {
            self.eval(&els)
        }
    }

    /// A unary operation (`-x`, `!b`, `~n`).
    fn eval_unary(&mut self, node: &Node<'static>) -> Eval {
        let op = node
            .op_text()
            .or_else(|| node.text())
            .ok_or_else(|| EvalError::Unsupported("unary without operator".into()))?;
        let operand = node
            .children()
            .next()
            .ok_or_else(|| EvalError::Unsupported("unary without operand".into()))?;
        let v = self.eval(&operand)?;
        // Ownership operators are transparent in the tree-walker: `consume x`,
        // `copy x`, and `borrow x` evaluate to the operand's value.
        if matches!(op.as_str(), "consume" | "copy" | "borrow") {
            return Ok(v);
        }
        ops::unary(&op, &v).map_err(trap)
    }

    /// Assignment: plain `=` and compound `+=`, `-=`, … to a binding.
    fn eval_assign(&mut self, node: &Node<'static>) -> Eval {
        let op = node
            .op_text()
            .ok_or_else(|| EvalError::Unsupported("assignment without operator".into()))?;
        let mut kids = node.children();
        let target = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("assignment without target".into()))?;
        let rhs = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("assignment without value".into()))?;

        // Tuple-destructuring assignment `(a, b) = (b, a + b)`: evaluate the
        // whole right side first (so swaps read the old values), then write each
        // element back through its own lvalue.
        if target.kind() == NodeKind::TupleExpr && op == "=" {
            let value = self.eval(&rhs)?;
            let targets: Vec<Node<'static>> = target.children().collect();
            self.assign_destructured(&targets, value)?;
            return Ok(SwiftValue::Void);
        }

        // Member assignment whose base is a class instance mutates in place
        // (reference semantics) rather than through a copy-on-write place.
        if target.kind() == NodeKind::MemberExpr {
            let field = target
                .text()
                .ok_or_else(|| EvalError::Unsupported("member assignment without a name".into()))?;
            let base = target
                .children()
                .next()
                .ok_or_else(|| EvalError::Unsupported("member assignment without a base".into()))?;
            // `Type.prop = value` — assign a type-level (static) stored property.
            if base.kind() == NodeKind::IdentExpr {
                if let Some(tn) = base.text() {
                    let key = format!("{tn}.{field}");
                    if self.env.get(&tn).is_none() && self.statics.contains_key(&key) {
                        let new_value = if op == "=" {
                            self.eval(&rhs)?
                        } else {
                            let current = self.statics[&key].clone();
                            let r = self.eval(&rhs)?;
                            ops::binary(op.trim_end_matches('='), &current, &r).map_err(trap)?
                        };
                        self.statics.insert(key, new_value);
                        return Ok(SwiftValue::Void);
                    }
                }
            }
            let base_value = self.eval(&base)?;
            if let SwiftValue::Object(obj) = &base_value {
                let new_value = if op == "=" {
                    self.eval(&rhs)?
                } else {
                    let current = self.read_object_member(&base_value, &field)?;
                    let r = self.eval(&rhs)?;
                    ops::binary(op.trim_end_matches('='), &current, &r).map_err(trap)?
                };
                self.set_object_field(obj, &field, new_value);
                return Ok(SwiftValue::Void);
            }
            // Subscript or struct member fall through to place-based handling.
        }

        // Subscript assignment `a[i] = v` over an array variable.
        if target.kind() == NodeKind::SubscriptExpr {
            return self.assign_subscript(&target, &rhs, &op);
        }

        // An unqualified static-property write inside a `static` method.
        if target.kind() == NodeKind::IdentExpr {
            if let Some(n) = target.text() {
                if self.env.get_local(&n).is_none() {
                    if let Some(key) = self.implicit_static_key(&n) {
                        let new_value = if op == "=" {
                            self.eval(&rhs)?
                        } else {
                            let current = self.statics[&key].clone();
                            let r = self.eval(&rhs)?;
                            ops::binary(op.trim_end_matches('='), &current, &r).map_err(trap)?
                        };
                        self.statics.insert(key, new_value);
                        return Ok(SwiftValue::Void);
                    }
                }
            }
        }

        // `self.<name>` where `self` is a class instance.
        if target.kind() == NodeKind::IdentExpr {
            if let Some(n) = target.text() {
                if self.env.get_local(&n).is_none() {
                    if let Some(SwiftValue::Object(obj)) = self.env.get("self") {
                        if self.class_has_member(&obj.borrow().class_name.clone(), &n) {
                            let new_value = if op == "=" {
                                self.eval(&rhs)?
                            } else {
                                let cur =
                                    self.read_object_member(&SwiftValue::Object(obj.clone()), &n)?;
                                let r = self.eval(&rhs)?;
                                ops::binary(op.trim_end_matches('='), &cur, &r).map_err(trap)?
                            };
                            self.set_object_field(&obj, &n, new_value);
                            return Ok(SwiftValue::Void);
                        }
                    }
                }
            }
        }

        // Resolve the target to an assignable place. A bare identifier that is
        // not a local binding but is a member of the current `self` becomes
        // `self.<name>`.
        let place = match self.resolve_place(&target) {
            Some(p) if p.path.is_empty() && self.env.get_local(&p.root).is_none() => {
                if self.self_has_member(&p.root) {
                    Place {
                        root: "self".into(),
                        path: vec![p.root],
                    }
                } else {
                    p
                }
            }
            Some(p) => p,
            None => {
                return Err(EvalError::Unsupported("unsupported assignment target".into()).into())
            }
        };

        let new_value = if op == "=" {
            self.eval(&rhs)?
        } else {
            let bin_op = op.trim_end_matches('=');
            let current = self.read_place(&place)?;
            let r = self.eval(&rhs)?;
            ops::binary(bin_op, &current, &r).map_err(trap)?
        };

        self.write_place(&place, new_value)?;
        Ok(SwiftValue::Void)
    }

    /// Read the current value stored at `place`.
    fn read_place(&mut self, place: &Place) -> Eval {
        let mut value = self
            .env
            .get(&place.root)
            .or_else(|| self.statics.get(&place.root).cloned())
            .ok_or_else(|| EvalError::UnknownVariable(place.root.clone()))?;
        for field in &place.path {
            value = self.read_struct_member(&value, field)?;
        }
        Ok(value)
    }

    /// Whether the current `self` (if any) has a stored/computed member `name`.
    fn self_has_member(&self, name: &str) -> bool {
        match self.env.get("self") {
            Some(SwiftValue::Struct(obj)) => {
                obj.get(name).is_some() || self.struct_has_member(&obj.type_name, name)
            }
            _ => false,
        }
    }

    /// Member access: static integer members (`Int.max`/`Int.min`) and
    /// `Array.count`.
    /// Evaluate a `MemoryLayout<T>.size` / `.stride` / `.alignment` access.
    /// Layouts are modelled on a 64-bit platform. Primitive scalar types and
    /// user structs (laid out field-by-field with C-style alignment/padding)
    /// are supported; other types report an unsupported-feature error.
    fn memory_layout_member(&self, ty: &str, member: &str) -> Eval {
        let (size, stride, alignment) = self
            .type_layout(ty)
            .ok_or_else(|| EvalError::Unsupported(format!("MemoryLayout<{ty}>")))?;
        let pick = match member {
            "size" => size,
            "stride" => stride,
            "alignment" => alignment,
            other => {
                return Err(EvalError::Unsupported(format!("MemoryLayout<{ty}>.{other}")).into())
            }
        };
        Ok(SwiftValue::Int(IntValue::new(pick as i128, IntWidth::I64)))
    }

    /// The `(size, stride, alignment)` of `ty` on a 64-bit platform, or `None`
    /// if the type's layout is not modelled.
    fn type_layout(&self, ty: &str) -> Option<(u64, u64, u64)> {
        self.type_layout_inner(ty, &mut Vec::new())
    }

    /// `type_layout`, tracking the chain of structs currently being laid out so
    /// a recursive value type (`struct A { var a: A }`) fails safely instead of
    /// overflowing the stack.
    fn type_layout_inner(&self, ty: &str, stack: &mut Vec<String>) -> Option<(u64, u64, u64)> {
        // Scalar primitives: `(size, alignment)`; stride == size for these.
        let scalar = |n: u64| Some((n, n, n));
        match ty.trim() {
            "Int" | "UInt" | "Int64" | "UInt64" | "Double" | "Float64" => scalar(8),
            "Int32" | "UInt32" | "Float" | "Float32" => scalar(4),
            "Int16" | "UInt16" => scalar(2),
            "Int8" | "UInt8" | "Bool" => scalar(1),
            // An empty type still occupies a stride of 1.
            "Void" | "()" => Some((0, 1, 1)),
            other => self.struct_layout(other, stack),
        }
    }

    /// Compute a user struct's layout by laying out its stored properties in
    /// declaration order with C-style alignment and tail padding. A nested
    /// struct field advances the running offset by the field's *size* (its tail
    /// padding is reusable), matching Swift's value-type layout. Returns `None`
    /// for unmodelled field types or a self-referential (cyclic) layout.
    fn struct_layout(&self, type_name: &str, stack: &mut Vec<String>) -> Option<(u64, u64, u64)> {
        let def = self.structs.get(type_name)?;
        // A struct that (transitively) contains itself has no finite layout.
        if stack.iter().any(|t| t == type_name) {
            return None;
        }
        stack.push(type_name.to_string());
        let mut offset: u64 = 0;
        let mut max_align: u64 = 1;
        for prop in &def.stored {
            let Some(field_ty) = prop.ty.as_deref() else {
                stack.pop();
                return None;
            };
            let Some((fsize, _fstride, falign)) = self.type_layout_inner(field_ty, stack) else {
                stack.pop();
                return None;
            };
            max_align = max_align.max(falign);
            // Round the running offset up to the field's alignment.
            offset = offset.div_ceil(falign) * falign;
            offset += fsize;
        }
        stack.pop();
        let size = offset;
        // Stride rounds the size up to the struct's overall alignment.
        let stride = size.div_ceil(max_align) * max_align;
        Some((size, stride, max_align))
    }

    fn eval_member(&mut self, node: &Node<'static>) -> Eval {
        let mut member = node
            .text()
            .ok_or_else(|| EvalError::Unsupported("member without a name".into()))?;

        // Shorthand `.case` (no base): construct the inferred enum case.
        let Some(base) = node.children().next() else {
            if member == "." {
                member = node.op_text().unwrap_or(member);
            }
            if let Some(tn) = self.resolve_member_enum(node, &member) {
                return Ok(self.make_enum_case(&tn, &member, Vec::new())?.unwrap());
            }
            // Implicit member of a static property: `.red` where the contextual
            // type declares `static let red`. Resolve via the node's inferred
            // type, else a unique static whose member name matches.
            if let Some(v) = self.resolve_implicit_static(node, &member) {
                return Ok(v);
            }
            return Err(EvalError::Unsupported(format!(".{member} (unresolved type)")).into());
        };

        if base.kind() == NodeKind::IdentExpr {
            if let Some(type_name) = base.text() {
                // `Self.member` resolves through the enclosing type. The keyword
                // is never a value binding, so it bypasses the env shadow check
                // below even if a local happens to share the resolved type name.
                let is_self_kw = type_name == "Self";
                let type_name = self.resolve_self_keyword(type_name);
                // A generic placeholder (`T.defaultValue`) resolves to its bound
                // concrete type for the current call.
                let type_name = self.resolve_type_alias(&type_name).unwrap_or(type_name);
                if is_self_kw || self.env.get(&type_name).is_none() {
                    // `MemoryLayout<T>.size` / `.stride` / `.alignment`. The
                    // written type `T` is recorded as a `TypeIdent` child of the
                    // `MemoryLayout` identifier by the parser.
                    if type_name == "MemoryLayout" {
                        if let Some(ty) = base
                            .children()
                            .find(|c| c.kind() == NodeKind::TypeRef)
                            .and_then(|c| c.text())
                        {
                            return self.memory_layout_member(&ty, &member);
                        }
                    }
                    // `Type.self` — a metatype value naming the type.
                    if member == "self" && self.is_type_name(&type_name) {
                        return Ok(SwiftValue::Metatype(type_name));
                    }
                    if let Some(w) = IntWidth::from_type_name(&type_name) {
                        return match member.as_str() {
                            "max" => Ok(SwiftValue::Int(IntValue::new(w.max(), w))),
                            "min" => Ok(SwiftValue::Int(IntValue::new(w.min(), w))),
                            _ => {
                                Err(EvalError::Unsupported(format!("{type_name}.{member}")).into())
                            }
                        };
                    }
                    // Static property of a struct or class type: `Type.prop`.
                    if self.structs.contains_key(&type_name)
                        || self.classes.contains_key(&type_name)
                    {
                        if let Some(v) = self.statics.get(&format!("{type_name}.{member}")) {
                            return Ok(v.clone());
                        }
                    }
                    // Static computed property: `static var prop { … }`.
                    if let Some(v) = self.read_static_computed(&type_name, &member)? {
                        return Ok(v);
                    }
                    // Enum case (no associated values) or `allCases`.
                    if self.enums.contains_key(&type_name) {
                        if member == "allCases" {
                            return self.enum_all_cases(&type_name);
                        }
                        if let Some(v) = self.make_enum_case(&type_name, &member, Vec::new())? {
                            return Ok(v);
                        }
                    }
                }
            }
        }

        let value = self.eval(&base)?;
        // Optional chaining: a nil base short-circuits the whole access to nil.
        if matches!(value, SwiftValue::Nil) {
            return Ok(SwiftValue::Nil);
        }
        // Task handle members: `.value`/`.result` keep the handle so the
        // enclosing `await` drives it; `.isCancelled` reads the flag (ADR-0005).
        if let SwiftValue::Task(tid) = &value {
            match member.as_str() {
                "value" | "result" => return Ok(value.clone()),
                "isCancelled" => return Ok(SwiftValue::Bool(self.tasks[*tid].cancelled)),
                _ => {}
            }
        }
        // Class instance members.
        if let SwiftValue::Object(_) = &value {
            return self.read_object_member(&value, &member);
        }
        // Enum members: rawValue and computed properties.
        if let SwiftValue::Enum(e) = &value {
            if member == "rawValue" {
                return self.enum_raw_value(&e.type_name, &e.case);
            }
            if let Some(v) = self.read_enum_computed(&value, &member)? {
                return Ok(v);
            }
        }
        if let SwiftValue::Struct(obj) = &value {
            // Lazy stored property: materialize on first access and cache it
            // back into the storage when the base is an lvalue.
            if obj.get(&member).is_none() {
                if let Some(def) = self.lazy_default(&obj.type_name, &member) {
                    let (computed, _) = self.run_with_self(value.clone(), |me| me.eval(&def))?;
                    if let Some(place) = self.resolve_place(&base) {
                        let cached =
                            self.set_struct_field(value.clone(), &member, computed.clone())?;
                        self.write_place(&place, cached)?;
                    }
                    return Ok(computed);
                }
            }
            if obj.get(&member).is_some() || self.struct_has_member(&obj.type_name, &member) {
                return self.read_struct_member(&value, &member);
            }
            if let Some(kind) = BuiltinReceiver::of(&value) {
                if let Some(func) = self.properties.get(&(kind, member.clone())).copied() {
                    return func(value).map_err(Self::std_error_to_signal);
                }
            }
            return self.read_struct_member(&value, &member);
        }
        // Standard-library computed-property intrinsics (`Double.isNaN`,
        // `Int.magnitude`, …) on builtin receivers.
        if let Some(kind) = BuiltinReceiver::of(&value) {
            if let Some(func) = self.properties.get(&(kind, member.clone())).copied() {
                return func(value).map_err(Self::std_error_to_signal);
            }
        }
        match (&value, member.as_str()) {
            // Array `count`/`isEmpty` are served by the property registry (S4).
            (SwiftValue::Str(s), "count") => Ok(SwiftValue::int(crate::graphemes(s).len() as i128)),
            (SwiftValue::Str(s), "isEmpty") => Ok(SwiftValue::Bool(s.is_empty())),
            (SwiftValue::Tuple(items, _), idx) if idx.parse::<usize>().is_ok() => {
                let i: usize = idx.parse().unwrap();
                items
                    .get(i)
                    .cloned()
                    .ok_or_else(|| EvalError::Type(format!("tuple index .{i} out of range")).into())
            }
            // Named tuple element access (`r.min` on `(min: 1, max: 9)`). This
            // also serves a dictionary element's `.key`/`.value`, since those
            // tuples carry the `key`/`value` labels (see `dict_element_tuple`).
            (SwiftValue::Tuple(items, labels), name)
                if SwiftValue::tuple_label_index(labels, name).is_some() =>
            {
                let i = SwiftValue::tuple_label_index(labels, name).unwrap();
                Ok(items[i].clone())
            }
            _ => {
                // User extension computed property on a builtin type
                // (`extension Int { var isEven: Bool { … } }`).
                let tn = value.type_name();
                if let Some(body) = self
                    .builtin_ext_computed
                    .get(&tn)
                    .and_then(|m| m.get(member.as_str()))
                    .and_then(|c| c.getter)
                {
                    return self
                        .run_with_self(value.clone(), |me| me.eval(&body))
                        .map(|(v, _)| v);
                }
                Err(EvalError::Unsupported(format!("member .{member} on {tn}")).into())
            }
        }
    }

    /// Evaluate a key-path literal `\Root.a.b` into a `KeyPath` value. The root
    /// type (a leading `TypeRef` child) is only needed at type-check time; the
    /// runtime keeps the ordered list of component names. `\.self` (and an
    /// embedded `.self`) is the identity path and contributes no component.
    fn eval_keypath(&mut self, node: &Node<'static>) -> Eval {
        let components: Vec<String> = node
            .children()
            .filter(|c| c.kind() == NodeKind::IdentExpr)
            .filter_map(|c| c.text())
            .filter(|n| n != "self")
            .collect();
        let id = self.closures.len();
        self.closures
            .push((ClosureDef::KeyPath(components), Vec::new()));
        Ok(SwiftValue::Closure(id))
    }

    /// The path components of a key-path closure value, if `value` is one.
    fn keypath_components(&self, value: &SwiftValue) -> Option<Vec<String>> {
        if let SwiftValue::Closure(id) = value {
            if let Some((ClosureDef::KeyPath(components), _)) = self.closures.get(*id) {
                return Some(components.clone());
            }
        }
        None
    }

    /// Read `root[keyPath: kp]` by walking each component in turn. A `nil`
    /// encountered mid-path short-circuits to `nil` (optional-chained access).
    fn apply_keypath(&mut self, root: SwiftValue, components: &[String]) -> Eval {
        let mut value = root;
        for name in components {
            if matches!(value, SwiftValue::Nil) {
                return Ok(SwiftValue::Nil);
            }
            value = self.read_named_member(value, name)?;
        }
        Ok(value)
    }

    /// Read the member named `name` from an already-evaluated `value`. Shared by
    /// key-path traversal; mirrors the value-dispatch tail of `eval_member`
    /// (struct/class/enum members, plus builtin `count`/`isEmpty` and labelled
    /// tuple elements).
    fn read_named_member(&mut self, value: SwiftValue, name: &str) -> Eval {
        if matches!(value, SwiftValue::Nil) {
            return Ok(SwiftValue::Nil);
        }
        match &value {
            SwiftValue::Object(_) => self.read_object_member(&value, name),
            SwiftValue::Struct(_) => self.read_struct_member(&value, name),
            SwiftValue::Enum(e) => {
                if name == "rawValue" {
                    return self.enum_raw_value(&e.type_name, &e.case);
                }
                if let Some(v) = self.read_enum_computed(&value, name)? {
                    return Ok(v);
                }
                Err(
                    EvalError::Unsupported(format!("key-path member .{name} on {}", e.type_name))
                        .into(),
                )
            }
            _ => {
                if let Some(kind) = BuiltinReceiver::of(&value) {
                    if let Some(func) = self.properties.get(&(kind, name.to_string())).copied() {
                        return func(value).map_err(Self::std_error_to_signal);
                    }
                }
                match (&value, name) {
                    (SwiftValue::Str(s), "count") => {
                        Ok(SwiftValue::int(crate::graphemes(s).len() as i128))
                    }
                    (SwiftValue::Str(s), "isEmpty") => Ok(SwiftValue::Bool(s.is_empty())),
                    (SwiftValue::Tuple(items, labels), n)
                        if SwiftValue::tuple_label_index(labels, n).is_some() =>
                    {
                        let i = SwiftValue::tuple_label_index(labels, n).unwrap();
                        Ok(items[i].clone())
                    }
                    _ => Err(EvalError::Unsupported(format!(
                        "key-path member .{name} on {}",
                        value.type_name()
                    ))
                    .into()),
                }
            }
        }
    }

    /// Write `container[keyPath: kp] = value`, returning the updated container
    /// (value types are rebuilt copy-on-write; class instances mutate in place
    /// and are returned unchanged). An empty path is the identity path, so the
    /// whole value is replaced.
    fn set_keypath(
        &mut self,
        container: SwiftValue,
        components: &[String],
        value: SwiftValue,
    ) -> Eval {
        match components {
            [] => Ok(value),
            [name] => self.set_named_member(container, name, value),
            [name, rest @ ..] => {
                let child = self.read_named_member(container.clone(), name)?;
                let new_child = self.set_keypath(child, rest, value)?;
                self.set_named_member(container, name, new_child)
            }
        }
    }

    /// Set the member `name` on `container` to `value`. Structs are rebuilt via
    /// `set_struct_field` (copy-on-write); class instances are mutated through
    /// their shared storage.
    fn set_named_member(&mut self, container: SwiftValue, name: &str, value: SwiftValue) -> Eval {
        match &container {
            SwiftValue::Struct(_) => self.set_struct_field(container.clone(), name, value),
            SwiftValue::Object(obj) => {
                self.set_object_field(obj, name, value);
                Ok(container)
            }
            other => Err(EvalError::Type(format!(
                "cannot set key-path member .{name} on {}",
                other.type_name()
            ))
            .into()),
        }
    }

    /// Evaluate a call: a method, a struct initializer, a user function, a
    /// native, or a conversion initializer.
    fn eval_call(&mut self, node: &Node<'static>) -> Eval {
        let children: Vec<Node<'static>> = node.children().collect();
        let callee = children
            .first()
            .ok_or_else(|| EvalError::Unsupported("call with no callee".into()))?;
        let arg_nodes = &children[1..];

        // Method call: `base.method(args)`.
        if callee.kind() == NodeKind::MemberExpr {
            return self.eval_method_call(callee, arg_nodes);
        }

        // Structured-concurrency entry points (ADR-0005). Handled before
        // argument evaluation so a metatype label like `of: Int.self` is never
        // eagerly evaluated; only the trailing body closure matters.
        if callee.kind() == NodeKind::IdentExpr {
            if let Some(name) = callee.text() {
                if let Some(v) = self.try_concurrency_builtin(&name, arg_nodes)? {
                    return Ok(v);
                }
            }
        }

        // If the callee is a known user function with `@autoclosure` params,
        // defer those argument expressions into thunks (capturing this scope).
        let autoclosure_params = if callee.kind() == NodeKind::IdentExpr {
            callee.text().and_then(|name| match self.env.get(&name) {
                Some(SwiftValue::Function(id)) => Some(clone_params(&self.funcs[id].params)),
                _ => None,
            })
        } else {
            None
        };
        let args = self.eval_args_with(arg_nodes, autoclosure_params.as_deref())?;

        if callee.kind() == NodeKind::IdentExpr {
            let name = callee
                .text()
                .ok_or_else(|| EvalError::Unsupported("unnamed callee".into()))?;
            // `Self(...)` constructs an instance of the enclosing type.
            let name = self.resolve_self_keyword(name);

            // `type(of: x)` — the dynamic type of `x` as a metatype value.
            if name == "type" && self.env.get("type").is_none() {
                if let Some(arg) = args
                    .iter()
                    .find(|a| a.label.as_deref() == Some("of"))
                    .or_else(|| args.first())
                {
                    return Ok(SwiftValue::Metatype(arg.value.type_name()));
                }
            }
            // Built-in JSON coder markers.
            if name == "JSONEncoder" || name == "JSONDecoder" {
                return Ok(SwiftValue::Struct(Rc::new(StructObj {
                    type_name: name,
                    fields: vec![],
                })));
            }
            // `EnumType(rawValue:)` — failable lookup of the case with that raw
            // value, returning the case or `nil` (RawRepresentable synthesis).
            if self.enums.contains_key(&name) {
                if let Some(raw) = args
                    .iter()
                    .find(|a| a.label.as_deref() == Some("rawValue"))
                    .map(|a| a.value.clone())
                {
                    let case = self.enums[&name]
                        .cases
                        .iter()
                        .find(|c| c.raw.as_ref() == Some(&raw))
                        .map(|c| c.name.clone());
                    return Ok(match case {
                        Some(name_) => SwiftValue::Enum(Rc::new(EnumObj {
                            type_name: name.clone(),
                            case: name_,
                            payload: Vec::new(),
                        })),
                        None => SwiftValue::Nil,
                    });
                }
            }
            // Class initializer.
            if self.classes.contains_key(&name) {
                return self.instantiate_class(&name, args);
            }
            // Struct memberwise initializer.
            if self.structs.contains_key(&name) {
                let simple: Vec<(Option<String>, SwiftValue)> = args
                    .iter()
                    .map(|a| (a.label.clone(), a.value.clone()))
                    .collect();
                return self.instantiate_struct(&name, &simple);
            }
            // `@dynamicCallable`: calling a struct instance routes through its
            // `dynamicallyCall(...)` method.
            if let Some(value @ SwiftValue::Struct(_)) = self.env.get(&name) {
                if self.is_dynamic_callable(&value) {
                    return self.dynamic_call(value, args);
                }
            }
            // A bound function or closure value (incl. recursion).
            match self.env.get(&name) {
                Some(SwiftValue::Function(id)) => return self.call_function(id, args),
                Some(SwiftValue::Closure(id)) => {
                    return self.call_closure_with_args(id, args);
                }
                _ => {}
            }
            // `swap(&a, &b)` — exchange two inout locations. Needs the caller
            // write-back `Place`s, so it cannot ride the value-only free-fn seam.
            if name == "swap" && self.env.get("swap").is_none() && args.len() == 2 {
                if let (Some(pa), Some(pb)) = (args[0].place.clone(), args[1].place.clone()) {
                    let va = args[0].value.clone();
                    let vb = args[1].value.clone();
                    self.write_place(&pa, vb)?;
                    self.write_place(&pb, va)?;
                    return Ok(SwiftValue::Void);
                }
            }
            // `isKnownUniquelyReferenced(&obj)` — true when the class instance is
            // not shared. The env binding plus this evaluated clone account for
            // two strong references, so a unique object reads as exactly two.
            if name == "isKnownUniquelyReferenced"
                && self.env.get("isKnownUniquelyReferenced").is_none()
                && args.len() == 1
            {
                return Ok(match &args[0].value {
                    SwiftValue::Object(rc) => SwiftValue::Bool(Rc::strong_count(rc) == 2),
                    _ => SwiftValue::Bool(false),
                });
            }

            // `Array(repeating:count:)` — build an array of repeated elements.
            if name == "Array" && self.env.get("Array").is_none() {
                let repeating = args
                    .iter()
                    .find(|a| a.label.as_deref() == Some("repeating"))
                    .map(|a| a.value.clone());
                let count = args
                    .iter()
                    .find(|a| a.label.as_deref() == Some("count"))
                    .and_then(|a| match &a.value {
                        SwiftValue::Int(i) if i.raw >= 0 => Some(i.raw as usize),
                        _ => None,
                    });
                if let (Some(elem), Some(n)) = (repeating, count) {
                    return Ok(SwiftValue::Array(Rc::new(vec![elem; n])));
                }
            }

            // `Dictionary(uniqueKeysWithValues:)` and `Dictionary(grouping:by:)`.
            if name == "Dictionary" && self.env.get("Dictionary").is_none() {
                if let Some(v) = self.build_dictionary(&args)? {
                    return Ok(v);
                }
            }

            // Conversion initializers take exactly one argument.
            if args.len() == 1 {
                if let Some(v) = self.try_conversion(&name, &args[0].value)? {
                    return Ok(v);
                }
            }
            // Free-function intrinsic served through the StdContext seam.
            if let Some(free) = self.free_fns.get(&name).copied() {
                let labeled: Vec<Arg> = args
                    .into_iter()
                    .map(|a| Arg {
                        label: a.label,
                        value: a.value,
                    })
                    .collect();
                return free(self, labeled).map_err(Self::std_error_to_signal);
            }
            if let Some(native) = self.natives.get(&name).copied() {
                let plain: Vec<SwiftValue> = args.into_iter().map(|a| a.value).collect();
                return Ok(native(self.out, &plain));
            }
            // An unqualified call inside a method resolves to `self.<name>()`.
            if let Some(this) = self.env.get("self") {
                match &this {
                    SwiftValue::Object(obj) => {
                        let cls = obj.borrow().class_name.clone();
                        if self.lookup_method(&cls, &name).is_some()
                            || self.protocol_default_method(&cls, &name).is_some()
                        {
                            return self.dispatch_class_method(this, &cls, &name, args);
                        }
                    }
                    SwiftValue::Struct(o) => {
                        let tn = o.type_name.clone();
                        if self.type_has_method(&tn, &name) {
                            let place = Place {
                                root: "self".into(),
                                path: vec![],
                            };
                            return self.call_struct_method(this, &tn, &name, args, Some(place));
                        }
                    }
                    SwiftValue::Enum(e) => {
                        let tn = e.type_name.clone();
                        if self.type_has_method(&tn, &name) {
                            return self.call_struct_method(this, &tn, &name, args, None);
                        }
                    }
                    _ => {}
                }
            }
            return Err(EvalError::UnknownFunction(name).into());
        }

        // Callee is an arbitrary expression — must evaluate to a callable value.
        let value = self.eval(callee)?;
        match value {
            SwiftValue::Function(id) => self.call_function(id, args),
            SwiftValue::Closure(id) => self.call_closure_with_args(id, args),
            other => {
                Err(EvalError::Type(format!("`{}` is not callable", other.type_name())).into())
            }
        }
    }

    /// Evaluate call arguments, resolving `inout` (`&place`) into a write-back
    /// location.
    fn eval_args(&mut self, arg_nodes: &[Node<'static>]) -> Result<Vec<CallArg>, Signal> {
        self.eval_args_with(arg_nodes, None)
    }

    /// Evaluate call arguments, deferring any that map to an `@autoclosure`
    /// parameter into a zero-argument thunk closure (capturing the caller's
    /// scope) instead of evaluating them eagerly.
    fn eval_args_with(
        &mut self,
        arg_nodes: &[Node<'static>],
        params: Option<&[Param]>,
    ) -> Result<Vec<CallArg>, Signal> {
        let autoclosure = params.map(|p| autoclosure_flags(p, arg_nodes));
        // Expected parameter type per argument, so an implicit-member argument
        // can resolve against the call-site contextual type.
        let hints = params.map(|p| param_type_hints(p, arg_nodes));
        let mut args = Vec::new();
        for (i, arg) in arg_nodes.iter().enumerate() {
            let label = arg.arg_label();
            if autoclosure.as_ref().is_some_and(|f| f[i]) {
                // `@autoclosure`: wrap the unevaluated expression in a thunk.
                let captured = self.env.capture();
                let id = self.closures.len();
                self.closures.push((
                    ClosureDef::User {
                        params: Vec::new(),
                        body: vec![*arg],
                    },
                    captured,
                ));
                args.push(CallArg {
                    label,
                    value: SwiftValue::Closure(id),
                    place: None,
                });
                continue;
            }
            let hint = hints.as_ref().and_then(|h| h[i].clone());
            if arg.kind() == NodeKind::InoutExpr {
                let inner = arg
                    .children()
                    .next()
                    .ok_or_else(|| EvalError::Unsupported("inout without an lvalue".into()))?;
                let place = self.resolve_place(&inner);
                self.type_hint.push(hint.clone());
                let value = self.eval(&inner);
                self.type_hint.pop();
                let value = value?;
                args.push(CallArg {
                    label,
                    value,
                    place,
                });
            } else {
                self.type_hint.push(hint.clone());
                let value = self.eval(arg);
                self.type_hint.pop();
                let mut value = value?;
                if let (Some(ty), Some(kind)) = (hint.as_deref(), literal_syntax_kind(arg)) {
                    let optional = ty.trim().ends_with('?');
                    if !(optional && kind == NodeKind::NilLiteral) {
                        value = self.coerce_literal_value(
                            ty.trim().trim_end_matches('?').trim(),
                            kind,
                            value,
                        )?;
                    }
                }
                args.push(CallArg {
                    label,
                    value,
                    place: None,
                });
            }
        }
        Ok(args)
    }

    /// `base.method(args)`. Binds `self`; for `mutating` methods, writes the
    /// updated `self` back to `base`'s storage.
    fn eval_method_call(&mut self, member: &Node<'static>, arg_nodes: &[Node<'static>]) -> Eval {
        let mut method = member
            .text()
            .ok_or_else(|| EvalError::Unsupported("method without a name".into()))?;
        // A bare `.` member spells its name in the operator slot.
        if method == "." {
            method = member.op_text().unwrap_or(method);
        }

        // Shorthand `.case(args)`: resolve the enum type from msf's inference.
        let Some(base) = member.children().next() else {
            if let Some(tn) = self.resolve_member_enum(member, &method) {
                let args = self.eval_args(arg_nodes)?;
                let payload = args.into_iter().map(|a| a.value).collect();
                return Ok(self.make_enum_case(&tn, &method, payload)?.unwrap());
            }
            // Implicit member static method: `.custom(x)` where the contextual
            // type declares `static func custom`.
            if let Some(tn) = self.resolve_implicit_static_method(member, &method) {
                let params = self.user_method_params(&tn, &method);
                let args = self.eval_args_with(arg_nodes, params.as_deref())?;
                if self.classes.contains_key(&tn) {
                    return self.dispatch_class_method(SwiftValue::Void, &tn, &method, args);
                }
                return self.call_struct_method(SwiftValue::Void, &tn, &method, args, None);
            }
            return Err(EvalError::Unsupported(format!(".{method}() (unresolved type)")).into());
        };

        // `super.method(args)`: dispatch to the superclass implementation.
        if base.kind() == NodeKind::IdentExpr && base.text().as_deref() == Some("super") {
            let this = self
                .env
                .get("self")
                .ok_or_else(|| EvalError::Unsupported("`super` outside a method".into()))?;
            let start = self
                .class_ctx
                .last()
                .and_then(|c| self.classes.get(c))
                .and_then(|d| d.superclass.clone())
                .ok_or_else(|| EvalError::Unsupported("`super` without a superclass".into()))?;
            let args = self.eval_args(arg_nodes)?;
            if method == "init" {
                self.run_class_init(this, &start, args)?;
                return Ok(SwiftValue::Void);
            }
            return self.dispatch_class_method(this, &start, &method, args);
        }

        // `Task.detached { }` / `Task.yield()` / `Task.sleep(...)` (ADR-0005).
        if base.kind() == NodeKind::IdentExpr
            && base.text().as_deref() == Some("Task")
            && self.env.get("Task").is_none()
        {
            let args = self.eval_args(arg_nodes)?;
            match method.as_str() {
                "detached" => {
                    let closure = Self::first_closure(&args).ok_or_else(|| {
                        EvalError::Unsupported("Task.detached without a body closure".into())
                    })?;
                    return Ok(SwiftValue::Task(self.spawn_task_closure(closure)));
                }
                // Cooperative no-ops on our single-threaded executor.
                "yield" | "sleep" | "checkCancellation" => return Ok(SwiftValue::Void),
                _ => {}
            }
        }

        // `Type.<...>(args)`: enum case construction or a static struct method.
        if base.kind() == NodeKind::IdentExpr {
            if let Some(tn) = base.text() {
                // `Self.method(...)` calls a static method of the enclosing type.
                // The keyword is never a value binding, so it bypasses the env
                // shadow check below.
                let is_self_kw = tn == "Self";
                let tn = self.resolve_self_keyword(tn);
                // A generic placeholder (`T.zero()`) resolves to its bound type.
                let tn = self.resolve_type_alias(&tn).unwrap_or(tn);
                if is_self_kw || self.env.get(&tn).is_none() {
                    // Builtin static methods, e.g. `Bool.random()`. A user type
                    // shadowing a builtin name (`struct Bool { … }`) wins, so
                    // only fall back to the builtin when no user type matches.
                    let user_defined = self.structs.contains_key(&tn)
                        || self.enums.contains_key(&tn)
                        || self.classes.contains_key(&tn);
                    if !user_defined {
                        if let Some(recv) = BuiltinReceiver::from_type_name(&tn) {
                            if let Some(func) =
                                self.static_methods.get(&(recv, method.clone())).copied()
                            {
                                let labeled: Vec<Arg> = self
                                    .eval_args(arg_nodes)?
                                    .into_iter()
                                    .map(|a| Arg {
                                        label: a.label,
                                        value: a.value,
                                    })
                                    .collect();
                                return func(self, labeled).map_err(Self::std_error_to_signal);
                            }
                        }
                    }
                    // `Outer.Nested(args)`: construct a nested type referenced
                    // through its enclosing type. Nested types are registered by
                    // their simple name, so resolve `method` against the type
                    // tables when `tn` is itself a user type.
                    let tn_is_type = self.structs.contains_key(&tn)
                        || self.classes.contains_key(&tn)
                        || self.enums.contains_key(&tn);
                    if tn_is_type {
                        if self.classes.contains_key(&method) {
                            let args = self.eval_args(arg_nodes)?;
                            return self.instantiate_class(&method, args);
                        }
                        if self.structs.contains_key(&method) {
                            let simple: Vec<(Option<String>, SwiftValue)> = self
                                .eval_args(arg_nodes)?
                                .iter()
                                .map(|a| (a.label.clone(), a.value.clone()))
                                .collect();
                            return self.instantiate_struct(&method, &simple);
                        }
                    }
                    if self.enum_has_case(&tn, &method) {
                        let args = self.eval_args(arg_nodes)?;
                        let payload = args.into_iter().map(|a| a.value).collect();
                        return Ok(self.make_enum_case(&tn, &method, payload)?.unwrap());
                    }
                    if self.structs.contains_key(&tn) {
                        let params = self.user_method_params(&tn, &method);
                        let args = self.eval_args_with(arg_nodes, params.as_deref())?;
                        return self.call_struct_method(SwiftValue::Void, &tn, &method, args, None);
                    }
                    // `Type.method(...)` — a static method on a class.
                    if self.classes.contains_key(&tn) && self.lookup_method(&tn, &method).is_some()
                    {
                        let params = self.user_method_params(&tn, &method);
                        let args = self.eval_args_with(arg_nodes, params.as_deref())?;
                        return self.dispatch_class_method(SwiftValue::Void, &tn, &method, args);
                    }
                }
            }
        }

        let base_value = self.eval(&base)?;

        // `group.addTask { }` / `group.cancelAll()` and `task.cancel()`.
        if let Some(result) = self.try_concurrency_method(&base_value, &method, arg_nodes)? {
            return Ok(result);
        }

        // `JSONEncoder().encode(value)` → a JSON `Data` (modeled as a String).
        if let SwiftValue::Struct(o) = &base_value {
            if o.type_name == "JSONEncoder" && method == "encode" {
                let args = self.eval_args(arg_nodes)?;
                let value = args
                    .first()
                    .map(|a| a.value.clone())
                    .ok_or_else(|| EvalError::Type("encode expects a value".into()))?;
                let json = self.json_encode(&value)?;
                return Ok(SwiftValue::Str(crate::json::to_string(&json)));
            }
            // `JSONDecoder().decode(T.self, from: data)` → a value of type `T`.
            if o.type_name == "JSONDecoder" && method == "decode" {
                let type_name = arg_nodes
                    .first()
                    .and_then(metatype_name)
                    .ok_or_else(|| EvalError::Type("decode expects a metatype".into()))?;
                let data = arg_nodes
                    .get(1)
                    .map(|n| self.eval(n))
                    .transpose()?
                    .ok_or_else(|| EvalError::Type("decode expects data".into()))?;
                let text = match data {
                    SwiftValue::Str(s) => s,
                    other => {
                        return Err(EvalError::Type(format!(
                            "decode expects String/Data, got {}",
                            other.type_name()
                        ))
                        .into())
                    }
                };
                let json = crate::json::parse(&text)
                    .map_err(|e| Signal::Throw(SwiftValue::Str(format!("decode error: {e}"))))?;
                return Ok(self.json_decode(&type_name, &json));
            }
        }

        // Class instance: dynamic dispatch from the runtime class.
        if let SwiftValue::Object(obj) = &base_value {
            let class_name = obj.borrow().class_name.clone();
            let params = self.user_method_params(&class_name, &method);
            let args = self.eval_args_with(arg_nodes, params.as_deref())?;
            return self.dispatch_class_method(base_value.clone(), &class_name, &method, args);
        }

        // User extension method on a builtin type (`extension Int { … }`).
        // User declarations are consulted before the stdlib seam so a program's
        // extension can shadow an otherwise-available intrinsic/algorithm.
        let builtin_name = base_value.type_name();
        if self
            .builtin_ext_methods
            .get(&builtin_name)
            .is_some_and(|m| m.contains_key(&method))
        {
            let params = self
                .builtin_ext_methods
                .get(&builtin_name)
                .and_then(|m| m.get(&method))
                .map(|def| clone_params(&def.params));
            let args = self.eval_args_with(arg_nodes, params.as_deref())?;
            let place = self.resolve_place(&base);
            if let Some(result) = self.call_builtin_ext_method(
                base_value.clone(),
                &builtin_name,
                &method,
                args,
                place,
            ) {
                return result;
            }
        }

        // Standard-library intrinsic registry (layer 1): type-specific members
        // such as `Array.append`. Consulted before the ad-hoc algorithm paths.
        if let Some(kind) = BuiltinReceiver::of(&base_value) {
            if self.intrinsics.contains_key(&(kind, method.clone())) {
                let args = self.eval_args(arg_nodes)?;
                // `IndexPath`/`IndexSet` intrinsics take positional arguments.
                // The sole exception is `IndexSet.update(with:)`, whose one
                // argument is labelled `with:` (and requires that label).
                if matches!(kind, BuiltinReceiver::IndexPath | BuiltinReceiver::IndexSet) {
                    let is_update = kind == BuiltinReceiver::IndexSet && method == "update";
                    let labels_valid = args.iter().all(|arg| match arg.label.as_deref() {
                        Some("with") => is_update,
                        Some(_) => false,
                        None => !is_update,
                    });
                    if !labels_valid {
                        return Err(EvalError::Type(format!(
                            "{}.{} called with unexpected argument label(s)",
                            kind.type_name(),
                            method
                        ))
                        .into());
                    }
                }
                let plain: Vec<SwiftValue> = args.into_iter().map(|a| a.value).collect();
                let place = self.resolve_place(&base);
                if let Some(result) = self.dispatch_intrinsic(base_value, &method, plain, place) {
                    return result;
                }
                unreachable!("intrinsic presence checked above");
            }
        }

        // Standard-library algorithm layer (layer 2): `Sequence`/`Collection`
        // methods (`map`/`filter`/`sorted`/…) over any builtin sequence.
        if self.algorithms.contains_key(&method) {
            let items = if let Some(items) = materialize_sequence(&base_value) {
                Some(items)
            } else if self.is_custom_sequence(&base_value) {
                Some(self.materialize_custom_sequence(base_value.clone())?)
            } else {
                None
            };
            if let Some(items) = items {
                let func = self.algorithms[&method];
                let labeled: Vec<Arg> = self
                    .eval_args(arg_nodes)?
                    .into_iter()
                    .map(|a| Arg {
                        label: a.label,
                        value: a.value,
                    })
                    .collect();
                return func(self, items, labeled).map_err(Self::std_error_to_signal);
            }
        }

        // `Result.get()`: unwrap success, or throw the failure error.
        if let SwiftValue::Enum(e) = &base_value {
            if e.type_name == "Result" && method == "get" {
                return match e.case.as_str() {
                    "success" => Ok(e.payload.first().cloned().unwrap_or(SwiftValue::Void)),
                    _ => Err(Signal::Throw(
                        e.payload.first().cloned().unwrap_or(SwiftValue::Nil),
                    )),
                };
            }
        }

        let type_name = match &base_value {
            SwiftValue::Struct(o) => Some(o.type_name.clone()),
            SwiftValue::Enum(e) => Some(e.type_name.clone()),
            _ => None,
        };
        let method_params = type_name
            .as_ref()
            .and_then(|tn| self.user_method_params(tn, &method));
        let args = self.eval_args_with(arg_nodes, method_params.as_deref())?;
        if let Some(type_name) = type_name {
            if self.type_has_method(&type_name, &method) {
                let place = self.resolve_place(&base);
                return self.call_struct_method(base_value, &type_name, &method, args, place);
            }
        }

        // Generic struct-method fallback (SwiftUI view modifiers): dispatched on
        // any struct receiver by name, after user methods and builtin receivers.
        if matches!(base_value, SwiftValue::Struct(_)) {
            if let Some(func) = self.struct_methods.get(&method).copied() {
                let labeled: Vec<Arg> = args
                    .into_iter()
                    .map(|a| Arg {
                        label: a.label,
                        value: a.value,
                    })
                    .collect();
                return func(self, base_value, labeled).map_err(Self::std_error_to_signal);
            }
        }

        let builtin_name = base_value.type_name();
        Err(EvalError::Unsupported(format!("method .{method}() on {builtin_name}")).into())
    }

    /// Run the initializer declared at or above `start_class` for `this`.
    fn run_class_init(
        &mut self,
        this: SwiftValue,
        start_class: &str,
        args: Vec<CallArg>,
    ) -> Result<(), Signal> {
        let mut chain = self.class_chain(start_class);
        chain.reverse(); // most-derived (start) first
        let owner = chain.into_iter().find(|c| self.classes[c].init.is_some());
        let Some(owner) = owner else {
            return Ok(()); // no explicit init to run
        };
        let (params, body) = {
            let m = self.classes[&owner].init.as_ref().unwrap();
            (clone_params(&m.params), m.body)
        };
        self.class_ctx.push(owner);
        let saved_env = self.env.enter_isolated();
        self.env.declare("self", this, false);
        let bound = self.bind_params(&params, args);
        let result = match bound {
            Ok(_) => match body {
                Some(b) => self.eval(&b),
                None => Ok(SwiftValue::Void),
            },
            Err(e) => Err(e),
        };
        self.env.restore(saved_env);
        self.class_ctx.pop();
        match result {
            // A failing `super.init?` (`return nil`) must propagate so the
            // calling subclass initializer also fails, rather than producing a
            // half-built instance.
            Err(Signal::Return(SwiftValue::Nil)) => Err(Signal::Return(SwiftValue::Nil)),
            Ok(_) | Err(Signal::Return(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Dispatch a class method dynamically (override-aware), binding `self`.
    fn dispatch_class_method(
        &mut self,
        this: SwiftValue,
        from_class: &str,
        method: &str,
        args: Vec<CallArg>,
    ) -> Eval {
        let (params, body, owner, generics) = match self.lookup_method(from_class, method) {
            Some(m) => m,
            None => {
                let (p, b, _, g) = self
                    .protocol_default_method(from_class, method)
                    .ok_or_else(|| {
                        EvalError::Unsupported(format!("{from_class} has no method `{method}`"))
                    })?;
                (p, b, from_class.to_string(), g)
            }
        };
        // A type-level (`static`/`class`) method has no instance `self`.
        let is_static_call = matches!(this, SwiftValue::Void);
        if is_static_call {
            self.static_ctx.push(from_class.to_string());
        }
        self.class_ctx.push(owner);
        let type_binding = self.infer_type_bindings(&generics, &params, &args);
        self.type_bindings.push(type_binding);
        // Isolate from caller locals (a class `self` is a reference, so field
        // mutations persist through the object regardless of the env).
        let saved_env = self.env.enter_isolated();
        self.env.declare("self", this, false);
        let bound = self.bind_params(&params, args);
        let result = match bound {
            Ok(_) => match body {
                Some(b) => self.eval(&b),
                None => Ok(SwiftValue::Void),
            },
            Err(e) => Err(e),
        };
        self.env.restore(saved_env);
        self.type_bindings.pop();
        self.class_ctx.pop();
        if is_static_call {
            self.static_ctx.pop();
        }
        match result {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
    }

    /// Select a struct method overload by the call's argument labels. Returns
    /// `None` unless the type declares more than one method of that name and
    /// exactly one of them matches the labels — keeping single-method dispatch
    /// and unresolved (type-only) overloads on the existing path.
    fn select_struct_overload(
        &self,
        type_name: &str,
        method: &str,
        args: &[CallArg],
    ) -> Option<(Vec<Param>, Option<Node<'static>>, bool, Vec<String>)> {
        let overloads = self.structs.get(type_name)?.method_overloads.get(method)?;
        if overloads.len() < 2 {
            return None;
        }
        let chosen = select_labeled_overload(overloads, args)?;
        Some((
            clone_params(&chosen.params),
            chosen.body,
            chosen.mutating,
            chosen.generic_params.clone(),
        ))
    }

    /// The declared parameters of a user method on `type_name`, across class,
    /// struct, and enum types (used to spot `@autoclosure` params before the
    /// arguments are evaluated).
    fn user_method_params(&self, type_name: &str, method: &str) -> Option<Vec<Param>> {
        if let Some((params, _, _, _)) = self.lookup_method(type_name, method) {
            return Some(params);
        }
        if let Some(d) = self.structs.get(type_name) {
            if let Some(m) = d.methods.get(method) {
                return Some(clone_params(&m.params));
            }
        }
        if let Some(d) = self.enums.get(type_name) {
            if let Some(m) = d.methods.get(method) {
                return Some(clone_params(&m.params));
            }
        }
        None
    }

    /// Whether a struct or enum type declares a method `method`.
    fn type_has_method(&self, type_name: &str, method: &str) -> bool {
        self.structs
            .get(type_name)
            .is_some_and(|d| d.methods.contains_key(method))
            || self
                .enums
                .get(type_name)
                .is_some_and(|d| d.methods.contains_key(method))
            || self.protocol_default_method(type_name, method).is_some()
    }

    /// Invoke a struct method with `self` bound and parameters applied.
    fn call_struct_method(
        &mut self,
        this: SwiftValue,
        type_name: &str,
        method: &str,
        args: Vec<CallArg>,
        base_place: Option<Place>,
    ) -> Eval {
        // Prefer a label-selected overload (`buildEither(first:)` vs
        // `(second:)`); fall back to the single stored method otherwise.
        let own = self
            .select_struct_overload(type_name, method, &args)
            .or_else(|| {
                self.structs
                    .get(type_name)
                    .and_then(|d| d.methods.get(method))
                    .or_else(|| {
                        self.enums
                            .get(type_name)
                            .and_then(|d| d.methods.get(method))
                    })
                    .map(|def| {
                        (
                            clone_params(&def.params),
                            def.body,
                            def.mutating,
                            def.generic_params.clone(),
                        )
                    })
            });
        let (params, body, mutating, generics) = match own {
            Some(m) => m,
            None => self
                .protocol_default_method(type_name, method)
                .ok_or_else(|| {
                    EvalError::Unsupported(format!("{type_name} has no method `{method}`"))
                })?,
        };

        // A `static`/type method has no instance `self`; record the type so an
        // unqualified static-property reference inside it resolves.
        let is_static_call = matches!(this, SwiftValue::Void);
        if is_static_call {
            self.static_ctx.push(type_name.to_string());
        }
        let type_binding = self.infer_type_bindings(&generics, &params, &args);
        self.type_bindings.push(type_binding);
        // For a `mutating` struct method with an lvalue receiver, take the
        // receiver out of its storage so `self` becomes its sole owner *aside
        // from other logical bindings* (the `var y = x` aliases). `make_mut`
        // then clones the `StructObj` — retaining its reference-type fields —
        // exactly when the value is shared, so a class-backed CoW buffer reads
        // the right answer from `isKnownUniquelyReferenced`. A unique value
        // keeps strong count 1 and is mutated in place. The end-of-call
        // write-back restores the storage we vacated here.
        let this = if mutating && matches!(this, SwiftValue::Struct(_)) {
            // Only a *root* stored binding is vacated: vacating a nested member
            // (`outer.buffer.append(...)`) would route the placeholder write
            // through `willSet`/`didSet`/computed setters/property wrappers,
            // which must not observe the transient. Nested receivers keep the
            // pre-existing clone-and-write-back behaviour.
            match &base_place {
                Some(place) if place.path.is_empty() => {
                    drop(this);
                    let mut taken = self.read_place(place)?;
                    self.write_place(place, SwiftValue::Void)?;
                    if let SwiftValue::Struct(rc) = &mut taken {
                        let _ = Rc::make_mut(rc);
                    }
                    taken
                }
                _ => this,
            }
        } else {
            this
        };
        // Run isolated from the caller's locals: the body sees globals, its
        // parameters, and `self`/its members, but not enclosing variables.
        let saved_env = self.env.enter_isolated();
        self.env.declare("self", this, true);
        let (outcome, inout_finals) = match self.bind_params(&params, args) {
            Ok(binds) => {
                let result = match body {
                    Some(b) => self.eval(&b),
                    None => Ok(SwiftValue::Void),
                };
                // Capture `inout` write-backs against the method env before it
                // is torn down; apply them to the caller below.
                let finals: Vec<(Place, SwiftValue)> = binds
                    .iter()
                    .filter_map(|(name, place)| self.env.get(name).map(|v| (place.clone(), v)))
                    .collect();
                (result, finals)
            }
            Err(e) => (Err(e), Vec::new()),
        };
        let updated_self = self.env.get("self").unwrap_or(SwiftValue::Void);
        self.env.restore(saved_env);
        self.type_bindings.pop();
        if is_static_call {
            self.static_ctx.pop();
        }

        // Write `inout` parameters and the mutated receiver back to the caller,
        // including on a thrown error (Swift copies them out on a caught
        // error); only a fatal interpreter trap skips the copy-out.
        if !matches!(outcome, Err(Signal::Error(_))) {
            for (place, v) in inout_finals {
                self.write_place(&place, v)?;
            }
            if mutating {
                if let Some(place) = base_place {
                    self.write_place(&place, updated_self)?;
                }
            }
        }
        match outcome {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
    }

    /// A string literal, processing escapes and `\( … )` interpolation.
    /// Evaluate a `/.../` or `#/.../#` regex literal into a compiled
    /// [`SwiftValue::Regex`]. The pattern between the delimiters is compiled
    /// once here; an invalid pattern traps (Swift would reject it at compile
    /// time, which this runtime surfaces as a runtime error).
    fn eval_regex_literal(&mut self, node: &Node<'static>) -> Eval {
        let raw = node.text().unwrap_or_default();
        let pattern = strip_regex_delimiters(&raw);
        match crate::regex::Regex::compile(pattern) {
            Ok(re) => Ok(SwiftValue::Regex(std::rc::Rc::new(re))),
            Err(msg) => Err(trap(format!("invalid regular expression: {msg}"))),
        }
    }

    fn eval_string_literal(&mut self, node: &Node<'static>) -> Eval {
        let raw = node.text().unwrap_or_default();
        // Raw strings do not interpolate; decode handles delimiters/escapes.
        if raw.starts_with('#') {
            return Ok(SwiftValue::Str(decode_string_literal(&raw)));
        }
        let (body, multiline) = if let Some(b) = raw
            .strip_prefix("\"\"\"")
            .and_then(|s| s.strip_suffix("\"\"\""))
        {
            (strip_multiline_indent(b).to_string(), true)
        } else {
            let b = raw
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(&raw)
                .to_string();
            (b, false)
        };
        let _ = multiline;

        let mut out = String::new();
        let mut chars = body.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' && chars.peek() == Some(&'(') {
                chars.next(); // consume '('
                let mut depth = 1;
                let mut fragment = String::new();
                for fc in chars.by_ref() {
                    match fc {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    fragment.push(fc);
                }
                let value = self.eval_interpolation(&fragment)?;
                out.push_str(&self.render_description(&value));
            } else if c == '\\' {
                // Re-use the escape decoder for the next escape sequence.
                let mut esc = String::from("\\");
                if let Some(&n) = chars.peek() {
                    esc.push(n);
                    chars.next();
                    if n == 'u' && chars.peek() == Some(&'{') {
                        for h in chars.by_ref() {
                            esc.push(h);
                            if h == '}' {
                                break;
                            }
                        }
                    }
                }
                out.push_str(&decode_escapes(&esc));
            } else {
                out.push(c);
            }
        }
        Ok(SwiftValue::Str(out))
    }

    /// Evaluate an interpolated expression fragment against the current scope,
    /// reusing this interpreter (and thus its type/function tables).
    ///
    /// The fragment's analysis is intentionally leaked so its AST nodes live for
    /// `'static`, matching the interpreter's AST lifetime. Fragments are tiny and
    /// a program is run once, so the leak is bounded and acceptable.
    fn eval_interpolation(&mut self, fragment: &str) -> Result<SwiftValue, Signal> {
        let analysis = Analysis::analyze(fragment, "interpolation")
            .map_err(|e| EvalError::Type(format!("interpolation parse error: {e}")))?;
        if !analysis.is_ok() {
            return Err(EvalError::Type(format!("invalid interpolation `{fragment}`")).into());
        }
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        let root = analysis.root();
        // Evaluate the wrapped expression statement directly.
        self.eval(&root)
    }

    /// Invoke a user function by its table id with (possibly labeled) arguments.
    fn call_function(&mut self, id: usize, args: Vec<CallArg>) -> Eval {
        if id >= self.funcs.len() {
            return Err(EvalError::UnknownFunction("<function value>".into()).into());
        }
        self.depth += 1;
        if self.depth > MAX_CALL_DEPTH {
            self.depth -= 1;
            return Err(trap(
                "stack overflow: recursion exceeded the maximum call depth".into(),
            ));
        }

        // Bind parameters in a fresh scope over the function's captured chain.
        let params = clone_params(&self.funcs[id].params);
        let body = self.funcs[id].body;
        let captured = self.funcs[id].captured.clone();
        let generics = self.funcs[id].generic_params.clone();
        let call_env = Env::with_captured(captured);
        let saved = std::mem::replace(&mut self.env, call_env);

        // Bind generic type parameters to the concrete types of the arguments
        // so a static reference through a placeholder (`T.zero()`) resolves.
        let type_binding = self.infer_type_bindings(&generics, &params, &args);
        self.type_bindings.push(type_binding);

        let bound = self.bind_params(&params, args);
        let outcome = match bound {
            Ok(inout_binds) => {
                let result = match body {
                    Some(b) => match self.eval(&b) {
                        Ok(v) => Ok(v),
                        Err(Signal::Return(v)) => Ok(v),
                        Err(other) => Err(other),
                    },
                    None => Ok(SwiftValue::Void),
                };
                // Capture inout finals before tearing down the call scope.
                let writes: Vec<(Place, SwiftValue)> = inout_binds
                    .iter()
                    .filter_map(|(name, place)| self.env.get(name).map(|v| (place.clone(), v)))
                    .collect();
                result.map(|v| (v, writes))
            }
            Err(e) => Err(e),
        };

        self.type_bindings.pop();
        self.env = saved;
        self.depth -= 1;

        let (mut value, writes) = outcome?;
        for (place, val) in writes {
            self.write_place(&place, val)?;
        }
        // Apply the declared tuple return labels so `f().lo` resolves even when
        // the returned tuple literal carried no labels of its own.
        if let (SwiftValue::Tuple(items, labels), Some(decl)) =
            (&mut value, &self.funcs[id].ret_tuple_labels)
        {
            if items.len() == decl.len() && labels.iter().all(Option::is_none) {
                *labels = decl.clone();
            }
        }
        Ok(value)
    }

    /// Infer generic type-parameter substitutions from the concrete runtime
    /// types of a call's arguments. A declared parameter type that is a single
    /// placeholder (`T`) or an array of one (`[T]`) and is not a registered or
    /// builtin type binds that placeholder to the argument's concrete type.
    fn infer_type_bindings(
        &self,
        generics: &[String],
        params: &[Param],
        args: &[CallArg],
    ) -> HashMap<String, String> {
        let mut bindings = HashMap::new();
        if generics.is_empty() {
            return bindings;
        }
        let is_placeholder = |name: &str| generics.iter().any(|g| g == name);
        for (i, p) in params.iter().enumerate() {
            let Some(ty) = p.ty.as_deref() else { continue };
            let Some(arg) = args.get(i) else { continue };
            let ty = ty.trim();
            // `[T]` — bind T to the element type of an array argument.
            if let Some(inner) = ty.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                let inner = inner.trim();
                if is_placeholder(inner) {
                    if let SwiftValue::Array(items) = &arg.value {
                        if let Some(first) = items.first() {
                            bindings
                                .entry(inner.to_string())
                                .or_insert_with(|| first.type_name());
                        }
                    }
                }
            } else if is_placeholder(ty) {
                bindings
                    .entry(ty.to_string())
                    .or_insert_with(|| arg.value.type_name());
            }
        }
        bindings
    }

    /// Resolve a type name through the active generic substitutions, returning
    /// the concrete type a placeholder (`T`) is bound to in the current call.
    fn resolve_type_alias(&self, name: &str) -> Option<String> {
        self.type_bindings
            .iter()
            .rev()
            .find_map(|frame| frame.get(name).cloned())
    }

    /// The name of the type enclosing the currently executing method, used to
    /// resolve `Self`. An instance method derives it from the dynamic type of
    /// the bound `self` (so `Self` in a base class refers to the subclass);
    /// a `static`/type method derives it from the recorded static context.
    fn current_self_type(&self) -> Option<String> {
        match self.env.get("self") {
            Some(SwiftValue::Struct(o)) => return Some(o.type_name.clone()),
            Some(SwiftValue::Object(o)) => return Some(o.borrow().class_name.clone()),
            Some(SwiftValue::Enum(e)) => return Some(e.type_name.clone()),
            _ => {}
        }
        self.static_ctx.last().cloned()
    }

    /// Rewrite the `Self` type keyword to the enclosing type's name; any other
    /// name is returned unchanged.
    fn resolve_self_keyword(&self, name: String) -> String {
        if name == "Self" {
            self.current_self_type().unwrap_or(name)
        } else {
            name
        }
    }

    /// Bind `args` to `params` in the current scope, returning the caller
    /// write-back locations for any `inout` parameters.
    fn bind_params(
        &mut self,
        params: &[Param],
        args: Vec<CallArg>,
    ) -> Result<Vec<(String, Place)>, Signal> {
        let mut inout_binds = Vec::new();
        let mut ai = 0;
        for p in params {
            if p.variadic {
                let mut pack = Vec::new();
                let effective = p.label.as_deref().unwrap_or(p.name.as_str());
                while ai < args.len()
                    && (args[ai].label.is_none() || args[ai].label.as_deref() == Some(effective))
                {
                    pack.push(args[ai].value.clone());
                    ai += 1;
                }
                self.env
                    .declare(&p.name, SwiftValue::Array(Rc::new(pack)), false);
            } else if ai < args.len() {
                let arg = &args[ai];
                // `inout` params are mutable and write back to the caller.
                // Coerce an integer literal argument into a floating parameter.
                let value = coerce_numeric(arg.value.clone(), p.ty.as_deref());
                self.env.declare(&p.name, value, p.inout_);
                if p.inout_ {
                    if let Some(place) = arg.place.clone() {
                        inout_binds.push((p.name.clone(), place));
                    }
                }
                ai += 1;
            } else if let Some(def) = p.default {
                let v = self.eval(&def)?;
                self.env.declare(&p.name, v, false);
            } else {
                return Err(EvalError::Type(format!("missing argument for `{}`", p.name)).into());
            }
        }
        Ok(inout_binds)
    }

    /// Draw the next 64-bit value from the SplitMix64 builtin RNG.
    fn next_random(&mut self) -> u64 {
        self.rng_state = self.rng_state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.rng_state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// Resolve an lvalue expression to a [`Place`] (root variable + field path).
    /// Write the elements of `value` (a tuple) to a list of lvalue targets, as
    /// in tuple-destructuring assignment `(a, b) = (b, a + b)`.
    fn assign_destructured(
        &mut self,
        targets: &[Node<'static>],
        value: SwiftValue,
    ) -> Result<(), Signal> {
        let SwiftValue::Tuple(items, _) = value else {
            return Err(EvalError::Type(
                "tuple-destructuring assignment expects a tuple value".into(),
            )
            .into());
        };
        if items.len() != targets.len() {
            return Err(EvalError::Type(format!(
                "tuple pattern has {} elements but value has {}",
                targets.len(),
                items.len()
            ))
            .into());
        }
        for (t, v) in targets.iter().zip(items.iter().cloned()) {
            self.assign_destructured_one(t, v)?;
        }
        Ok(())
    }

    /// Assign one already-evaluated `value` to a single lvalue `target` (a
    /// destructuring-assignment element): a nested tuple, a wildcard discard, a
    /// class-instance member, or a place-based binding.
    fn assign_destructured_one(
        &mut self,
        target: &Node<'static>,
        value: SwiftValue,
    ) -> Result<(), Signal> {
        match target.kind() {
            NodeKind::TupleExpr => {
                let nested: Vec<Node<'static>> = target.children().collect();
                self.assign_destructured(&nested, value)
            }
            // `_` discards its element.
            NodeKind::WildcardPattern => Ok(()),
            NodeKind::IdentExpr if target.text().as_deref() == Some("_") => Ok(()),
            NodeKind::MemberExpr => {
                // A class-instance member mutates in place (reference semantics).
                if let Some(base) = target.children().next() {
                    let base_value = self.eval(&base)?;
                    if let SwiftValue::Object(obj) = &base_value {
                        let field = target.text().ok_or_else(|| {
                            EvalError::Unsupported("member assignment without a name".into())
                        })?;
                        self.set_object_field(obj, &field, value);
                        return Ok(());
                    }
                }
                let place = self.resolve_place(target).ok_or_else(|| {
                    EvalError::Unsupported("unsupported assignment target".into())
                })?;
                self.write_place(&place, value)
            }
            _ => {
                let place = self.resolve_place(target).ok_or_else(|| {
                    EvalError::Unsupported("unsupported assignment target".into())
                })?;
                self.write_place(&place, value)
            }
        }
    }

    fn resolve_place(&self, node: &Node<'static>) -> Option<Place> {
        match node.kind() {
            NodeKind::IdentExpr => {
                let root = node.text()?;
                // A bare identifier that is not a local binding but names a
                // member of the enclosing `self` resolves as an implicit
                // `self.<name>` place (members shadow module globals), so
                // mutating-method writes flow back.
                if self.env.get_local(&root).is_none() {
                    if self.is_self_member(&root) {
                        return Some(Place {
                            root: "self".into(),
                            path: vec![root],
                        });
                    }
                    // An unqualified static property inside a `static` method
                    // becomes a place rooted at its `Type.name` static key, so
                    // mutating-method writes flow back to the static storage.
                    if let Some(key) = self.implicit_static_key(&root) {
                        return Some(Place {
                            root: key,
                            path: Vec::new(),
                        });
                    }
                }
                Some(Place {
                    root,
                    path: Vec::new(),
                })
            }
            NodeKind::MemberExpr => {
                let member = node.text()?;
                let base = node.children().next()?;
                let mut place = self.resolve_place(&base)?;
                place.path.push(member);
                Some(place)
            }
            _ => None,
        }
    }

    /// Whether the leaf member written by `path` resolves to a `nonmutating`
    /// computed setter on its containing struct. Used to decide that an
    /// immutable value-type root need not (and must not) be reassigned after the
    /// write, because the effect landed through a reference.
    fn leaf_setter_nonmutating(&mut self, root: &SwiftValue, path: &[String]) -> bool {
        let Some((leaf, parents)) = path.split_last() else {
            return false;
        };
        // Descend to the struct that directly holds the leaf member.
        let mut container = root.clone();
        for seg in parents {
            match self.read_struct_member(&container, seg) {
                Ok(v) => container = v,
                Err(_) => return false,
            }
        }
        let SwiftValue::Struct(obj) = &container else {
            return false;
        };
        self.structs
            .get(&obj.type_name)
            .and_then(|d| d.computed.get(leaf))
            .is_some_and(|c| c.setter_nonmutating)
    }

    /// Write `value` to the storage named by `place`, applying copy-on-write and
    /// any property observers at the leaf.
    fn write_place(&mut self, place: &Place, value: SwiftValue) -> Result<(), Signal> {
        if place.path.is_empty() {
            // A static-property place is rooted at its `Type.name` key.
            if self.env.get(&place.root).is_none() && self.statics.contains_key(&place.root) {
                self.statics.insert(place.root.clone(), value);
                return Ok(());
            }
            return match self.env.assign(&place.root, value) {
                Ok(()) => Ok(()),
                Err(BindError::Immutable(n)) => Err(EvalError::Immutable(n).into()),
                Err(BindError::Unbound(n)) => Err(EvalError::UnknownVariable(n).into()),
            };
        }
        let root_val = self
            .env
            .get(&place.root)
            .or_else(|| self.statics.get(&place.root).cloned())
            .ok_or_else(|| EvalError::UnknownVariable(place.root.clone()))?;
        if self.env.get(&place.root).is_none() && self.statics.contains_key(&place.root) {
            let updated = self.set_in(root_val, &place.path, value)?;
            self.statics.insert(place.root.clone(), updated);
            return Ok(());
        }
        // A class instance is mutated in place through its shared storage, so
        // the root binding need not (and, for an immutable `self`, must not) be
        // reassigned — its identity is unchanged.
        let root_is_object = matches!(root_val, SwiftValue::Object(_));
        // A `nonmutating` computed setter at the leaf writes through a reference
        // (e.g. `Binding.wrappedValue` storing into a shared `_StateBox`),
        // leaving the value-type root unchanged — so there is nothing to write
        // back, and a `let` root must not be treated as an illegal mutation.
        let leaf_nonmutating = self.leaf_setter_nonmutating(&root_val, &place.path);
        let updated = self.set_in(root_val, &place.path, value)?;
        if root_is_object || leaf_nonmutating {
            return Ok(());
        }
        match self.env.assign(&place.root, updated) {
            Ok(()) => Ok(()),
            Err(BindError::Immutable(n)) => Err(EvalError::Immutable(n).into()),
            Err(BindError::Unbound(n)) => Err(EvalError::UnknownVariable(n).into()),
        }
    }

    /// Recursively set the value at `path` within `container`, honoring
    /// observers/computed setters at each struct level.
    fn set_in(&mut self, container: SwiftValue, path: &[String], value: SwiftValue) -> Eval {
        let (head, rest) = path.split_first().expect("non-empty path");
        // A class instance is mutated in place through its shared storage (its
        // identity is preserved), so writing a field — possibly nested through a
        // value member — does not rebuild the object.
        if let SwiftValue::Object(obj) = &container {
            let obj = obj.clone();
            if rest.is_empty() {
                self.set_object_field(&obj, head, value);
            } else {
                let sub = self.read_object_member(&container, head)?;
                let new_sub = self.set_in(sub, rest, value)?;
                self.set_object_field(&obj, head, new_sub);
            }
            return Ok(container);
        }
        if rest.is_empty() {
            return self.set_struct_field(container, head, value);
        }
        let sub = self.read_struct_member(&container, head)?;
        let new_sub = self.set_in(sub, rest, value)?;
        self.set_struct_field(container, head, new_sub)
    }

    /// Attempt a numeric/string conversion `Type(value)`. Returns `Ok(None)` if
    /// `name` is not a known conversion type.
    /// Build a dictionary from `Dictionary(uniqueKeysWithValues:)` (a sequence
    /// of key/value tuples) or `Dictionary(grouping:by:)` (bucket elements by a
    /// key closure). Returns `None` if the args match neither initializer.
    fn build_dictionary(&mut self, args: &[CallArg]) -> Result<Option<SwiftValue>, Signal> {
        let labeled = |name: &str| args.iter().find(|a| a.label.as_deref() == Some(name));

        if let Some(seq) = labeled("uniqueKeysWithValues") {
            let elements = materialize_sequence(&seq.value)
                .ok_or_else(|| EvalError::Type("uniqueKeysWithValues expects a sequence".into()))?;
            let mut pairs: Vec<(SwiftValue, SwiftValue)> = Vec::new();
            for el in elements {
                if let SwiftValue::Tuple(t, _) = el {
                    if t.len() == 2 {
                        pairs.push((t[0].clone(), t[1].clone()));
                        continue;
                    }
                }
                return Err(EvalError::Type(
                    "uniqueKeysWithValues expects (key, value) pairs".into(),
                )
                .into());
            }
            return Ok(Some(SwiftValue::Dict(StdRc::new(pairs))));
        }

        if let (Some(seq), Some(by)) = (labeled("grouping"), labeled("by")) {
            let elements = materialize_sequence(&seq.value)
                .ok_or_else(|| EvalError::Type("grouping: expects a sequence".into()))?;
            let SwiftValue::Closure(id) = by.value else {
                return Err(EvalError::Type("grouping by: expects a closure".into()).into());
            };
            let mut pairs: Vec<(SwiftValue, Vec<SwiftValue>)> = Vec::new();
            for el in elements {
                let key = self.call_closure(id, vec![el.clone()])?;
                match pairs.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1.push(el),
                    None => pairs.push((key, vec![el])),
                }
            }
            let pairs = pairs
                .into_iter()
                .map(|(k, v)| (k, SwiftValue::Array(StdRc::new(v))))
                .collect();
            return Ok(Some(SwiftValue::Dict(StdRc::new(pairs))));
        }

        Ok(None)
    }

    fn try_conversion(&self, name: &str, value: &SwiftValue) -> Result<Option<SwiftValue>, Signal> {
        if let Some(w) = IntWidth::from_type_name(name) {
            let raw = match value {
                SwiftValue::Int(i) => i.raw,
                SwiftValue::Double(d) => d.trunc() as i128,
                SwiftValue::Bool(b) => *b as i128,
                // Failable string conversion: `Int("42")` → 42, `Int("x")` → nil.
                SwiftValue::Str(s) => {
                    return Ok(Some(match s.trim().parse::<i128>() {
                        Ok(n) => {
                            let v = IntValue::new(n, w);
                            if v.in_range() {
                                SwiftValue::Int(v)
                            } else {
                                SwiftValue::Nil
                            }
                        }
                        Err(_) => SwiftValue::Nil,
                    }))
                }
                _ => {
                    return Err(EvalError::Type(format!(
                        "cannot convert {} to {name}",
                        value.type_name()
                    ))
                    .into())
                }
            };
            let v = IntValue::new(raw, w);
            if !v.in_range() {
                return Err(trap(format!("{raw} is not representable as {name}")));
            }
            return Ok(Some(SwiftValue::Int(v)));
        }
        match name {
            "Double" | "Float" => {
                let d = match value {
                    SwiftValue::Int(i) => i.raw as f64,
                    SwiftValue::Double(d) => *d,
                    // Failable string conversion: `Double("3.14")` → 3.14.
                    SwiftValue::Str(s) => {
                        return Ok(Some(match s.trim().parse::<f64>() {
                            Ok(n) => SwiftValue::Double(n),
                            Err(_) => SwiftValue::Nil,
                        }))
                    }
                    _ => {
                        return Err(EvalError::Type(format!(
                            "cannot convert {} to {name}",
                            value.type_name()
                        ))
                        .into())
                    }
                };
                Ok(Some(SwiftValue::Double(d)))
            }
            "String" => Ok(Some(SwiftValue::Str(value.to_string()))),
            // Failable `Bool("true")`/`Bool("false")`.
            "Bool" => Ok(match value {
                SwiftValue::Str(s) => Some(match s.trim() {
                    "true" => SwiftValue::Bool(true),
                    "false" => SwiftValue::Bool(false),
                    _ => SwiftValue::Nil,
                }),
                SwiftValue::Bool(b) => Some(SwiftValue::Bool(*b)),
                _ => None,
            }),
            // `Array(seq)` materializes any builtin sequence into an array.
            "Array" => Ok(materialize_sequence(value).map(|v| SwiftValue::Array(StdRc::new(v)))),
            // `Set(seq)` deduplicates a materialized sequence into a set.
            "Set" => Ok(materialize_sequence(value)
                .map(|v| SwiftValue::Set(StdRc::new(dedup_preserving_order(v))))),
            // `ContiguousArray(seq)` is an array in this model.
            "ContiguousArray" => {
                Ok(materialize_sequence(value).map(|v| SwiftValue::Array(StdRc::new(v))))
            }
            // `CollectionOfOne(x)` is a one-element array.
            "CollectionOfOne" => Ok(Some(SwiftValue::Array(StdRc::new(vec![value.clone()])))),
            _ => Ok(None),
        }
    }
}

/// Eagerly materialize a builtin sequence value into a `Vec` of its elements,
/// or `None` if the value is not a sequence the tree-walker can expand.
fn materialize_sequence(value: &SwiftValue) -> Option<Vec<SwiftValue>> {
    match value {
        SwiftValue::Array(items) => Some(items.as_ref().clone()),
        SwiftValue::Range { lo, hi, inclusive } => {
            let end = if *inclusive { *hi + 1 } else { *hi };
            Some((*lo..end).map(SwiftValue::int).collect())
        }
        SwiftValue::Str(s) => Some(
            crate::graphemes(s)
                .into_iter()
                .map(SwiftValue::Str)
                .collect(),
        ),
        // A dictionary is a sequence of `(key, value)` tuples.
        SwiftValue::Dict(pairs) => Some(
            pairs
                .iter()
                .map(|(k, v)| dict_element_tuple(k.clone(), v.clone()))
                .collect(),
        ),
        SwiftValue::Set(items) => Some(items.as_ref().clone()),
        _ => None,
    }
}

/// A dictionary element `(key:, value:)` tuple, carrying the Swift labels so
/// both `element.key`/`element.value` access and printing behave like Swift.
fn dict_element_tuple(key: SwiftValue, value: SwiftValue) -> SwiftValue {
    SwiftValue::tuple_labeled(
        vec![key, value],
        vec![Some("key".to_string()), Some("value".to_string())],
    )
}

/// The capability surface the standard-library seam sees: a narrow window onto
/// the interpreter (call a closure, write output, throw) — not the whole engine.
impl StdContext for Interpreter<'_> {
    fn call_closure(&mut self, id: usize, args: Vec<SwiftValue>) -> crate::stdlib::StdResult {
        // Reuse the inherent closure caller; translate its control-flow channel
        // into the seam's error type.
        match Interpreter::call_closure(self, id, args) {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(sig) => Err(Self::signal_to_std_error(sig)),
        }
    }

    fn eval_block_values(&mut self, id: usize) -> crate::stdlib::StdResult {
        match self.eval_builder_block(id) {
            Ok(values) => Ok(SwiftValue::Array(Rc::new(values))),
            Err(sig) => Err(Self::signal_to_std_error(sig)),
        }
    }

    fn eval_block_values_with_args(
        &mut self,
        id: usize,
        args: Vec<SwiftValue>,
    ) -> crate::stdlib::StdResult {
        match self.eval_builder_block_with_args(id, args) {
            Ok(values) => Ok(SwiftValue::Array(Rc::new(values))),
            Err(sig) => Err(Self::signal_to_std_error(sig)),
        }
    }

    fn get_member(&mut self, value: &SwiftValue, name: &str) -> crate::stdlib::StdResult {
        self.read_struct_member(value, name)
            .map_err(Self::signal_to_std_error)
    }

    fn inject_environment_objects(
        &mut self,
        view: &SwiftValue,
        wrapper_type: &str,
        objects: &[SwiftValue],
    ) -> crate::stdlib::StdResult {
        let SwiftValue::Struct(obj) = view else {
            return Ok(view.clone());
        };
        // The fields wrapped by `wrapper_type` (e.g. `EnvironmentObject`), with
        // each field's declared type for matching the right object.
        let plan: Vec<(String, Option<String>)> = self
            .structs
            .get(&obj.type_name)
            .map(|d| {
                d.stored
                    .iter()
                    .filter(|p| d.wrappers.get(&p.name).map(String::as_str) == Some(wrapper_type))
                    .map(|p| (p.name.clone(), p.ty.clone()))
                    .collect()
            })
            .unwrap_or_default();
        if plan.is_empty() {
            return Ok(view.clone());
        }
        let mut updated = (**obj).clone();
        for (field, declared) in plan {
            // Match by declared type name; only when the declared type is
            // unknown do we fall back to the sole environment object — never
            // inject a type-mismatched object into a typed slot.
            let chosen = objects
                .iter()
                .find(|o| self.value_type_name(o).as_deref() == declared.as_deref())
                .or(if declared.is_none() && objects.len() == 1 {
                    objects.first()
                } else {
                    None
                });
            let Some(object) = chosen else { continue };
            // Set the wrapper instance's single stored slot to the object.
            if let Some(slot) = updated.fields.iter_mut().find(|(k, _)| k == &field) {
                if let SwiftValue::Struct(wrapper) = &slot.1 {
                    let mut w = (**wrapper).clone();
                    if let Some(inner) = w.fields.first_mut() {
                        inner.1 = object.clone();
                    }
                    slot.1 = SwiftValue::Struct(Rc::new(w));
                }
            }
        }
        Ok(SwiftValue::Struct(Rc::new(updated)))
    }

    fn out(&mut self) -> &mut dyn Write {
        self.out
    }

    fn display(&mut self, value: &SwiftValue) -> String {
        self.render_description(value)
    }

    fn random_u64(&mut self) -> u64 {
        self.next_random()
    }

    fn value_less_than(&mut self, a: &SwiftValue, b: &SwiftValue) -> Option<bool> {
        // Scalars use the natural order; struct/enum/class operands consult a
        // static `<` operator method on their type.
        if let Some(less) = crate::stdlib::scalar_less_than(a, b) {
            return Some(less);
        }
        let tn = self.value_type_name(a)?;
        if self.type_has_method(&tn, "<") {
            let args = vec![
                CallArg {
                    label: None,
                    value: a.clone(),
                    place: None,
                },
                CallArg {
                    label: None,
                    value: b.clone(),
                    place: None,
                },
            ];
            if let Ok(SwiftValue::Bool(b)) =
                self.call_struct_method(SwiftValue::Void, &tn, "<", args, None)
            {
                return Some(b);
            }
        }
        // Synthesized `Comparable` for an enum that declares the conformance but
        // defines no `<`: order by case-declaration index, then lexicographically
        // by associated values (Swift's derived ordering, since 5.3).
        if let (SwiftValue::Enum(ea), SwiftValue::Enum(eb)) = (a, b) {
            if ea.type_name == eb.type_name
                && self
                    .all_protocols(&ea.type_name)
                    .iter()
                    .any(|p| p == "Comparable")
            {
                // Case index in declaration order; differing cases decide it.
                let indices = self.enums.get(&ea.type_name).and_then(|def| {
                    let ia = def.cases.iter().position(|c| c.name == ea.case)?;
                    let ib = def.cases.iter().position(|c| c.name == eb.case)?;
                    Some((ia, ib))
                });
                if let Some((ia, ib)) = indices {
                    if ia != ib {
                        return Some(ia < ib);
                    }
                    // Same case: lexicographic comparison over payloads.
                    let (pa, pb) = (ea.payload.clone(), eb.payload.clone());
                    for (va, vb) in pa.iter().zip(pb.iter()) {
                        if self.value_less_than(va, vb)? {
                            return Some(true);
                        }
                        if self.value_less_than(vb, va)? {
                            return Some(false);
                        }
                    }
                    return Some(false);
                }
            }
        }
        None
    }
}

/// Extract `T` from a metatype argument node `T.self`.
fn metatype_name(node: &Node<'static>) -> Option<String> {
    if node.kind() == NodeKind::MemberExpr && node.text().as_deref() == Some("self") {
        node.children().next().and_then(|b| b.text())
    } else {
        node.text()
    }
}

/// Clone a parameter list (`Node` is `Copy`; only the strings allocate).
/// Whether `name` is an operator token (every character is an operator symbol),
/// i.e. a bare operator used in value position such as the `+` in
/// `reduce(0, +)`. A normal identifier never contains these characters.
/// The element type of an array type spelling (`[C?]` → `C?`, `[Int]` → `Int`),
/// or `None` if `name` is not an `[…]` array type.
/// The Codable element type of a field spelling: strip one `[...]` array layer
/// and/or a trailing `?` optional so `[User]`/`Role?`/`User` all yield the
/// nominal element type to decode against.
fn decode_element_type(spelling: &str) -> &str {
    let t = spelling.trim();
    let t = t.strip_suffix('?').unwrap_or(t).trim();
    let t = t
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(t)
        .trim();
    // A second optional layer (`[User]?` already handled; `User??` rare).
    t.strip_suffix('?').unwrap_or(t).trim()
}

fn array_element_type(name: &str) -> Option<&str> {
    let inner = name.strip_prefix('[')?.strip_suffix(']')?;
    // Reject dictionary types `[K: V]`; only homogeneous element arrays here.
    if inner.contains(':') {
        return None;
    }
    Some(inner.trim())
}

/// The written type annotation of a stored-property declaration (`var x: Double`
/// → `Double`), if any.
fn field_type_name(member: &Node<'static>) -> Option<String> {
    member
        .children()
        .find(|c| c.kind() == NodeKind::TypeRef)
        .and_then(|c| c.text())
}

/// Coerce an integer value to floating point when the target type is
/// `Double`/`Float` (an integer literal in a floating context).
fn coerce_numeric(value: SwiftValue, target_ty: Option<&str>) -> SwiftValue {
    if let (SwiftValue::Int(i), Some("Double") | Some("Float")) = (&value, target_ty) {
        return SwiftValue::Double(i.raw as f64);
    }
    value
}

fn is_operator_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| "+-*/%<>=!&|^~".contains(c))
}

/// Extract the generic type-parameter names from a declaration's
/// `GenericParam` child. `<T: P, U>` yields `["T", "U"]`.
fn generic_param_names(node: &Node<'static>) -> Vec<String> {
    let Some(gp) = node.children().find(|c| c.kind() == NodeKind::GenericParam) else {
        return Vec::new();
    };
    let Some(text) = gp.text() else {
        return Vec::new();
    };
    let inner = text.trim().trim_start_matches('<').trim_end_matches('>');
    inner
        .split(',')
        .filter_map(|part| {
            let name = part.split(':').next().unwrap_or("").trim();
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

/// Parse the element labels of a tuple type written as `(lo: Int, hi: Int)`.
/// Returns `None` when the text is not a labeled tuple of two or more elements
/// (a single parenthesized type is not a tuple, and a function type `->` is
/// excluded). Each element yields `Some(label)` or `None` when unlabeled.
fn tuple_type_labels(text: &str) -> Option<Vec<Option<String>>> {
    let inner = text.trim().strip_prefix('(')?.strip_suffix(')')?;
    // Split on top-level commas, tracking bracket depth so nested tuples,
    // arrays, dictionaries, and generics are not split apart.
    let mut parts: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in inner.chars() {
        match ch {
            '(' | '[' | '<' => {
                depth += 1;
                cur.push(ch);
            }
            ')' | ']' | '>' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut cur));
            }
            _ => cur.push(ch),
        }
    }
    parts.push(cur);
    if parts.len() < 2 {
        return None;
    }
    let mut labels = Vec::with_capacity(parts.len());
    let mut any = false;
    for part in parts {
        let part = part.trim();
        // A function type element rules out a plain tuple return.
        if part.contains("->") {
            return None;
        }
        // `name: Type` — the label is a leading identifier before a top-level
        // colon. Anything else (a bare type) is unlabeled.
        let label = part.split_once(':').and_then(|(name, _)| {
            let name = name.trim();
            (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
                .then(|| name.to_string())
        });
        any |= label.is_some();
        labels.push(label);
    }
    any.then_some(labels)
}

fn clone_method(m: &MethodDef) -> MethodDef {
    MethodDef {
        params: clone_params(&m.params),
        body: m.body,
        mutating: m.mutating,
        generic_params: m.generic_params.clone(),
        is_static: m.is_static,
    }
}

/// Select exactly one overload by labels, then by runtime argument types when
/// labels alone leave multiple candidates.
fn select_labeled_overload<'a>(
    overloads: &'a [MethodDef],
    args: &[CallArg],
) -> Option<&'a MethodDef> {
    let label_matches: Vec<&MethodDef> = overloads
        .iter()
        .filter(|def| args_select_params(args, &def.params))
        .collect();
    match label_matches.as_slice() {
        [one] => Some(*one),
        [] => None,
        many => {
            let type_matches: Vec<&&MethodDef> = many
                .iter()
                .filter(|def| args_match_param_types(args, &def.params))
                .collect();
            match type_matches.as_slice() {
                [one] => Some(**one),
                _ => None,
            }
        }
    }
}

/// Whether a call's argument labels select `params` as the matching overload.
///
/// A parameter's *effective* label is its explicit argument label, or its name
/// when none is written (Swift's rule: a bare `name:` parameter has label
/// `name`). A positional (unlabeled) argument is treated as matching any
/// parameter, since the runtime cannot separate overloads by type. The caller
/// only commits to a selection when exactly one candidate matches.
fn args_select_params(args: &[CallArg], params: &[Param]) -> bool {
    let mut ai = 0usize;
    for param in params {
        let effective = param.label.as_deref().unwrap_or(param.name.as_str());
        if param.variadic {
            while ai < args.len()
                && (args[ai].label.is_none() || args[ai].label.as_deref() == Some(effective))
            {
                ai += 1;
            }
        } else if let Some(arg) = args.get(ai) {
            if let Some(label) = &arg.label {
                if label != effective {
                    return false;
                }
            }
            ai += 1;
        } else if param.default.is_none() {
            return false;
        }
    }
    ai == args.len()
}

/// Whether each argument's runtime type matches its parameter's declared type,
/// used to disambiguate overloads separable only by type. A parameter with no
/// written type accepts any argument.
fn args_match_param_types(args: &[CallArg], params: &[Param]) -> bool {
    let mut ai = 0usize;
    for param in params {
        let expected = param
            .ty
            .as_deref()
            .map(|ty| ty.trim().trim_end_matches('?'));
        if param.variadic {
            while ai < args.len()
                && (args[ai].label.is_none()
                    || args[ai].label.as_deref()
                        == Some(param.label.as_deref().unwrap_or(param.name.as_str())))
            {
                if let Some(ty) = expected {
                    if args[ai].value.type_name() != ty {
                        return false;
                    }
                }
                ai += 1;
            }
        } else if let Some(arg) = args.get(ai) {
            if let Some(ty) = expected {
                if arg.value.type_name() != ty {
                    return false;
                }
            }
            ai += 1;
        } else if param.default.is_none() {
            return false;
        }
    }
    ai == args.len()
}

fn clone_params(params: &[Param]) -> Vec<Param> {
    params
        .iter()
        .map(|p| Param {
            label: p.label.clone(),
            name: p.name.clone(),
            ty: p.ty.clone(),
            variadic: p.variadic,
            inout_: p.inout_,
            autoclosure: p.autoclosure,
            default: p.default,
        })
        .collect()
}

/// Parse the `AST_PARAM` children of a function/method declaration.
fn parse_params(node: &Node<'static>) -> Vec<Param> {
    let mut params = Vec::new();
    for child in node.children() {
        if child.kind() == NodeKind::Param {
            let info = child.param_info();
            // The parameter's default value, if any, is a non-type child.
            let default = child.children().find(is_value_node);
            let ty = child
                .children()
                .find(|c| c.kind() == NodeKind::TypeRef)
                .and_then(|c| c.text());
            params.push(Param {
                label: info.label,
                name: info.name,
                ty,
                variadic: info.variadic,
                inout_: info.is_inout,
                autoclosure: info.autoclosure,
                default,
            });
        }
    }
    params
}

/// Map each call argument to its declared parameter type, mirroring the
/// positional/labeled matching of [`autoclosure_flags`]. Variadic parameters
/// absorb the remaining positional arguments and keep their element type as the
/// hint. Used to push a contextual type while evaluating an argument.
fn param_type_hints(params: &[Param], args: &[Node<'static>]) -> Vec<Option<String>> {
    let mut hints = vec![None; args.len()];
    let mut pi = 0usize;
    for (i, arg) in args.iter().enumerate() {
        let target = match arg.arg_label() {
            Some(l) => params
                .iter()
                .enumerate()
                .find(|(_, p)| p.label.as_deref() == Some(l.as_str()) || p.name == l),
            None => params.get(pi).map(|p| (pi, p)),
        };
        if let Some((idx, p)) = target {
            hints[i] = p.ty.clone();
            if !p.variadic {
                pi = idx + 1;
            }
        }
    }
    hints
}

/// For each call argument, whether its target parameter is `@autoclosure`
/// (and so its expression must be deferred into a thunk). Arguments are aligned
/// to parameters positionally, with labelled arguments matched by name — enough
/// for the way `@autoclosure` parameters are written in practice.
fn autoclosure_flags(params: &[Param], args: &[Node<'static>]) -> Vec<bool> {
    let mut flags = vec![false; args.len()];
    let mut pi = 0usize;
    for (i, arg) in args.iter().enumerate() {
        let target = match arg.arg_label() {
            Some(l) => params
                .iter()
                .enumerate()
                .find(|(_, p)| p.label.as_deref() == Some(l.as_str()) || p.name == l),
            None => params.get(pi).map(|p| (pi, p)),
        };
        if let Some((idx, p)) = target {
            if !p.variadic {
                flags[i] = p.autoclosure;
                pi = idx + 1;
            }
        }
    }
    flags
}

/// Whether a node kind is a refutable/binding pattern (as appears in a
/// `for case <pattern> in …` loop or a `switch` case).
fn is_pattern_node(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::EnumCasePattern
            | NodeKind::TuplePattern
            | NodeKind::RangePattern
            | NodeKind::WildcardPattern
            | NodeKind::NamePattern
    )
}

/// What a loop body asks its loop to do next.
enum LoopFlow {
    Continue,
    Break,
}

/// Whether a node is an expression (vs. a type annotation or other non-value
/// child appearing under a declaration).
fn is_expr(node: &Node) -> bool {
    is_value_node(node)
}

/// The literal syntax kind represented by `node`, including Swift's treatment
/// of a negated numeric literal as literal-convertible syntax.
fn literal_syntax_kind(node: &Node) -> Option<NodeKind> {
    match node.kind() {
        NodeKind::ArrayLiteral
        | NodeKind::DictLiteral
        | NodeKind::StringLiteral
        | NodeKind::IntegerLiteral
        | NodeKind::FloatLiteral
        | NodeKind::BoolLiteral
        | NodeKind::NilLiteral => Some(node.kind()),
        NodeKind::PrefixExpr if node.op_text().as_deref() == Some("-") => {
            node.children().next().and_then(|child| match child.kind() {
                NodeKind::IntegerLiteral | NodeKind::FloatLiteral => Some(child.kind()),
                _ => None,
            })
        }
        _ => None,
    }
}

/// Whether `value` is a sequence the synchronous `iterate` already understands
/// (so a `for await` over it can use the eager path rather than the async
/// iterator protocol).
fn is_builtin_iterable(value: &SwiftValue) -> bool {
    matches!(
        value,
        SwiftValue::Range { .. }
            | SwiftValue::Array(_)
            | SwiftValue::Str(_)
            | SwiftValue::Dict(_)
            | SwiftValue::Set(_)
    )
}

/// Deduplicate elements preserving first-seen order (set construction).
fn dedup_preserving_order(items: Vec<SwiftValue>) -> Vec<SwiftValue> {
    let mut out: Vec<SwiftValue> = Vec::with_capacity(items.len());
    for it in items {
        if !out.contains(&it) {
            out.push(it);
        }
    }
    out
}

/// Whether a node is a value expression (not a type annotation, accessor, or
/// pattern node that can appear as a declaration child).
fn is_value_node(node: &Node) -> bool {
    // Binding patterns (`let x`, `let (a, b)`, …) sit as children of a
    // declaration alongside its value; they are never the value themselves.
    !is_pattern_node(node.kind())
        && !matches!(
            node.kind(),
            NodeKind::TypeRef | NodeKind::Accessor | NodeKind::Attribute
        )
}

/// Whether a node kind is a statement (as opposed to a `switch` pattern).
fn is_statement_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::ExprStmt
            | NodeKind::Block
            | NodeKind::ReturnStmt
            | NodeKind::IfStmt
            | NodeKind::GuardStmt
            | NodeKind::ForStmt
            | NodeKind::WhileStmt
            | NodeKind::RepeatStmt
            | NodeKind::SwitchStmt
            | NodeKind::BreakStmt
            | NodeKind::ContinueStmt
            | NodeKind::FallthroughStmt
            | NodeKind::VarDecl
            | NodeKind::LetDecl
            | NodeKind::FuncDecl
    )
}

/// Expand `#if` conditional-compilation wrappers inline so the active branch's
/// statements, declarations, and members belong to the enclosing scope. The
/// parser already selected the active branch; this only flattens the
/// `CompilerDirective("if")` wrapper (recursively, for nested `#if`). Other
/// directives (`#warning`, `#line`, …) are left untouched.
fn expand_directives(node: &Node<'static>) -> Vec<Node<'static>> {
    let mut out = Vec::new();
    for child in node.children() {
        push_active_branch(child, &mut out);
    }
    out
}

/// Like [`expand_directives`], but over an already-collected list of nodes
/// (e.g. a closure body whose statements were gathered before execution).
fn expand_directive_list(nodes: Vec<Node<'static>>) -> Vec<Node<'static>> {
    let mut out = Vec::new();
    for node in nodes {
        push_active_branch(node, &mut out);
    }
    out
}

/// Push `node` into `out`, flattening a `MacroExpansion("if")` wrapper into its
/// active-branch children (recursively, for nested `#if`).
fn push_active_branch(node: Node<'static>, out: &mut Vec<Node<'static>>) {
    if node.kind() == NodeKind::CompilerDirective && node.text().as_deref() == Some("if") {
        for child in node.children() {
            push_active_branch(child, out);
        }
    } else {
        out.push(node);
    }
}

/// Split a `case` clause into (patterns, body-statements). Patterns are the
/// leading non-statement children; the body is everything from the first
/// statement onward.
fn case_parts(case: &Node<'static>) -> (Vec<Node<'static>>, Vec<Node<'static>>) {
    let mut patterns = Vec::new();
    let mut body = Vec::new();
    let mut in_body = false;
    for child in case.children() {
        // The `where` guard is read separately via `case_info()`, not matched
        // as a pattern.
        if child.kind() == NodeKind::WhereClause {
            continue;
        }
        if !in_body && is_statement_kind(child.kind()) {
            in_body = true;
        }
        if in_body {
            body.push(child);
        } else {
            patterns.push(child);
        }
    }
    (patterns, body)
}

/// Structural value equality used by `switch` value patterns and `==`.
fn values_equal(a: &SwiftValue, b: &SwiftValue) -> bool {
    match (a, b) {
        (SwiftValue::Int(x), SwiftValue::Int(y)) => x.raw == y.raw,
        (SwiftValue::Double(x), SwiftValue::Double(y)) => x == y,
        (SwiftValue::Bool(x), SwiftValue::Bool(y)) => x == y,
        (SwiftValue::Str(x), SwiftValue::Str(y)) => x == y,
        (SwiftValue::Nil, SwiftValue::Nil) => true,
        (SwiftValue::Tuple(x, _), SwiftValue::Tuple(y, _)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| values_equal(p, q))
        }
        (SwiftValue::Enum(x), SwiftValue::Enum(y)) => {
            x.type_name == y.type_name
                && x.case == y.case
                && x.payload.len() == y.payload.len()
                && x.payload
                    .iter()
                    .zip(&y.payload)
                    .all(|(p, q)| values_equal(p, q))
        }
        _ => false,
    }
}

/// Extract a single integer subscript index from evaluated index args.
/// Resolve a `lo..<hi` / `lo...hi` integer range into validated `start..end`
/// slice bounds against a collection of length `len`. Traps (Swift `fatalError`)
/// when the range escapes `0...len` or is inverted.
fn slice_bounds(lo: i128, hi: i128, inclusive: bool, len: usize) -> Result<(usize, usize), Signal> {
    // Validate against the *raw* bounds first: both `a..<b` and `a...b` require
    // `b >= a` (a closed `2...1` is inverted and traps). Computing `end` before
    // this check would mask `inclusive` inversions, since `hi + 1` can lift an
    // inverted upper bound back to `== lo`.
    if lo < 0 || hi < lo {
        return Err(trap(format!(
            "invalid range {lo}..{}{hi} for collection of length {len}",
            if inclusive { "=" } else { "<" }
        )));
    }
    let end = if inclusive { hi + 1 } else { hi };
    if end > len as i128 {
        return Err(trap(format!(
            "range {lo}..{}{hi} out of bounds for collection of length {len}",
            if inclusive { "=" } else { "<" }
        )));
    }
    Ok((lo as usize, end as usize))
}

fn subscript_index(indices: &[SwiftValue]) -> Result<usize, Signal> {
    match indices.first() {
        Some(SwiftValue::Int(i)) if i.raw >= 0 => Ok(i.raw as usize),
        Some(SwiftValue::Int(i)) => Err(trap(format!("negative index {}", i.raw))),
        _ => Err(EvalError::Type("subscript index must be an integer".into()).into()),
    }
}

/// Decode a Swift string literal's *source text* (including its delimiters) into
/// the runtime string it denotes: strips quotes and processes escapes.
/// Strip the delimiters off a regex literal lexeme, leaving the bare pattern.
/// Handles the bare `/.../` form and the extended `#/.../#` form (any number of
/// `#`). Extended literals additionally trim surrounding whitespace on a
/// single-line body, matching Swift's extended-delimiter regex semantics.
fn strip_regex_delimiters(raw: &str) -> &str {
    if raw.starts_with('#') {
        let hashes = raw.chars().take_while(|&c| c == '#').count();
        let inner = &raw[hashes..raw.len().saturating_sub(hashes)];
        let inner = inner
            .strip_prefix('/')
            .and_then(|s| s.strip_suffix('/'))
            .unwrap_or(inner);
        return if inner.contains('\n') {
            inner
        } else {
            inner.trim()
        };
    }
    raw.strip_prefix('/')
        .and_then(|s| s.strip_suffix('/'))
        .unwrap_or(raw)
}

fn decode_string_literal(raw: &str) -> String {
    if raw.starts_with('#') {
        let hashes = raw.chars().take_while(|&c| c == '#').count();
        let inner = &raw[hashes..raw.len().saturating_sub(hashes)];
        let inner = inner
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(inner);
        return inner.to_string();
    }
    if let Some(body) = raw
        .strip_prefix("\"\"\"")
        .and_then(|s| s.strip_suffix("\"\"\""))
    {
        return decode_escapes(strip_multiline_indent(body));
    }
    let body = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(raw);
    decode_escapes(body)
}

fn strip_multiline_indent(body: &str) -> &str {
    body.trim_start_matches('\n').trim_end_matches([' ', '\t'])
}

fn decode_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('u') => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    let mut hex = String::new();
                    for h in chars.by_ref() {
                        if h == '}' {
                            break;
                        }
                        hex.push(h);
                    }
                    if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(cp) {
                            out.push(ch);
                        }
                    }
                } else {
                    out.push('u');
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Result<String, EvalError> {
        let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut interp = Interpreter::new(&mut buf);
            crate::install_test_print(&mut interp);
            interp.run(analysis)?;
        }
        Ok(String::from_utf8(buf).unwrap())
    }

    #[test]
    fn struct_method_fallback_dispatches_and_yields_to_user_methods() {
        // A registered generic struct method appends a tag; a user method of the
        // same name must win over it.
        fn tag(_ctx: &mut dyn StdContext, recv: SwiftValue, _args: Vec<Arg>) -> crate::StdResult {
            let SwiftValue::Struct(obj) = &recv else {
                return Ok(SwiftValue::Str("non-struct".into()));
            };
            Ok(SwiftValue::Str(format!("builtin:{}", obj.type_name)))
        }

        let src = concat!(
            "struct A {}\n",
            "struct B { func tag() -> String { \"user\" } }\n",
            "let a = A()\n",
            "let b = B()\n",
            "print(a.tag())\n",
            "print(b.tag())\n",
        );
        let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut interp = Interpreter::new(&mut buf);
            crate::install_test_print(&mut interp);
            interp.register_struct_method("tag", tag);
            interp.run(analysis).expect("run");
        }
        // `A` has no `tag` method, so the generic fallback fires; `B` defines
        // one, so the user method wins.
        assert_eq!(String::from_utf8(buf).unwrap(), "builtin:A\nuser\n");
    }

    #[test]
    fn eval_block_values_returns_each_statement_value() {
        // A free function receives a trailing closure and asks the context to
        // evaluate it as a result-builder block, counting its statements.
        fn count(ctx: &mut dyn StdContext, args: Vec<Arg>) -> crate::StdResult {
            let SwiftValue::Closure(id) = args[0].value else {
                return Ok(SwiftValue::int(-1));
            };
            let block = ctx.eval_block_values(id)?;
            let n = match block {
                SwiftValue::Array(items) => items.len() as i128,
                _ => -1,
            };
            Ok(SwiftValue::int(n))
        }
        let src = "print(count { 1; 2; 3 })\n";
        let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut interp = Interpreter::new(&mut buf);
            crate::install_test_print(&mut interp);
            interp.register_free_fn("count", count);
            interp.run(analysis).expect("run");
        }
        assert_eq!(String::from_utf8(buf).unwrap(), "3\n");
    }

    #[test]
    fn nonmutating_setter_writes_through_a_let_binding() {
        // A `nonmutating set` writes through a shared reference, so assigning
        // through a `let` value-type binding is allowed and the effect lands.
        let out = run("class Box { var v: Int; init(_ n: Int) { v = n } }
\
struct Ref {
  let box: Box
  var value: Int { get { box.value } nonmutating set { box.value = newValue } }
}
let bx = Box(7)
let r = Ref(box: bx)
r.value = 9
print(bx.value)
")
        .expect("nonmutating set through a let binding is allowed");
        assert_eq!(out, "9\n");
    }

    #[test]
    fn assigning_a_mutating_member_through_a_let_root_still_errors() {
        // The `nonmutating` allowance must not loosen ordinary value semantics:
        // writing a stored property of a `let` struct is still illegal even when
        // the new value equals the old.
        let err = run("struct S { var x: Int }
let s = S(x: 1)
s.x = 1
")
        .expect_err("assigning a let struct's stored property must error");
        assert!(
            matches!(err, EvalError::Immutable(_)),
            "expected immutability error, got {err:?}"
        );
    }

    #[test]
    fn make_struct_and_get_member_drive_a_computed_property() {
        // The public render entry points: instantiate a struct, then read a
        // computed getter (as a render host evaluates a `View`'s `body`).
        let src = "struct V { var greeting: String { \"hi\" } }\n";
        let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        let mut buf: Vec<u8> = Vec::new();
        let mut interp = Interpreter::new(&mut buf);
        crate::install_test_print(&mut interp);
        interp.run(analysis).expect("run");
        let v = interp.make_struct("V", &[]).expect("make_struct");
        let greeting = interp.get_member(&v, "greeting").expect("get_member");
        assert_eq!(greeting, SwiftValue::Str("hi".into()));
    }

    #[test]
    fn conditional_compilation_expands_in_scopes() {
        // `#if` active branches expand at top level, in type bodies, in
        // function bodies, and in closure bodies.
        let out = run("#if DEBUG
let tag = \"d\"
struct Box { func v() -> String { \"box:\\(tag)\" } }
#endif
print(tag)
print(Box().v())
func f() -> Int {
  #if DEBUG
  return 1
  #else
  return 2
  #endif
}
print(f())
let c = {
  #if DEBUG
  print(\"clo\")
  #endif
}
c()
")
        .unwrap();
        assert_eq!(out, "d\nbox:d\n1\nclo\n");
    }

    #[test]
    fn optional_binding_conditions_unwrap_and_match() {
        // Simple optional binding unwraps and binds; nil fails the branch.
        let out = run("let a: Int? = 7
let b: Int? = nil
if let x = a { print(\"a=\\(x)\") } else { print(\"a-none\") }
if let _ = b { print(\"b-some\") } else { print(\"b-none\") }
if let _ = a { print(\"a-some\") } else { print(\"a-none2\") }
")
        .unwrap();
        assert_eq!(out, "a=7\nb-none\na-some\n");
    }

    #[test]
    fn case_condition_binds_associated_value() {
        // `if case` matches a refutable pattern and binds its payload.
        let out = run("enum E { case a(Int), b }
let e = E.a(42)
if case .a(let n) = e { print(n) } else { print(\"no\") }
if case .b = e { print(\"b\") } else { print(\"not-b\") }
")
        .unwrap();
        assert_eq!(out, "42\nnot-b\n");
    }

    #[test]
    fn implicit_static_method_disambiguated_by_param_type() {
        // Two types each declare `static func custom(_:)`; the call-site
        // parameter type disambiguates the implicit member.
        let out = run("struct Color { let name: String\n\
               static func custom(_ n: String) -> Color { Color(name: n) } }\n\
             struct Font { let name: String\n\
               static func custom(_ n: String) -> Font { Font(name: n) } }\n\
             func describe(_ c: Color) -> String { \"color:\\(c.name)\" }\n\
             print(describe(.custom(\"green\")))\n")
        .unwrap();
        assert_eq!(out, "color:green\n");
    }

    #[test]
    fn implicit_enum_case_disambiguated_by_param_type() {
        // `.red` is a case of both enums; the parameter type picks `Color`.
        let out = run("enum Color { case red, green }\n\
             enum Mood { case red, calm }\n\
             func describe(_ c: Color) -> String { \"\\(c)\" }\n\
             print(describe(.red))\n")
        .unwrap();
        assert_eq!(out, "red\n");
    }

    #[test]
    fn implicit_static_property_disambiguated_by_param_type() {
        // `static let light` exists on both types; the param type picks `Theme`.
        let out = run(
            "struct Theme { static let light = Theme(); let tag = \"T\" }\n\
             struct Page { static let light = Page() }\n\
             func render(_ t: Theme) -> String { t.tag }\n\
             print(render(.light))\n",
        )
        .unwrap();
        assert_eq!(out, "T\n");
    }

    #[test]
    fn implicit_member_disambiguated_in_variadic_arg() {
        // The contextual type propagates into each variadic argument.
        let out = run("struct Tag { let v: String\n\
               static func custom(_ s: String) -> Tag { Tag(v: s) } }\n\
             struct Mark { let v: String\n\
               static func custom(_ s: String) -> Mark { Mark(v: s) } }\n\
             func join(_ items: Tag...) -> Int { items.count }\n\
             print(join(.custom(\"a\"), .custom(\"b\")))\n")
        .unwrap();
        assert_eq!(out, "2\n");
    }

    #[test]
    fn protocol_composition_typealias_resolves_default_method() {
        let out = run(
            "protocol Scorable { var score: Int { get }; func grade() -> String }\n\
             extension Scorable { func grade() -> String { score >= 90 ? \"A\" : \"B\" } }\n\
             protocol Named { var name: String { get } }\n\
             typealias NS = Named & Scorable\n\
             struct S: NS { let name: String; let score: Int }\n\
             print(S(name: \"a\", score: 95).grade())\n",
        )
        .unwrap();
        assert_eq!(out, "A\n");
    }

    #[test]
    fn static_method_overloads_dispatch_by_argument_label() {
        // Two same-named static methods separable only by argument label must
        // each be reachable (the result-builder transform relies on this for
        // `buildEither(first:)` vs `(second:)`).
        let out = run("struct B {\n\
             static func pick(first: String) -> String { \"F:\" + first }\n\
             static func pick(second: String) -> String { \"S:\" + second }\n\
             }\n\
             print(B.pick(first: \"a\"))\n\
             print(B.pick(second: \"b\"))\n")
        .unwrap();
        assert_eq!(out, "F:a\nS:b\n");
    }

    #[test]
    fn static_overloads_dispatch_by_argument_type() {
        // Two same-named static methods separable only by parameter type are
        // each selected by the argument's runtime type (Tier B, #124).
        let out = run("struct B {\n\
             static func describe(_ v: String) -> String { \"str:\" + v }\n\
             static func describe(_ v: Int) -> String { \"int:\\(v)\" }\n\
             }\n\
             print(B.describe(\"a\"))\n\
             print(B.describe(7))\n")
        .unwrap();
        assert_eq!(out, "str:a\nint:7\n");
    }

    #[test]
    fn memory_layout_primitives_and_structs() {
        let out = run(
            "print(MemoryLayout<Int>.size, MemoryLayout<Int>.stride, MemoryLayout<Int>.alignment)\n\
             print(MemoryLayout<Int8>.size, MemoryLayout<Bool>.size, MemoryLayout<Float>.size)\n\
             struct Pair { var flag: Int8; var value: Int }\n\
             print(MemoryLayout<Pair>.size, MemoryLayout<Pair>.stride, MemoryLayout<Pair>.alignment)\n",
        )
        .unwrap();
        assert_eq!(out, "8 8 8\n1 1 4\n16 16 8\n");
    }

    #[test]
    fn key_paths_read_write_and_function() {
        let out = run("struct Address { var city: String }\n\
             struct Person { var name: String; var address: Address }\n\
             var p = Person(name: \"Ada\", address: Address(city: \"London\"))\n\
             print(p[keyPath: \\Person.name])\n\
             print(p[keyPath: \\Person.address.city])\n\
             p[keyPath: \\Person.address.city] = \"Paris\"\n\
             print(p.address.city)\n\
             print([\"a\", \"bb\"].map(\\.count))\n\
             print([1, 2, 3].map(\\.self))\n")
        .unwrap();
        assert_eq!(out, "Ada\nLondon\nParis\n[1, 2]\n[1, 2, 3]\n");
    }

    #[test]
    fn key_path_writes_through_let_class_reference() {
        // A class is a reference type: a writable key path mutates it in place
        // even when it is held in a `let` binding.
        let out = run("class Box { var n: Int; init(_ v: Int) { n = v } }\n\
             let b = Box(1)\n\
             b[keyPath: \\Box.n] = 9\n\
             print(b.n)\n")
        .unwrap();
        assert_eq!(out, "9\n");
    }

    #[test]
    fn key_path_identity_replacement_on_var_class() {
        // `obj[keyPath: \.self] = other` replaces the whole reference (a
        // different instance), so the `var` binding is rebound.
        let out = run("class Box { var n: Int; init(_ v: Int) { n = v } }\n\
             var b = Box(1)\n\
             b[keyPath: \\Box.n] = 5\n\
             print(b.n)\n\
             b[keyPath: \\Box.self] = Box(99)\n\
             print(b.n)\n")
        .unwrap();
        assert_eq!(out, "5\n99\n");
    }

    #[test]
    fn memory_layout_recursive_struct_fails_safely() {
        // A self-referential value type has no finite layout: report it as an
        // unsupported construct rather than recursing until the stack overflows.
        let err = run("struct A { var a: A }\nprint(MemoryLayout<A>.size)\n").unwrap_err();
        match err {
            EvalError::Unsupported(msg) => assert!(msg.contains("MemoryLayout<A>"), "{msg}"),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn tuple_type_labels_parses_labeled_tuples() {
        assert_eq!(
            tuple_type_labels("(lo: Int, hi: Int)"),
            Some(vec![Some("lo".into()), Some("hi".into())])
        );
        // Mixed labeled/unlabeled.
        assert_eq!(
            tuple_type_labels("(Int, value: String)"),
            Some(vec![None, Some("value".into())])
        );
        // Nested brackets are not split at their inner commas.
        assert_eq!(
            tuple_type_labels("(a: (Int, Int), b: [Int: Int])"),
            Some(vec![Some("a".into()), Some("b".into())])
        );
        // Not tuples / no labels.
        assert_eq!(tuple_type_labels("(Int, Int)"), None);
        assert_eq!(tuple_type_labels("(Int)"), None);
        assert_eq!(tuple_type_labels("Int"), None);
        // A function type is not a labeled tuple return.
        assert_eq!(tuple_type_labels("(a: Int) -> Int"), None);
    }

    #[test]
    fn is_operator_name_recognizes_operator_tokens() {
        assert!(is_operator_name("+"));
        assert!(is_operator_name(">"));
        assert!(is_operator_name("<="));
        assert!(is_operator_name("=="));
        assert!(!is_operator_name("foo"));
        assert!(!is_operator_name(""));
        assert!(!is_operator_name("a+"));
    }

    #[test]
    fn operator_function_reference_reduces() {
        let out = run("print([1, 2, 3, 4].reduce(0, +))\n").unwrap();
        assert_eq!(out, "10\n");
    }

    #[test]
    fn operator_function_reference_multiplies() {
        // The `*` operator passed as a function value (the full comparator form,
        // e.g. `sorted(by: >)`, is covered by the tswift-cli golden fixtures
        // where the real sequence algorithms are installed).
        let out = run("print([1, 2, 3, 4].reduce(1, *))\n").unwrap();
        assert_eq!(out, "24\n");
    }

    #[test]
    fn if_case_binds_enum_payload() {
        let out = run(
            "enum E { case a(Int); case b }\nlet e = E.a(7)\nif case .a(let n) = e { print(n) }\n",
        )
        .unwrap();
        assert_eq!(out, "7\n");
    }

    #[test]
    fn guard_case_matches_without_binding() {
        let out = run(
            "enum E { case a; case b }\nfunc f(_ e: E) -> Bool {\n  guard case .a = e else { return false }\n  return true\n}\nprint(f(.a), f(.b))\n",
        )
        .unwrap();
        assert_eq!(out, "true false\n");
    }

    #[test]
    fn tuple_destructuring_assignment_swaps() {
        let out = run("var a = 1\nvar b = 2\n(a, b) = (b, a + b)\nprint(a, b)\n").unwrap();
        assert_eq!(out, "2 3\n");
    }

    #[test]
    fn named_tuple_element_access() {
        let out = run("let p = (min: 1, max: 9)\nprint(p.min, p.max, p.0, p.1)\n").unwrap();
        assert_eq!(out, "1 9 1 9\n");
    }

    #[test]
    fn named_tuple_from_function_return_label() {
        let out = run(
            "func f() -> (lo: Int, hi: Int) { return (lo: 2, hi: 8) }\nlet r = f()\nprint(r.lo, r.hi)\n",
        )
        .unwrap();
        assert_eq!(out, "2 8\n");
    }

    #[test]
    fn named_tuple_prints_labels() {
        let out = run("print((x: 10, y: 20))\n").unwrap();
        assert_eq!(out, "(x: 10, y: 20)\n");
    }

    #[test]
    fn dict_element_key_value_resolve_by_label() {
        let out = run("let d = [\"a\": 1]\nfor e in d { print(e.key, e.value) }\n").unwrap();
        assert_eq!(out, "a 1\n");
    }

    #[test]
    fn plain_tuple_rejects_key_label() {
        // `.key`/`.value` are not admitted on an arbitrary 2-tuple — only on a
        // tuple actually carrying those labels (a dictionary element).
        let err = run("let p = (x: 1, y: 2)\nprint(p.key)\n").unwrap_err();
        assert!(err.to_string().contains("member .key"), "{err}");
    }

    #[test]
    fn int_literal_coerces_to_double_binding() {
        let out = run("let r: Double = 5\nprint(r)\n").unwrap();
        assert_eq!(out, "5.0\n");
    }

    #[test]
    fn struct_double_field_coerces_int_literal() {
        let out = run(
            "struct P { var x: Double; var y: Double }\nlet p = P(x: 3, y: 4)\nprint(p.x, p.y)\n",
        )
        .unwrap();
        assert_eq!(out, "3.0 4.0\n");
    }

    #[test]
    fn integer_division_unaffected_by_promotion() {
        let out = run("print(7 / 2, 7 % 2)\n").unwrap();
        assert_eq!(out, "3 1\n");
    }

    #[test]
    fn array_covariant_optional_cast() {
        let out = run("let xs = [1, 2, 3] as [Int?]\nprint(xs.count)\n").unwrap();
        assert_eq!(out, "3\n");
    }

    #[test]
    fn arithmetic_and_bindings() {
        let out = run("let a = 7\nvar b = a * 6\nb += 1\nprint(b)\n").unwrap();
        assert_eq!(out, "43\n");
    }

    #[test]
    fn wrapping_add_on_int_max() {
        let out = run("print(Int.max &+ 1)\n").unwrap();
        assert_eq!(out, format!("{}\n", i64::MIN));
    }

    #[test]
    fn overflow_traps() {
        let err = run("print(Int.max + 1)\n").unwrap_err();
        assert!(matches!(err, EvalError::Trap(_)), "got {err:?}");
    }

    #[test]
    fn let_is_immutable() {
        let err = run("let a = 1\na = 2\n").unwrap_err();
        assert!(
            matches!(err, EvalError::Immutable(_) | EvalError::Analysis(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn typed_width_conversions() {
        let out = run("let x: UInt8 = 255\nprint(x &+ 1)\n").unwrap();
        assert_eq!(out, "0\n");
    }

    #[test]
    fn double_formatting() {
        let out = run("print(3.0)\nprint(3.5)\n").unwrap();
        assert_eq!(out, "3.0\n3.5\n");
    }

    #[test]
    fn int_from_double_truncates() {
        let out = run("print(Int(3.9))\n").unwrap();
        assert_eq!(out, "3\n");
    }

    #[test]
    fn factorial_recurses() {
        let out = run(
            "func factorial(_ n: Int) -> Int { return n == 0 ? 1 : n * factorial(n - 1) }\nprint(factorial(5))\n",
        )
        .unwrap();
        assert_eq!(out, "120\n");
    }

    #[test]
    fn labels_defaults_and_calls() {
        let out = run(
            "func add(_ a: Int, to b: Int = 5) -> Int { return a + b }\nprint(add(1))\nprint(add(2, to: 3))\n",
        )
        .unwrap();
        assert_eq!(out, "6\n5\n");
    }

    #[test]
    fn first_class_functions() {
        let out = run(
            "func inc(_ n: Int) -> Int { return n + 1 }\nfunc apply(_ f: (Int) -> Int, _ x: Int) -> Int { return f(x) }\nprint(apply(inc, 5))\n",
        )
        .unwrap();
        assert_eq!(out, "6\n");
    }

    #[test]
    fn variadic_collects_into_array() {
        let out =
            run("func n(_ xs: Int...) -> Int { return xs.count }\nprint(n(1, 2, 3))\nprint(n())\n")
                .unwrap();
        assert_eq!(out, "3\n0\n");
    }

    #[test]
    fn mutual_recursion_and_forward_reference() {
        let out = run(
            "func isEven(_ n: Int) -> Bool { return n == 0 ? true : isOdd(n - 1) }\nfunc isOdd(_ n: Int) -> Bool { return n == 0 ? false : isEven(n - 1) }\nprint(isEven(10))\n",
        )
        .unwrap();
        assert_eq!(out, "true\n");
    }

    #[test]
    fn control_flow_loops_and_switch() {
        let out = run(
            "var total = 0\nfor i in 0..<5 where i % 2 == 0 { total += i }\nswitch total {\ncase 0...3: print(\"small \\(total)\")\ndefault: print(\"big \\(total)\")\n}\n",
        )
        .unwrap();
        assert_eq!(out, "big 6\n");
    }

    #[test]
    fn labeled_break_and_continue() {
        let out = run(
            "outer: for i in 1...3 {\n  for j in 1...3 {\n    if j == 2 { continue outer }\n    if i == 3 { break outer }\n    print(\"\\(i),\\(j)\")\n  }\n}\n",
        )
        .unwrap();
        assert_eq!(out, "1,1\n2,1\n");
    }

    #[test]
    fn switch_tuple_where_and_fallthrough() {
        let out = run(
            "func c(_ p: (Int, Int)) -> String {\n  switch p {\n  case (let x, 0): return \"x \\(x)\"\n  case (_, let y) where y > 10: return \"hi \\(y)\"\n  default: return \"other\"\n  }\n}\nprint(c((5, 0)))\nprint(c((1, 20)))\nprint(c((1, 2)))\nswitch 2 { case 2: print(\"two\"); fallthrough\ncase 3: print(\"three\")\ndefault: print(\"x\") }\n",
        )
        .unwrap();
        assert_eq!(out, "x 5\nhi 20\nother\ntwo\nthree\n");
    }

    #[test]
    fn tuple_decomposition_and_guard() {
        let out = run(
            "let (a, b) = (3, 4)\nprint(a + b)\nfunc f(_ x: Int) -> Int { guard x > 0 else { return -1 }\n return x * 2 }\nprint(f(5), f(-2))\n",
        )
        .unwrap();
        assert_eq!(out, "7\n10 -1\n");
    }

    #[test]
    fn string_interpolation_renders_expressions() {
        let out = run("let n = 6\nprint(\"n*n = \\(n * n)\")\n").unwrap();
        assert_eq!(out, "n*n = 36\n");
    }

    #[test]
    fn struct_value_copy_semantics() {
        let out = run(
            "struct Point { var x: Int; var y: Int\n  mutating func move(dx: Int) { x += dx }\n  var magnitude: Int { x*x + y*y } }\nvar a = Point(x: 1, y: 2)\nvar b = a\nb.move(dx: 10)\nprint(a.x, b.x)\nprint(b.magnitude)\n",
        )
        .unwrap();
        assert_eq!(out, "1 11\n125\n");
    }

    #[test]
    fn computed_setter_and_observers() {
        let out = run(
            "struct C { var n: Int = 0 { didSet { print(\"set \\(n)\") } }\n  var twice: Int { get { n * 2 } set { n = newValue / 2 } } }\nvar c = C()\nc.n = 3\nprint(c.twice)\nc.twice = 10\nprint(c.n)\n",
        )
        .unwrap();
        assert_eq!(out, "set 3\n6\nset 5\n5\n");
    }

    #[test]
    fn inout_writes_back_through_place() {
        let out = run(
            "struct B { var v: Int }\nfunc bump(_ x: inout Int) { x += 1 }\nvar n = 10\nbump(&n)\nvar b = B(v: 5)\nbump(&b.v)\nprint(n, b.v)\n",
        )
        .unwrap();
        assert_eq!(out, "11 6\n");
    }

    #[test]
    fn closure_shorthand_inout_writes_back() {
        let out =
            run("let f: (inout Int) -> Void = { $0 += 1 }\nvar x = 5\nf(&x)\nf(&x)\nprint(x)\n")
                .unwrap();
        assert_eq!(out, "7\n");
    }

    #[test]
    fn closure_inout_requires_ampersand() {
        let err =
            run("let f: (inout Int) -> Void = { (n: inout Int) in n += 1 }\nvar x = 5\nf(x)\n")
                .unwrap_err();
        assert!(matches!(err, EvalError::Trap(_)), "got {err:?}");
    }

    #[test]
    fn closure_throws_signature_writes_back_before_throw() {
        let out = run(
            "struct Boom: Error {}\nlet f: (inout Int) throws -> Void = { (n: inout Int) throws in n += 1; throw Boom() }\nvar x = 5\ndo { try f(&x) } catch {}\nprint(x)\n",
        )
        .unwrap();
        assert_eq!(out, "6\n");
    }

    #[test]
    fn static_type_property() {
        let out = run(
            "struct M { static let answer = 42\n  var x: Int }\nprint(M.answer)\nlet m = M(x: 1)\nprint(m.x)\n",
        )
        .unwrap();
        assert_eq!(out, "42\n1\n");
    }

    #[test]
    fn enum_associated_values_and_matching() {
        let out = run(
            "enum Shape { case circle(r: Int); case rect(Int, Int) }\nfunc area(_ s: Shape) -> Int {\n  switch s {\n  case .circle(let r): return 3 * r * r\n  case .rect(let w, let h): return w * h\n  }\n}\nprint(area(.circle(r: 5)))\nprint(area(Shape.rect(3, 4)))\n",
        )
        .unwrap();
        assert_eq!(out, "75\n12\n");
    }

    #[test]
    fn enum_raw_values_and_methods() {
        let out = run(
            "enum Dir: Int, CaseIterable { case n = 1, e = 2, s = 3\n  func twice() -> Int { return rawValue * 2 } }\nprint(Dir.s.rawValue)\nprint(Dir.e.twice())\nprint(Dir.allCases.count)\n",
        )
        .unwrap();
        assert_eq!(out, "3\n4\n3\n");
    }

    #[test]
    fn optionals_binding_chaining_coalescing() {
        let out = run(
            "var maybe: Int? = nil\nprint(maybe ?? -1)\nif let v = maybe { print(v) } else { print(\"none\") }\nmaybe = 7\nprint(maybe!)\nlet s: String? = \"hi\"\nprint(s?.count ?? 0)\n",
        )
        .unwrap();
        assert_eq!(out, "-1\nnone\n7\n2\n");
    }

    #[test]
    fn force_unwrap_nil_traps() {
        let err = run("let x: Int? = nil\nprint(x!)\n").unwrap_err();
        assert!(matches!(err, EvalError::Trap(_)), "got {err:?}");
    }

    #[test]
    fn indirect_enum_recursion() {
        let out = run(
            "indirect enum E { case num(Int); case add(E, E) }\nfunc ev(_ e: E) -> Int {\n  switch e { case .num(let n): return n; case .add(let a, let b): return ev(a) + ev(b) }\n}\nprint(ev(.add(.num(3), .add(.num(4), .num(5)))))\n",
        )
        .unwrap();
        assert_eq!(out, "12\n");
    }

    #[test]
    fn subscripts_array_and_user() {
        let out = run(
            "let a = [10, 20, 30]\nprint(a[1])\nstruct Grid { var cells: [Int]\n  subscript(_ i: Int) -> Int { return cells[i] * 2 } }\nlet g = Grid(cells: [5, 6, 7])\nprint(g[2])\n",
        )
        .unwrap();
        assert_eq!(out, "20\n14\n");
    }

    #[test]
    fn class_reference_semantics_and_dispatch() {
        let out = run(
            "class Animal { func speak() -> String { return \"...\" } }\nclass Dog: Animal { override func speak() -> String { return \"woof\" } }\nlet a: Animal = Dog()\nprint(a.speak())\nlet b = a\nprint(a === b)\n",
        )
        .unwrap();
        assert_eq!(out, "woof\ntrue\n");
    }

    #[test]
    fn class_shares_by_reference() {
        let out = run(
            "class Box { var v: Int\n init(_ x: Int) { v = x } }\nlet a = Box(1)\nlet b = a\nb.v = 99\nprint(a.v)\n",
        )
        .unwrap();
        assert_eq!(out, "99\n");
    }

    #[test]
    fn super_init_and_method() {
        let out = run(
            "class A { var w: Int\n init(_ x: Int) { w = x }\n func d() -> String { return \"A\\(w)\" } }\nclass B: A { init() { super.init(5) }\n override func d() -> String { return \"B+\" + super.d() } }\nprint(B().d())\n",
        )
        .unwrap();
        assert_eq!(out, "B+A5\n");
    }

    #[test]
    fn closures_capture_and_higher_order() {
        let out = run(
            "let add: (Int) -> Int = { [x = 10] n in n + x }\nprint(add(5))\nlet nums = [1, 2, 3, 4]\nprint(nums.map { $0 * 2 })\nprint(nums.filter { $0 % 2 == 0 })\nprint(nums.reduce(0) { $0 + $1 })\n",
        )
        .unwrap();
        assert_eq!(out, "15\n[2, 4, 6, 8]\n[2, 4]\n10\n");
    }

    #[test]
    fn deinit_fires_at_scope_exit() {
        let out =
            run("class R { deinit { print(\"freed\") } }\ndo { let _ = R() }\nprint(\"after\")\n");
        // `do` blocks arrive with error handling; fall back to a function scope.
        let out = out.or_else(|_| {
            run("class R { deinit { print(\"freed\") } }\nfunc scope() { let _ = R() }\nscope()\nprint(\"after\")\n")
        })
        .unwrap();
        assert_eq!(out, "freed\nafter\n");
    }

    #[test]
    fn arc_retain_release_counts() {
        use crate::value::{ClassObj, SwiftValue};
        use std::cell::RefCell;
        use std::rc::Rc;
        let a = SwiftValue::Object(Rc::new(RefCell::new(ClassObj {
            class_name: "C".into(),
            fields: vec![],
        })));
        let b = a.clone(); // assignment retains
        if let SwiftValue::Object(o) = &a {
            assert_eq!(Rc::strong_count(o), 2, "shared reference retains");
        }
        drop(b); // release
        if let SwiftValue::Object(o) = &a {
            assert_eq!(Rc::strong_count(o), 1, "release lowers the count");
        }
    }

    #[test]
    fn casting_is_as_optional() {
        let out = run(
            "class A {}\nclass B: A {}\nlet x: A = B()\nprint(x is B)\nprint((x as? B) != nil)\nlet n: Int = 5\nprint(n is Int)\n",
        )
        .unwrap();
        assert_eq!(out, "true\ntrue\ntrue\n");
    }

    #[test]
    fn protocol_default_impl_and_existentials() {
        let out = run(
            "protocol Shape { var area: Int { get } }\nstruct Square: Shape { let s: Int; var area: Int { s*s } }\nstruct Circle: Shape { let r: Int; var area: Int { 3*r*r } }\nextension Shape { func describe() -> String { return \"area=\\(area)\" } }\nlet shapes: [any Shape] = [Square(s: 2), Circle(r: 3)]\nprint(shapes.map { $0.area })\nprint(shapes.map { $0.describe() })\n",
        )
        .unwrap();
        assert_eq!(out, "[4, 27]\n[area=4, area=27]\n");
    }

    #[test]
    fn generic_functions_with_constraints() {
        let out = run(
            "func myMax<T: Comparable>(_ a: T, _ b: T) -> T { return a > b ? a : b }\nprint(myMax(3, 7))\nprint(myMax(\"apple\", \"banana\"))\nprotocol Shape { var area: Int { get } }\nstruct Sq: Shape { let s: Int; var area: Int { s*s } }\nfunc total<S: Shape>(_ xs: [S]) -> Int { return xs.reduce(0) { $0 + $1.area } }\nprint(total([Sq(s: 2), Sq(s: 3)]))\n",
        )
        .unwrap();
        assert_eq!(out, "7\nbanana\n13\n");
    }

    #[test]
    fn synthesized_equatable_for_structs() {
        let out = run(
            "struct P: Equatable { let x: Int; let y: Int }\nprint(P(x: 1, y: 2) == P(x: 1, y: 2))\nprint(P(x: 1, y: 2) == P(x: 3, y: 4))\n",
        )
        .unwrap();
        assert_eq!(out, "true\nfalse\n");
    }

    #[test]
    fn custom_operator() {
        let out = run(
            "infix operator **\nfunc ** (a: Int, b: Int) -> Int {\n  var r = 1\n  for _ in 0..<b { r *= a }\n  return r\n}\nprint(2 ** 10)\n",
        )
        .unwrap();
        assert_eq!(out, "1024\n");
    }

    #[test]
    fn extension_adds_methods() {
        // Extensions on a user type add methods.
        let out = run("struct V { let n: Int }\nextension V { func doubled() -> Int { return n * 2 } }\nprint(V(n: 21).doubled())\n")
            .unwrap();
        assert_eq!(out, "42\n");
        // Extensions on a builtin type also add methods, dispatched on the
        // value-typed receiver.
        let out = run(
            "extension Int { func squared() -> Int { return self * self } }\nprint(5.squared())\n",
        )
        .unwrap();
        assert_eq!(out, "25\n");
    }

    #[test]
    fn throw_catch_with_payload_and_defer() {
        let out = run(
            "enum FileError: Error { case notFound(String) }\nfunc read(_ name: String) throws -> String {\n  guard name == \"a.txt\" else { throw FileError.notFound(name) }\n  return \"data\"\n}\ndo {\n  defer { print(\"cleanup\") }\n  print(try read(\"x.txt\"))\n} catch FileError.notFound(let n) { print(\"missing \\(n)\") }\n",
        )
        .unwrap();
        assert_eq!(out, "cleanup\nmissing x.txt\n");
    }

    #[test]
    fn try_optional_and_bang() {
        let out = run(
            "enum E: Error { case bad }\nfunc f(_ ok: Bool) throws -> Int { if ok { return 7 } else { throw E.bad } }\nprint((try? f(true)) ?? -1)\nprint((try? f(false)) ?? -1)\nprint(try! f(true))\n",
        )
        .unwrap();
        assert_eq!(out, "7\n-1\n7\n");
    }

    #[test]
    fn defer_runs_lifo_on_all_exits() {
        let out = run(
            "func g() {\n  defer { print(\"a\") }\n  defer { print(\"b\") }\n  print(\"body\")\n}\ng()\n",
        )
        .unwrap();
        assert_eq!(out, "body\nb\na\n");
    }

    #[test]
    fn result_get_throws() {
        let out = run(
            "enum E: Error { case bad(Int) }\nfunc r(_ x: Int) -> Result<Int, E> { if x < 0 { return .failure(E.bad(x)) } \n return .success(x * 10) }\ndo { print(try r(4).get()) ; _ = try r(-1).get() } catch E.bad(let n) { print(\"failed \\(n)\") }\n",
        )
        .unwrap();
        assert_eq!(out, "40\nfailed -1\n");
    }

    #[test]
    fn property_wrapper_with_projected_value() {
        let out = run(
            "@propertyWrapper struct Clamped {\n  private var value: Int\n  let limit: Int\n  var wrappedValue: Int { get { value } set { value = newValue > limit ? limit : newValue } }\n  var projectedValue: Bool { value == limit }\n  init(wrappedValue: Int) { limit = 10; value = wrappedValue > 10 ? 10 : wrappedValue }\n}\nstruct P { @Clamped var hp: Int = 5 }\nvar p = P()\nprint(p.hp)\np.hp = 100\nprint(p.hp)\nprint(p.$hp)\n",
        )
        .unwrap();
        assert_eq!(out, "5\n10\ntrue\n");
    }

    #[test]
    fn main_entry_point_runs() {
        let out = run(
            "@main struct App {\n  static func main() {\n    print(\"hello from main\")\n  }\n}\n",
        )
        .unwrap();
        assert_eq!(out, "hello from main\n");
    }

    #[test]
    fn codable_json_round_trip() {
        let out = run(
            "struct User: Codable { let name: String; let age: Int }\n@main struct App {\n  static func main() throws {\n    let u = User(name: \"Sam\", age: 30)\n    let data = try JSONEncoder().encode(u)\n    print(data)\n    let back = try JSONDecoder().decode(User.self, from: data)\n    print(back.name, back.age)\n  }\n}\n",
        )
        .unwrap();
        assert_eq!(out, "{\"name\":\"Sam\",\"age\":30}\nSam 30\n");
    }

    #[test]
    fn conditional_compilation_and_macros() {
        // msf resolves `#if` at parse time, leaving only the active branch.
        let out = run(
            "#if os(macOS)\nlet p = \"mac\"\n#else\nlet p = \"other\"\n#endif\nprint(p)\nprint(#line)\n",
        )
        .unwrap();
        // The active branch's value plus the literal line of the `#line` token.
        assert!(
            out.starts_with("mac\n") || out.starts_with("other\n"),
            "got {out:?}"
        );
    }

    #[test]
    fn deep_recursion_traps_not_crashes() {
        // Run on a generous stack so the depth guard fires before any native
        // overflow, proving recursion yields a catchable error.
        let handle = std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| run("func loop(_ n: Int) -> Int { return loop(n + 1) }\nprint(loop(0))\n"))
            .unwrap();
        let result = handle.join().unwrap();
        assert!(matches!(result, Err(EvalError::Trap(_))), "got {result:?}");
    }

    // ----- Structured concurrency (ADR-0005) -----

    #[test]
    fn async_await_round_trips() {
        let out = run(
            "func double(_ x: Int) async -> Int { x * 2 }\nfunc run() async { print(await double(21)) }\nrun()\n",
        )
        .unwrap();
        assert_eq!(out, "42\n");
    }

    #[test]
    fn async_let_runs_children_and_awaits_results() {
        let out = run(
            "func fetch(_ id: Int) async -> Int { id * 2 }\nfunc run() async {\n  async let a = fetch(1)\n  async let b = fetch(2)\n  print(await a + await b)\n}\nrun()\n",
        )
        .unwrap();
        assert_eq!(out, "6\n");
    }

    #[test]
    fn task_value_and_detached_complete() {
        let out = run(
            "func run() async {\n  let t = Task { 20 + 1 }\n  let d = Task.detached { 7 * 6 }\n  print(await t.value)\n  print(await d.value)\n}\nrun()\n",
        )
        .unwrap();
        assert_eq!(out, "21\n42\n");
    }

    #[test]
    fn detached_task_drains_at_program_end() {
        // A `Task { }` whose handle is never awaited still runs before the
        // program's outermost scope exits.
        let out = run("Task { print(\"ran\") }\nprint(\"main\")\n").unwrap();
        assert_eq!(out, "main\nran\n");
    }

    #[test]
    fn task_cancellation_is_cooperative() {
        let out = run(
            "func run() async {\n  let t = Task { 5 }\n  t.cancel()\n  print(t.isCancelled)\n  print(await t.value)\n}\nrun()\n",
        )
        .unwrap();
        // The flag flips, but a body that does not check it still completes.
        assert_eq!(out, "true\n5\n");
    }

    #[test]
    fn task_group_aggregates_child_results() {
        let out = run(
            "func sum(_ n: Int) async -> Int {\n  await withTaskGroup(of: Int.self) { group in\n    for i in 1...n { group.addTask { i * i } }\n    var total = 0\n    for await r in group { total += r }\n    return total\n  }\n}\nfunc run() async { print(await sum(4)) }\nrun()\n",
        )
        .unwrap();
        assert_eq!(out, "30\n");
    }

    #[test]
    fn actor_serializes_state_and_main_actor_runs() {
        let out = run(
            "actor Counter { private var v = 0\n func inc() { v += 1 }\n func get() -> Int { v } }\n@MainActor func show(_ n: Int) { print(n) }\nfunc run() async {\n  let c = Counter()\n  await c.inc(); await c.inc()\n  await show(await c.get())\n}\nrun()\n",
        )
        .unwrap();
        assert_eq!(out, "2\n");
    }

    #[test]
    fn for_await_drives_custom_async_sequence() {
        let out = run(
            "struct Down: AsyncSequence, AsyncIteratorProtocol {\n  var n: Int\n  mutating func next() async -> Int? { if n > 0 { let c = n; n -= 1; return c } else { return nil } }\n  func makeAsyncIterator() -> Down { self }\n}\nfunc run() async {\n  var s = 0\n  for await x in Down(n: 3) { s += x }\n  print(s)\n}\nrun()\n",
        )
        .unwrap();
        assert_eq!(out, "6\n");
    }

    #[test]
    fn checked_continuation_resumes_inline() {
        let out = run(
            "func value() async -> Int {\n  await withCheckedContinuation { c in\n    c.resume(returning: 42)\n  }\n}\nfunc run() async { print(await value()) }\nrun()\n",
        )
        .unwrap();
        assert_eq!(out, "42\n");
    }

    #[test]
    fn unsafe_continuation_resumes_from_a_spawned_task() {
        // The body parks the continuation in a `Task`; draining pending tasks
        // resumes it before `withUnsafeContinuation` reads the slot.
        let out = run(
            "func value() async -> Int {\n  await withUnsafeContinuation { c in\n    Task { c.resume(returning: 7) }\n  }\n}\nfunc run() async { print(await value()) }\nrun()\n",
        )
        .unwrap();
        assert_eq!(out, "7\n");
    }

    #[test]
    fn throwing_continuation_propagates_resume_throwing() {
        let out = run(
            "struct Boom: Error {}\nfunc value() async throws -> Int {\n  try await withCheckedThrowingContinuation { c in\n    c.resume(throwing: Boom())\n  }\n}\nfunc run() async {\n  do { _ = try await value() } catch { print(\"caught\") }\n}\nrun()\n",
        )
        .unwrap();
        assert_eq!(out, "caught\n");
    }

    #[test]
    fn continuation_resume_with_result_unwraps_success() {
        let out = run(
            "func value() async -> Int {\n  await withCheckedContinuation { c in\n    c.resume(with: .success(99))\n  }\n}\nfunc run() async { print(await value()) }\nrun()\n",
        )
        .unwrap();
        assert_eq!(out, "99\n");
    }

    #[test]
    fn void_continuation_resumes_with_no_value() {
        let out = run(
            "func wait() async {\n  await withCheckedContinuation { c in\n    c.resume()\n  }\n}\nfunc run() async { await wait(); print(\"done\") }\nrun()\n",
        )
        .unwrap();
        assert_eq!(out, "done\n");
    }

    #[test]
    fn unresumed_continuation_traps() {
        let err = run(
            "func value() async -> Int {\n  await withCheckedContinuation { c in\n    let _ = 0\n  }\n}\nfunc run() async { print(await value()) }\nrun()\n",
        )
        .unwrap_err();
        assert!(
            matches!(err, EvalError::Trap(ref m) if m.contains("not resumed")),
            "got {err:?}"
        );
    }

    #[test]
    fn double_resume_traps() {
        let err = run(
            "func value() async -> Int {\n  await withCheckedContinuation { c in\n    c.resume(returning: 1)\n    c.resume(returning: 2)\n  }\n}\nfunc run() async { print(await value()) }\nrun()\n",
        )
        .unwrap_err();
        assert!(
            matches!(err, EvalError::Trap(ref m) if m.contains("more than once")),
            "got {err:?}"
        );
    }

    #[test]
    fn late_resume_after_completion_traps() {
        // Capturing the continuation in a class and resuming it *after*
        // `withCheckedContinuation` already returned is misuse and must trap,
        // not be silently accepted.
        let err = run(
            "final class Box { var c: CheckedContinuation<Int, Never>? = nil }\nfunc value(_ box: Box) async -> Int {\n  await withCheckedContinuation { cont in\n    box.c = cont\n    cont.resume(returning: 1)\n  }\n}\nfunc run() async {\n  let box = Box()\n  let _ = await value(box)\n  box.c!.resume(returning: 2)\n}\nrun()\n",
        )
        .unwrap_err();
        assert!(
            matches!(err, EvalError::Trap(ref m) if m.contains("more than once")),
            "got {err:?}"
        );
    }

    #[test]
    fn continuation_does_not_drain_unrelated_pending_tasks() {
        // An earlier detached task must NOT be forced to run just because a
        // later continuation waits to be resolved: it stays pending until the
        // program's end-of-scope drain, printing after the continuation result.
        let out = run(
            "func value() async -> Int {\n  await withCheckedContinuation { cont in\n    Task { cont.resume(returning: 5) }\n  }\n}\nfunc run() async {\n  Task { print(\"unrelated\") }\n  print(\"value \\(await value())\")\n}\nrun()\n",
        )
        .unwrap();
        assert_eq!(out, "value 5\nunrelated\n");
    }

    #[test]
    fn for_loop_gives_each_iteration_a_fresh_binding() {
        // A task created per iteration must capture *that* iteration's value.
        let out = run(
            "func run() async {\n  await withTaskGroup(of: Int.self) { group in\n    for i in 1...3 { group.addTask { i } }\n    var total = 0\n    for await r in group { total += r }\n    print(total)\n  }\n}\nrun()\n",
        )
        .unwrap();
        assert_eq!(out, "6\n");
    }

    #[test]
    fn custom_sequence_algorithm_traps_when_materialization_limit_is_exceeded() {
        let err = run(
            "struct Naturals: Sequence, IteratorProtocol {\n  var n: Int\n  mutating func next() -> Int? { n += 1; return n }\n  func makeIterator() -> Naturals { self }\n}\nlet xs = Naturals(n: 0).map { $0 }\nprint(xs.count)\n",
        )
        .unwrap_err();
        assert!(
            matches!(err, EvalError::Trap(ref msg) if msg.contains("custom sequence algorithm exceeded")),
            "got {err:?}"
        );
    }
}
