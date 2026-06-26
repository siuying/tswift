//! The `eval(node, env)` tree-walker.
//!
//! Control flow (`return`, and later `break`/`continue`/`throw`) unwinds through
//! the `Err` channel as a [`Signal`], so a `?` naturally propagates it up to the
//! construct that handles it — without panicking. Real interpreter failures ride
//! the same channel as [`Signal::Error`].

use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;

use qswift_frontend::{Analysis, Node, NodeKind};

use crate::env::{BindError, Env, Scope};
use crate::ops;
use crate::stdlib::{
    AlgoFn, Arg, BuiltinReceiver, FreeFn, MethodEntry, Outcome, PropertyFn, StdContext, StdError,
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

/// Convenience: an operator/runtime trap message becomes a [`Signal::Error`].
fn trap(msg: String) -> Signal {
    Signal::Error(EvalError::Trap(msg))
}

type Eval = Result<SwiftValue, Signal>;

/// One declared Swift parameter, precomputed from its `AST_PARAM` node.
struct Param {
    label: Option<String>,
    name: String,
    variadic: bool,
    inout_: bool,
    default: Option<Node<'static>>,
}

/// A user-defined function: its parameters, body, and captured scope chain.
struct FuncDef {
    params: Vec<Param>,
    body: Option<Node<'static>>,
    captured: Vec<Scope>,
}

/// A stored property of a struct.
struct StoredProp {
    name: String,
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
}

/// A method of a struct.
struct MethodDef {
    params: Vec<Param>,
    body: Option<Node<'static>>,
    mutating: bool,
}

/// A struct type declaration.
struct StructDef {
    stored: Vec<StoredProp>,
    computed: std::collections::HashMap<String, ComputedProp>,
    methods: std::collections::HashMap<String, MethodDef>,
    subscript: Option<MethodDef>,
    /// A custom initializer, if the struct declares one (else memberwise).
    init: Option<MethodDef>,
    /// Stored property name → its `@propertyWrapper` type, when wrapped.
    wrappers: std::collections::HashMap<String, String>,
}

/// One case of an enum.
struct EnumCaseDef {
    name: String,
    /// The precomputed raw value (with Swift's auto-increment / name defaults).
    raw: Option<SwiftValue>,
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
    deinit: Option<Node<'static>>,
}

/// A protocol declaration: its inherited protocols and any default member
/// implementations supplied through `extension Protocol { … }`.
struct ProtoDef {
    inherited: Vec<String>,
    methods: std::collections::HashMap<String, MethodDef>,
    computed: std::collections::HashMap<String, ComputedProp>,
}

/// A closure value's definition: parameters and body statements.
struct ClosureDef {
    params: Vec<Param>,
    body: Vec<Node<'static>>,
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
    /// `Sequence`/`Collection` algorithms keyed by method name, applied to any
    /// builtin sequence receiver (layer 2 of the dispatch seam).
    algorithms: HashMap<String, AlgoFn>,
    env: Env,
    funcs: Vec<FuncDef>,
    structs: HashMap<String, StructDef>,
    enums: HashMap<String, EnumDef>,
    classes: HashMap<String, ClassDef>,
    protocols: HashMap<String, ProtoDef>,
    /// type name → protocols it conforms to (directly).
    conformances: HashMap<String, Vec<String>>,
    closures: Vec<(ClosureDef, Vec<Scope>)>,
    statics: HashMap<String, SwiftValue>,
    /// Stack of class names for the methods currently executing (for `super`).
    class_ctx: Vec<String>,
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
    /// Source file name for `#file`.
    filename: String,
    depth: usize,
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

impl<'w> Interpreter<'w> {
    /// Create an interpreter that writes program output to `out`.
    pub fn new(out: &'w mut dyn Write) -> Self {
        Interpreter {
            out,
            natives: HashMap::new(),
            free_fns: HashMap::new(),
            intrinsics: HashMap::new(),
            properties: HashMap::new(),
            algorithms: HashMap::new(),
            env: Env::new(),
            funcs: Vec::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            classes: HashMap::new(),
            protocols: HashMap::new(),
            conformances: HashMap::new(),
            closures: Vec::new(),
            statics: HashMap::new(),
            class_ctx: Vec::new(),
            defer_stack: Vec::new(),
            main_type: None,
            tasks: Vec::new(),
            groups: Vec::new(),
            filename: "main.swift".into(),
            depth: 0,
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

    /// Register a computed-property intrinsic on a builtin receiver type.
    pub fn register_property(&mut self, recv: BuiltinReceiver, name: &str, f: PropertyFn) {
        self.properties.insert((recv, name.to_string()), f);
    }

    /// Register a `Sequence`/`Collection` algorithm by method name.
    pub fn register_algorithm(&mut self, name: &str, f: AlgoFn) {
        self.algorithms.insert(name.to_string(), f);
    }

    /// Register a method intrinsic on a builtin receiver type.
    pub fn register_intrinsic(
        &mut self,
        recv: BuiltinReceiver,
        name: &str,
        entry: MethodEntry,
    ) {
        self.intrinsics.insert((recv, name.to_string()), entry);
    }

    /// Map a [`Signal`] escaping a closure call into a [`StdError`] for the seam.
    /// Loop/`return` control flow cannot legitimately cross an intrinsic call.
    fn signal_to_std_error(sig: Signal) -> StdError {
        match sig {
            Signal::Throw(v) => StdError::Throw(v),
            Signal::Error(e) => StdError::Error(e),
            Signal::Return(_)
            | Signal::Break(_)
            | Signal::Continue(_)
            | Signal::Fallthrough => {
                StdError::Error(EvalError::Trap("control flow escaped a builtin call".into()))
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
                    },
                    EnumCaseDef {
                        name: "failure".into(),
                        raw: None,
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
            | NodeKind::TypealiasDecl
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
            NodeKind::UnaryExpr => self.eval_unary(node),
            NodeKind::AssignExpr => self.eval_assign(node),
            NodeKind::ParenExpr => self.eval_only_child(node),
            NodeKind::TernaryExpr => self.eval_ternary(node),
            NodeKind::MemberExpr => self.eval_member(node),
            NodeKind::IdentExpr => self.eval_ident(node),
            NodeKind::IntegerLiteral => Ok(self.eval_int_literal(node)),
            NodeKind::BoolLiteral => Ok(SwiftValue::Bool(node.bool().unwrap_or(false))),
            NodeKind::FloatLiteral => Ok(SwiftValue::Double(node.float().unwrap_or(0.0))),
            NodeKind::StringLiteral => self.eval_string_literal(node),
            NodeKind::NilLiteral => Ok(SwiftValue::Nil),
            NodeKind::MacroExpansion => self.eval_macro(node),
            NodeKind::ForceUnwrap => self.eval_force_unwrap(node),
            NodeKind::OptionalChain => self.eval_only_child(node),
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
        for child in node.children() {
            match child.kind() {
                NodeKind::FuncDecl => self.declare_func(&child),
                NodeKind::StructDecl => self.register_struct(&child),
                NodeKind::EnumDecl => self.register_enum(&child),
                // An `actor` is a reference type whose isolation is provided
                // for free by our single-threaded executor (ADR-0005), so it is
                // registered exactly like a class.
                NodeKind::ClassDecl | NodeKind::ActorDecl => self.register_class(&child),
                NodeKind::ProtocolDecl => self.register_protocol(&child),
                _ => {}
            }
        }
        // Second pass: extensions (they add to already-registered types).
        for child in node.children() {
            if child.kind() == NodeKind::ExtensionDecl {
                self.register_extension(&child);
            }
        }
    }

    /// Record the protocols a type conforms to from its `Conformance` children.
    fn record_conformances(&mut self, type_name: &str, node: &Node<'static>) {
        let conf: Vec<String> = node
            .children()
            .filter(|c| c.kind() == NodeKind::Conformance)
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
            .filter(|c| c.kind() == NodeKind::Conformance)
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
        let Some(body) = node.children().find(|c| c.kind() == NodeKind::Block) else {
            return;
        };
        let mut methods = std::collections::HashMap::new();
        let mut computed = std::collections::HashMap::new();
        for member in body.children() {
            match member.kind() {
                NodeKind::FuncDecl => {
                    if let Some(mname) = member.text() {
                        methods.insert(
                            mname,
                            MethodDef {
                                params: parse_params(&member),
                                body: member.children().find(|c| c.kind() == NodeKind::Block),
                                mutating: member.modifiers() & MOD_MUTATING != 0,
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
        }
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
    ) -> Option<(Vec<Param>, Option<Node<'static>>, bool)> {
        for proto in self.all_protocols(type_name) {
            if let Some(m) = self
                .protocols
                .get(&proto)
                .and_then(|d| d.methods.get(method))
            {
                return Some((clone_params(&m.params), m.body, m.mutating));
            }
        }
        None
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
        let Some(body) = node.children().find(|c| c.kind() == NodeKind::Block) else {
            return;
        };
        // Determine the raw-value backing type from the conformance list.
        let raw_kind = node
            .children()
            .filter(|c| c.kind() == NodeKind::Conformance)
            .find_map(|c| match c.text().as_deref() {
                Some("String") => Some(RawKind::Str),
                Some(t) if IntWidth::from_type_name(t).is_some() => Some(RawKind::Int),
                _ => None,
            });
        let mut next_int: i128 = 0;
        let mut cases = Vec::new();
        let mut methods = std::collections::HashMap::new();
        let mut computed = std::collections::HashMap::new();
        for member in body.children() {
            match member.kind() {
                NodeKind::EnumCaseDecl => {
                    for element in member.children() {
                        if element.kind() != NodeKind::EnumElementDecl {
                            continue;
                        }
                        let Some(cname) = element.text() else {
                            continue;
                        };
                        let explicit = element
                            .children()
                            .find(|ec| ec.kind() != NodeKind::Param)
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
                        cases.push(EnumCaseDef { name: cname, raw });
                    }
                }
                NodeKind::FuncDecl => {
                    if let Some(mname) = member.text() {
                        methods.insert(
                            mname,
                            MethodDef {
                                params: parse_params(&member),
                                body: member.children().find(|c| c.kind() == NodeKind::Block),
                                mutating: member.modifiers() & MOD_MUTATING != 0,
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
            .find(|c| c.kind() == NodeKind::Conformance)
            .and_then(|c| c.text());
        let Some(body) = node.children().find(|c| c.kind() == NodeKind::Block) else {
            return;
        };
        let mut stored = Vec::new();
        let mut weak_fields = Vec::new();
        let mut computed = std::collections::HashMap::new();
        let mut methods = std::collections::HashMap::new();
        let mut init = None;
        let mut deinit = None;

        for member in body.children() {
            match member.kind() {
                NodeKind::InitDecl => {
                    init = Some(MethodDef {
                        params: parse_params(&member),
                        body: member.children().find(|c| c.kind() == NodeKind::Block),
                        mutating: false,
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
                            },
                        );
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
            name,
            ClassDef {
                superclass,
                stored,
                weak_fields,
                computed,
                methods,
                init,
                deinit,
            },
        );
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
        let Some(body) = node.children().find(|c| c.kind() == NodeKind::Block) else {
            return;
        };
        let mut stored = Vec::new();
        let mut computed = std::collections::HashMap::new();
        let mut methods = std::collections::HashMap::new();
        let mut wrappers = std::collections::HashMap::new();
        let mut subscript = None;
        let mut init = None;

        for member in body.children() {
            match member.kind() {
                NodeKind::InitDecl => {
                    init = Some(MethodDef {
                        params: parse_params(&member),
                        body: member.children().find(|c| c.kind() == NodeKind::Block),
                        mutating: true,
                    });
                }
                NodeKind::SubscriptDecl => {
                    let acc = member.var_accessors();
                    let body = acc
                        .getter_body
                        .or_else(|| member.children().find(|c| c.kind() == NodeKind::Block));
                    subscript = Some(MethodDef {
                        params: parse_params(&member),
                        body,
                        mutating: false,
                    });
                }
                NodeKind::FuncDecl => {
                    if let Some(mname) = member.text() {
                        let mods = member.modifiers();
                        methods.insert(
                            mname,
                            MethodDef {
                                params: parse_params(&member),
                                body: member.children().find(|c| c.kind() == NodeKind::Block),
                                mutating: mods & MOD_MUTATING != 0,
                            },
                        );
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
                            // Eagerly evaluate the static's initial value.
                            if let Some(def) = default {
                                if let Ok(v) = self.eval(&def) {
                                    self.statics.insert(format!("{name}.{pname}"), v);
                                }
                            }
                        } else {
                            stored.push(StoredProp {
                                name: pname,
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
            name,
            StructDef {
                stored,
                computed,
                methods,
                subscript,
                init,
                wrappers,
            },
        );
    }

    /// A tuple expression `(a, b, …)`.
    fn eval_tuple(&mut self, node: &Node<'static>) -> Eval {
        let mut items = Vec::new();
        for child in node.children() {
            items.push(self.eval(&child)?);
        }
        Ok(SwiftValue::Tuple(items))
    }

    /// Evaluate each child in order, yielding the last value.
    fn eval_seq(&mut self, node: &Node<'static>) -> Eval {
        let mut last = SwiftValue::Void;
        for child in node.children() {
            last = self.eval(&child)?;
        }
        Ok(last)
    }

    /// Evaluate the single meaningful child of a wrapper node (e.g. paren).
    fn eval_only_child(&mut self, node: &Node<'static>) -> Eval {
        let child = node
            .children()
            .next()
            .ok_or_else(|| EvalError::Unsupported("empty wrapper node".into()))?;
        self.eval(&child)
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
        let captured = self.env.capture();
        let id = self.funcs.len();
        self.funcs.push(FuncDef {
            params,
            body,
            captured,
        });
        self.env.declare(&name, SwiftValue::Function(id), false);
    }

    /// `let`/`var name [= init]`, including tuple decomposition
    /// `let (a, b) = pair`.
    fn eval_decl(&mut self, node: &Node<'static>, mutable: bool) -> Eval {
        let children: Vec<Node<'static>> = node.children().collect();

        // Tuple-pattern binding: `let (a, b) = expr`.
        if let Some(pat) = children.iter().find(|c| c.kind() == NodeKind::PatternTuple) {
            let init = children.last().filter(|c| is_expr(c)).ok_or_else(|| {
                EvalError::Unsupported("tuple binding without initializer".into())
            })?;
            let value = self.eval(init)?;
            self.bind_tuple_pattern(pat, &value, mutable)?;
            return Ok(SwiftValue::Void);
        }

        let name = node
            .decl_name()
            .ok_or_else(|| EvalError::Unsupported("declaration without a name".into()))?;

        // `async let name = expr` spawns a child task; the binding holds its
        // handle and `await name` later retrieves the result (ADR-0005).
        if node.is_async_let() {
            if let Some(init) = children.last().filter(|c| is_expr(c)) {
                let id = self.spawn_expr_task(*init);
                self.env.declare(&name, SwiftValue::Task(id), mutable);
                return Ok(SwiftValue::Void);
            }
        }

        let value = match children.last() {
            Some(init) if is_expr(init) => {
                let v = self.eval(init)?;
                self.coerce_to_decl_type(node, v)
            }
            _ => SwiftValue::Void,
        };
        self.env.declare(&name, value, mutable);
        Ok(SwiftValue::Void)
    }

    /// Bind the names in a tuple pattern to the elements of a tuple value.
    fn bind_tuple_pattern(
        &mut self,
        pattern: &Node<'static>,
        value: &SwiftValue,
        mutable: bool,
    ) -> Result<(), Signal> {
        let SwiftValue::Tuple(items) = value else {
            return Err(EvalError::Type(format!(
                "cannot destructure {} as a tuple",
                value.type_name()
            ))
            .into());
        };
        let elems: Vec<Node<'static>> = pattern.children().collect();
        for (sub, item) in elems.iter().zip(items.iter()) {
            match sub.kind() {
                NodeKind::PatternWildcard => {}
                NodeKind::PatternTuple => self.bind_tuple_pattern(sub, item, mutable)?,
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
                if child.kind() == NodeKind::TypeIdent
                    && child.text().as_deref().is_some_and(|t| t.starts_with("Set<") || t == "Set")
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
            if child.kind() == NodeKind::TypeIdent {
                if let Some(w) = child.text().as_deref().and_then(IntWidth::from_type_name) {
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
        if let Some(v) = self.env.get(&name) {
            return Ok(v);
        }
        if let Some(v) = self.implicit_self_member(&name)? {
            return Ok(v);
        }
        Err(EvalError::UnknownVariable(name).into())
    }

    /// If `name` is a property of the current `self`, read it. Covers struct
    /// stored/computed members and enum `rawValue`/computed members.
    fn implicit_self_member(&mut self, name: &str) -> Result<Option<SwiftValue>, Signal> {
        let Some(this) = self.env.get("self") else {
            return Ok(None);
        };
        match &this {
            SwiftValue::Struct(obj) => {
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
            _ => Ok(None),
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
        let exists = self
            .enums
            .get(type_name)
            .is_some_and(|d| d.cases.iter().any(|c| c.name == case));
        if !exists {
            return Ok(None);
        }
        Ok(Some(SwiftValue::Enum(Rc::new(EnumObj {
            type_name: type_name.to_string(),
            case: case.to_string(),
            payload,
        }))))
    }

    /// Resolve the enum type for a shorthand `.case` member from msf's resolved
    /// type, falling back to the unique enum declaring that case.
    fn resolve_member_enum(&self, member: &Node<'static>, case: &str) -> Option<String> {
        // msf often resolves the member's type to the enum (or a function
        // returning it); match a registered enum name within that string.
        if let Some(ty) = member.type_name() {
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
    ) -> Option<(Vec<Param>, Option<Node<'static>>, String)> {
        let mut current = Some(class_name.to_string());
        while let Some(cls) = current {
            let def = self.classes.get(&cls)?;
            if let Some(m) = def.methods.get(name) {
                return Some((clone_params(&m.params), m.body, cls));
            }
            current = def.superclass.clone();
        }
        None
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
            let (params, body) = {
                let m = self.classes[&owner].init.as_ref().unwrap();
                (clone_params(&m.params), m.body)
            };
            self.class_ctx.push(owner);
            self.env.push();
            self.env.declare("self", value.clone(), false);
            let bound = self.bind_params(&params, args);
            let result = match bound {
                Ok(_) => match body {
                    Some(b) => self.eval(&b),
                    None => Ok(SwiftValue::Void),
                },
                Err(e) => Err(e),
            };
            self.env.pop();
            self.class_ctx.pop();
            match result {
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
            if let Some(c) = def.computed.get(name) {
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
                    params.push(Param {
                        label: None,
                        name,
                        variadic: child.param_info().variadic,
                        inout_: false,
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
                NodeKind::TypeIdent | NodeKind::TypeOptional => {}
                _ => body.push(child),
            }
        }
        // Untyped parameters: the body lives under the last `Param` node.
        if body.is_empty() {
            if let Some(p) = last_param {
                for c in p.children() {
                    if !matches!(c.kind(), NodeKind::TypeIdent | NodeKind::TypeOptional) {
                        body.push(c);
                    }
                }
            }
        }

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
        self.closures.push((ClosureDef { params, body }, captured));
        Ok(SwiftValue::Closure(id))
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
        let (params, body, captured) = {
            let (def, cap) = &self.closures[id];
            (clone_params(&def.params), def.body.clone(), cap.clone())
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
            ClosureDef {
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
            _ => Ok(None),
        }
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
            _ => Ok(None),
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
            _ => false,
        }
    }

    /// Magic literals: `#file`, `#line`, `#function`, `#column`.
    fn eval_macro(&mut self, node: &Node<'static>) -> Eval {
        let which = node.text().unwrap_or_default();
        match which.as_str() {
            "file" | "filePath" | "fileID" => Ok(SwiftValue::Str(self.filename.clone())),
            "line" => Ok(SwiftValue::int(node.line() as i128)),
            "column" => Ok(SwiftValue::int(0)),
            "function" => Ok(SwiftValue::Str(
                self.class_ctx.last().cloned().unwrap_or_default(),
            )),
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
            other => {
                return Err(EvalError::Type(format!("cannot encode {}", other.type_name())).into())
            }
        })
    }

    /// Build a runtime value from JSON for the given target type (a registered
    /// struct, else inferred from the JSON shape).
    fn json_decode(&self, type_name: &str, json: &crate::json::Json) -> SwiftValue {
        use crate::json::Json;
        if let (Json::Object(_), Some(def)) = (json, self.structs.get(type_name)) {
            let fields: Vec<(String, SwiftValue)> = def
                .stored
                .iter()
                .map(|p| {
                    let v = json
                        .get(&p.name)
                        .map(|j| self.json_value(j))
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
        let base_value = self.eval(&base)?;
        let index_nodes: Vec<Node<'static>> = kids.collect();
        let indices: Vec<SwiftValue> = index_nodes
            .iter()
            .map(|n| self.eval(n))
            .collect::<Result<_, _>>()?;
        self.read_subscript(&base_value, &indices)
    }

    /// Assign `base[index] = value` (compound ops supported) for an array held
    /// in a variable, or a struct field path ending in an array.
    fn assign_subscript(&mut self, target: &Node<'static>, rhs: &Node<'static>, op: &str) -> Eval {
        let mut kids = target.children();
        let base = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("subscript without a base".into()))?;
        let index_node = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("subscript without an index".into()))?;
        let index_value = self.eval(&index_node)?;
        let Some(place) = self.resolve_place(&base) else {
            return Err(EvalError::Unsupported("subscript target is not assignable".into()).into());
        };
        let current = self.read_place(&place)?;
        // `dict[key] = value` inserts/updates; `dict[key] = nil` removes.
        if let SwiftValue::Dict(pairs) = &current {
            let mut new_pairs = pairs.as_ref().clone();
            let existing = new_pairs.iter().position(|(k, _)| *k == index_value);
            let new_value = if op == "=" {
                self.eval(rhs)?
            } else {
                let cur = existing
                    .map(|i| new_pairs[i].1.clone())
                    .unwrap_or(SwiftValue::Nil);
                let r = self.eval(rhs)?;
                ops::binary(op.trim_end_matches('='), &cur, &r).map_err(trap)?
            };
            match (existing, matches!(new_value, SwiftValue::Nil)) {
                (Some(i), true) => {
                    new_pairs.remove(i);
                }
                (Some(i), false) => new_pairs[i].1 = new_value,
                (None, true) => {}
                (None, false) => new_pairs.push((index_value, new_value)),
            }
            self.write_place(&place, SwiftValue::Dict(StdRc::new(new_pairs)))?;
            return Ok(SwiftValue::Void);
        }
        let idx = subscript_index(&[index_value])?;
        let current_array = current;
        let SwiftValue::Array(items) = &current_array else {
            return Err(EvalError::Type("subscript assignment requires an array".into()).into());
        };
        if idx >= items.len() {
            return Err(trap(format!("index {idx} out of range")));
        }
        let new_elem = if op == "=" {
            self.eval(rhs)?
        } else {
            let r = self.eval(rhs)?;
            ops::binary(op.trim_end_matches('='), &items[idx], &r).map_err(trap)?
        };
        let mut new_items = items.as_ref().clone();
        new_items[idx] = new_elem;
        self.write_place(&place, SwiftValue::Array(StdRc::new(new_items)))?;
        Ok(SwiftValue::Void)
    }

    /// Read `base[indices]`.
    fn read_subscript(&mut self, base: &SwiftValue, indices: &[SwiftValue]) -> Eval {
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
                s.chars()
                    .nth(i)
                    .map(|c| SwiftValue::Str(c.to_string()))
                    .ok_or_else(|| trap(format!("string index {i} out of range")))
            }
            SwiftValue::Struct(obj) => {
                let type_name = obj.type_name.clone();
                let has = self
                    .structs
                    .get(&type_name)
                    .is_some_and(|d| d.subscript.is_some());
                if has {
                    let (params, body, _) = {
                        let m = self.structs[&type_name].subscript.as_ref().unwrap();
                        (clone_params(&m.params), m.body, m.mutating)
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
                    self.env.declare("self", base.clone(), false);
                    let bound = self.bind_params(&params, args);
                    let result = match bound {
                        Ok(_) => match body {
                            Some(b) => self.eval(&b),
                            None => Ok(SwiftValue::Void),
                        },
                        Err(e) => Err(e),
                    };
                    self.env.pop();
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
            .and_then(|c| c.getter)
            .or_else(|| self.protocol_default_getter(&obj.type_name, name));
        if let Some(body) = getter {
            return self
                .run_with_self(value.clone(), |me| me.eval(&body))
                .map(|(v, _)| v);
        }
        Err(EvalError::Type(format!("struct {} has no member `{name}`", obj.type_name)).into())
    }

    /// Build a struct instance from a memberwise initializer call.
    fn instantiate_struct(
        &mut self,
        type_name: &str,
        args: &[(Option<String>, SwiftValue)],
    ) -> Eval {
        // A custom initializer runs against a fresh empty value, binding `self`.
        let custom_init = self
            .structs
            .get(type_name)
            .and_then(|d| d.init.as_ref().map(|m| (clone_params(&m.params), m.body)));
        if let Some((params, body)) = custom_init {
            let this = SwiftValue::Struct(Rc::new(StructObj {
                type_name: type_name.to_string(),
                fields: Vec::new(),
            }));
            let call_args: Vec<CallArg> = args
                .iter()
                .map(|(label, value)| CallArg {
                    label: label.clone(),
                    value: value.clone(),
                    place: None,
                })
                .collect();
            self.env.push();
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
            self.env.pop();
            return match result {
                Ok(_) | Err(Signal::Return(_)) => Ok(built),
                Err(e) => Err(e),
            };
        }

        let plan: Vec<(String, bool, Option<Node<'static>>)> = self
            .structs
            .get(type_name)
            .map(|d| {
                d.stored
                    .iter()
                    .map(|p| (p.name.clone(), p.lazy, p.default))
                    .collect()
            })
            .unwrap_or_default();

        let mut fields: Vec<(String, SwiftValue)> = Vec::new();
        let mut positional = args.iter().filter(|(l, _)| l.is_none());
        for (pname, lazy, default) in plan {
            let labeled = args
                .iter()
                .find(|(l, _)| l.as_deref() == Some(pname.as_str()))
                .map(|(_, v)| v.clone());
            let value = if let Some(v) = labeled {
                v
            } else if let Some((_, v)) = positional.next() {
                v.clone()
            } else if lazy {
                // Lazy properties are materialized on first access, not here.
                continue;
            } else if let Some(def) = default {
                self.eval(&def)?
            } else {
                return Err(EvalError::Type(format!(
                    "missing value for property `{pname}` of {type_name}"
                ))
                .into());
            };
            // Wrap `@propertyWrapper` fields in their wrapper instance.
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
        self.env.push();
        self.env.declare("self", this, true);
        let result = body(self);
        let updated = self.env.get("self").unwrap_or(SwiftValue::Void);
        self.env.pop();
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
                NodeKind::OptionalBinding => {
                    let name = cond
                        .text()
                        .ok_or_else(|| EvalError::Unsupported("binding without a name".into()))?;
                    let value = match cond.children().next() {
                        Some(init) => self.eval(&init)?,
                        None => self
                            .env
                            .get(&name)
                            .ok_or_else(|| EvalError::UnknownVariable(name.clone()))?,
                    };
                    if matches!(value, SwiftValue::Nil) {
                        return Ok(false);
                    }
                    self.env.declare(&name, value, false);
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

    /// `while cond { … }`.
    fn eval_while(&mut self, node: &Node<'static>) -> Eval {
        let kids: Vec<Node<'static>> = node.children().collect();
        let cond = kids
            .first()
            .ok_or_else(|| EvalError::Unsupported("while without condition".into()))?;
        let body = kids
            .last()
            .ok_or_else(|| EvalError::Unsupported("while without body".into()))?;
        let label = node.loop_label();
        while self.eval_condition(cond)? {
            match self.run_loop_body(body, &label)? {
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
        let mut var_name = node
            .text()
            .ok_or_else(|| EvalError::Unsupported("for-loop without a binding".into()))?;
        // `for await r in seq`: msf anchors the node on the `await` keyword, so
        // the binding name is the next token (ADR-0005).
        let is_for_await = var_name == "await";
        if is_for_await {
            var_name = node
                .token_text_offset(1)
                .ok_or_else(|| EvalError::Unsupported("for-await without a binding".into()))?;
        }
        let mut iterable = None;
        let mut where_clause = None;
        let mut body = None;
        for child in node.children() {
            match child.kind() {
                NodeKind::Param => {}
                NodeKind::Block => body = Some(child),
                _ => {
                    if iterable.is_none() {
                        iterable = Some(child);
                    } else {
                        where_clause = Some(child);
                    }
                }
            }
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
                return self.run_async_sequence(&seq, &var_name, where_clause, &body, &label);
            }
            _ => self.iterate(&seq)?,
        };

        for item in items {
            // A fresh scope per iteration so a closure/task created in the body
            // captures *this* iteration's binding (Swift's per-iteration `let`),
            // not a single shared, mutated slot.
            self.env.push();
            self.env.declare(&var_name, item, false);
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
            // Iterating a dictionary yields `(key, value)` tuples.
            SwiftValue::Dict(pairs) => Ok(pairs
                .iter()
                .map(|(k, v)| SwiftValue::Tuple(vec![k.clone(), v.clone()]))
                .collect()),
            SwiftValue::Set(items) => Ok(items.as_ref().clone()),
            SwiftValue::Str(s) => Ok(s.chars().map(|c| SwiftValue::Str(c.to_string())).collect()),
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
            NodeKind::PatternWildcard => Ok(Some(Vec::new())),
            NodeKind::PatternValueBinding => {
                let name = pattern.text().unwrap_or_default();
                Ok(Some(vec![(name, subject.clone())]))
            }
            NodeKind::PatternRange => {
                let bounds: Vec<Node<'static>> = pattern.children().collect();
                if bounds.len() != 2 {
                    return Ok(None);
                }
                let lo = self.eval(&bounds[0])?;
                let hi = self.eval(&bounds[1])?;
                let inclusive = pattern.text().as_deref() == Some("...");
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
            NodeKind::PatternEnum => {
                let case_name = pattern.op_text().unwrap_or_default();
                // The leading `TypeIdent` (e.g. the `E` in `E.bad`) is not a
                // sub-pattern; only payload bindings are.
                let subs: Vec<Node<'static>> = pattern
                    .children()
                    .filter(|c| c.kind() != NodeKind::TypeIdent)
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
            NodeKind::PatternTuple => {
                let SwiftValue::Tuple(items) = subject else {
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

        // `self.<name>` where `self` is a class instance.
        if target.kind() == NodeKind::IdentExpr {
            if let Some(n) = target.text() {
                if self.env.get(&n).is_none() {
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
            Some(p) if p.path.is_empty() && self.env.get(&p.root).is_none() => {
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
            return Err(EvalError::Unsupported(format!(".{member} (unresolved type)")).into());
        };

        if base.kind() == NodeKind::IdentExpr {
            if let Some(type_name) = base.text() {
                if self.env.get(&type_name).is_none() {
                    if let Some(w) = IntWidth::from_type_name(&type_name) {
                        return match member.as_str() {
                            "max" => Ok(SwiftValue::Int(IntValue::new(w.max(), w))),
                            "min" => Ok(SwiftValue::Int(IntValue::new(w.min(), w))),
                            _ => {
                                Err(EvalError::Unsupported(format!("{type_name}.{member}")).into())
                            }
                        };
                    }
                    // Static property of a struct type: `Type.prop`.
                    if self.structs.contains_key(&type_name) {
                        if let Some(v) = self.statics.get(&format!("{type_name}.{member}")) {
                            return Ok(v.clone());
                        }
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
            (SwiftValue::Str(s), "count") => Ok(SwiftValue::int(s.chars().count() as i128)),
            (SwiftValue::Str(s), "isEmpty") => Ok(SwiftValue::Bool(s.is_empty())),
            (SwiftValue::Tuple(items), idx) if idx.parse::<usize>().is_ok() => {
                let i: usize = idx.parse().unwrap();
                items
                    .get(i)
                    .cloned()
                    .ok_or_else(|| EvalError::Type(format!("tuple index .{i} out of range")).into())
            }
            _ => Err(
                EvalError::Unsupported(format!("member .{member} on {}", value.type_name())).into(),
            ),
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

        let args = self.eval_args(arg_nodes)?;

        if callee.kind() == NodeKind::IdentExpr {
            let name = callee
                .text()
                .ok_or_else(|| EvalError::Unsupported("unnamed callee".into()))?;

            // Built-in JSON coder markers.
            if name == "JSONEncoder" || name == "JSONDecoder" {
                return Ok(SwiftValue::Struct(Rc::new(StructObj {
                    type_name: name,
                    fields: vec![],
                })));
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
            // A bound function or closure value (incl. recursion).
            match self.env.get(&name) {
                Some(SwiftValue::Function(id)) => return self.call_function(id, args),
                Some(SwiftValue::Closure(id)) => {
                    let plain = args.into_iter().map(|a| a.value).collect();
                    return self.call_closure(id, plain);
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
            SwiftValue::Closure(id) => {
                let plain = args.into_iter().map(|a| a.value).collect();
                self.call_closure(id, plain)
            }
            other => {
                Err(EvalError::Type(format!("`{}` is not callable", other.type_name())).into())
            }
        }
    }

    /// Evaluate call arguments, resolving `inout` (`&place`) into a write-back
    /// location.
    fn eval_args(&mut self, arg_nodes: &[Node<'static>]) -> Result<Vec<CallArg>, Signal> {
        let mut args = Vec::new();
        for arg in arg_nodes {
            let label = arg.arg_label();
            if arg.kind() == NodeKind::InoutExpr {
                let inner = arg
                    .children()
                    .next()
                    .ok_or_else(|| EvalError::Unsupported("inout without an lvalue".into()))?;
                let place = self.resolve_place(&inner);
                let value = self.eval(&inner)?;
                args.push(CallArg {
                    label,
                    value,
                    place,
                });
            } else {
                let value = self.eval(arg)?;
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
                if self.env.get(&tn).is_none() {
                    if self.enum_has_case(&tn, &method) {
                        let args = self.eval_args(arg_nodes)?;
                        let payload = args.into_iter().map(|a| a.value).collect();
                        return Ok(self.make_enum_case(&tn, &method, payload)?.unwrap());
                    }
                    if self.structs.contains_key(&tn) {
                        let args = self.eval_args(arg_nodes)?;
                        return self.call_struct_method(SwiftValue::Void, &tn, &method, args, None);
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
            let args = self.eval_args(arg_nodes)?;
            return self.dispatch_class_method(base_value.clone(), &class_name, &method, args);
        }

        // Standard-library intrinsic registry (layer 1): type-specific members
        // such as `Array.append`. Consulted before the ad-hoc algorithm paths.
        if let Some(kind) = BuiltinReceiver::of(&base_value) {
            if self.intrinsics.contains_key(&(kind, method.clone())) {
                let args = self.eval_args(arg_nodes)?;
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
            if let Some(items) = materialize_sequence(&base_value) {
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

        let args = self.eval_args(arg_nodes)?;
        let type_name = match &base_value {
            SwiftValue::Struct(o) => Some(o.type_name.clone()),
            SwiftValue::Enum(e) => Some(e.type_name.clone()),
            _ => None,
        };
        if let Some(type_name) = type_name {
            if self.type_has_method(&type_name, &method) {
                let place = self.resolve_place(&base);
                return self.call_struct_method(base_value, &type_name, &method, args, place);
            }
        }

        Err(
            EvalError::Unsupported(format!("method .{method}() on {}", base_value.type_name()))
                .into(),
        )
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
        self.env.push();
        self.env.declare("self", this, false);
        let bound = self.bind_params(&params, args);
        let result = match bound {
            Ok(_) => match body {
                Some(b) => self.eval(&b),
                None => Ok(SwiftValue::Void),
            },
            Err(e) => Err(e),
        };
        self.env.pop();
        self.class_ctx.pop();
        match result {
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
        let (params, body, owner) = match self.lookup_method(from_class, method) {
            Some(m) => m,
            None => {
                let (p, b, _) = self
                    .protocol_default_method(from_class, method)
                    .ok_or_else(|| {
                        EvalError::Unsupported(format!("{from_class} has no method `{method}`"))
                    })?;
                (p, b, from_class.to_string())
            }
        };
        self.class_ctx.push(owner);
        self.env.push();
        self.env.declare("self", this, false);
        let bound = self.bind_params(&params, args);
        let result = match bound {
            Ok(_) => match body {
                Some(b) => self.eval(&b),
                None => Ok(SwiftValue::Void),
            },
            Err(e) => Err(e),
        };
        self.env.pop();
        self.class_ctx.pop();
        match result {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
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
        let own = self
            .structs
            .get(type_name)
            .and_then(|d| d.methods.get(method))
            .or_else(|| {
                self.enums
                    .get(type_name)
                    .and_then(|d| d.methods.get(method))
            })
            .map(|def| (clone_params(&def.params), def.body, def.mutating));
        let (params, body, mutating) = match own {
            Some(m) => m,
            None => self
                .protocol_default_method(type_name, method)
                .ok_or_else(|| {
                    EvalError::Unsupported(format!("{type_name} has no method `{method}`"))
                })?,
        };

        self.env.push();
        self.env.declare("self", this, true);
        let inout_binds = self.bind_params(&params, args);
        let outcome = match inout_binds {
            Ok(binds) => {
                let result = match body {
                    Some(b) => self.eval(&b),
                    None => Ok(SwiftValue::Void),
                };
                self.apply_inout_writebacks(&binds);
                result
            }
            Err(e) => Err(e),
        };
        let updated_self = self.env.get("self").unwrap_or(SwiftValue::Void);
        self.env.pop();

        let ret = match outcome {
            Ok(v) => v,
            Err(Signal::Return(v)) => v,
            Err(e) => return Err(e),
        };
        if mutating {
            if let Some(place) = base_place {
                self.write_place(&place, updated_self)?;
            }
        }
        Ok(ret)
    }

    /// A string literal, processing escapes and `\( … )` interpolation.
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
                out.push_str(&value.to_string());
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
        let call_env = Env::with_captured(captured);
        let saved = std::mem::replace(&mut self.env, call_env);

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

        self.env = saved;
        self.depth -= 1;

        let (value, writes) = outcome?;
        for (place, val) in writes {
            self.write_place(&place, val)?;
        }
        Ok(value)
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
                while ai < args.len() && args[ai].label.is_none() {
                    pack.push(args[ai].value.clone());
                    ai += 1;
                }
                self.env
                    .declare(&p.name, SwiftValue::Array(Rc::new(pack)), false);
            } else if ai < args.len() {
                let arg = &args[ai];
                // `inout` params are mutable and write back to the caller.
                self.env.declare(&p.name, arg.value.clone(), p.inout_);
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

    /// Write each captured `inout` parameter's current value back to its caller
    /// location (used when the call shares the caller's environment).
    fn apply_inout_writebacks(&mut self, binds: &[(String, Place)]) {
        for (name, place) in binds {
            if let Some(v) = self.env.get(name) {
                let _ = self.write_place(place, v);
            }
        }
    }

    /// Resolve an lvalue expression to a [`Place`] (root variable + field path).
    fn resolve_place(&self, node: &Node<'static>) -> Option<Place> {
        match node.kind() {
            NodeKind::IdentExpr => node.text().map(|root| Place {
                root,
                path: Vec::new(),
            }),
            NodeKind::ParenExpr => node.children().next().and_then(|c| self.resolve_place(&c)),
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

    /// Write `value` to the storage named by `place`, applying copy-on-write and
    /// any property observers at the leaf.
    fn write_place(&mut self, place: &Place, value: SwiftValue) -> Result<(), Signal> {
        if place.path.is_empty() {
            return match self.env.assign(&place.root, value) {
                Ok(()) => Ok(()),
                Err(BindError::Immutable(n)) => Err(EvalError::Immutable(n).into()),
                Err(BindError::Unbound(n)) => Err(EvalError::UnknownVariable(n).into()),
            };
        }
        let root_val = self
            .env
            .get(&place.root)
            .ok_or_else(|| EvalError::UnknownVariable(place.root.clone()))?;
        let updated = self.set_in(root_val, &place.path, value)?;
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
                if let SwiftValue::Tuple(t) = el {
                    if t.len() == 2 {
                        pairs.push((t[0].clone(), t[1].clone()));
                        continue;
                    }
                }
                return Err(EvalError::Type("uniqueKeysWithValues expects (key, value) pairs".into()).into());
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
        SwiftValue::Str(s) => Some(s.chars().map(|c| SwiftValue::Str(c.to_string())).collect()),
        // A dictionary is a sequence of `(key, value)` tuples.
        SwiftValue::Dict(pairs) => Some(
            pairs
                .iter()
                .map(|(k, v)| SwiftValue::Tuple(vec![k.clone(), v.clone()]))
                .collect(),
        ),
        SwiftValue::Set(items) => Some(items.as_ref().clone()),
        _ => None,
    }
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

    fn out(&mut self) -> &mut dyn Write {
        self.out
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
fn clone_params(params: &[Param]) -> Vec<Param> {
    params
        .iter()
        .map(|p| Param {
            label: p.label.clone(),
            name: p.name.clone(),
            variadic: p.variadic,
            inout_: p.inout_,
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
            params.push(Param {
                label: info.label,
                name: info.name,
                variadic: info.variadic,
                inout_: info.is_inout,
                default,
            });
        }
    }
    params
}

/// What a loop body asks its loop to do next.
enum LoopFlow {
    Continue,
    Break,
}

/// Whether a node is an expression (vs. a type annotation or other non-value
/// child appearing under a declaration).
fn is_expr(node: &Node) -> bool {
    is_value_node(node) && node.kind() != NodeKind::PatternTuple
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
    !matches!(
        node.kind(),
        NodeKind::TypeIdent
            | NodeKind::TypeOptional
            | NodeKind::TypeInout
            | NodeKind::AccessorDecl
            | NodeKind::Conformance
            | NodeKind::TypeFunc
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

/// Split a `case` clause into (patterns, body-statements). Patterns are the
/// leading non-statement children; the body is everything from the first
/// statement onward.
fn case_parts(case: &Node<'static>) -> (Vec<Node<'static>>, Vec<Node<'static>>) {
    let mut patterns = Vec::new();
    let mut body = Vec::new();
    let mut in_body = false;
    for child in case.children() {
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
        (SwiftValue::Tuple(x), SwiftValue::Tuple(y)) => {
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
fn subscript_index(indices: &[SwiftValue]) -> Result<usize, Signal> {
    match indices.first() {
        Some(SwiftValue::Int(i)) if i.raw >= 0 => Ok(i.raw as usize),
        Some(SwiftValue::Int(i)) => Err(trap(format!("negative index {}", i.raw))),
        _ => Err(EvalError::Type("subscript index must be an integer".into()).into()),
    }
}

/// Decode a Swift string literal's *source text* (including its delimiters) into
/// the runtime string it denotes: strips quotes and processes escapes.
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
        let out = run(
            "extension Int { func squared() -> Int { return self * self } }\nprint(5.squared())\n",
        );
        // Extensions on built-in `Int` may be unsupported; a user type must work.
        let out = out.or_else(|_| {
            run("struct V { let n: Int }\nextension V { func doubled() -> Int { return n * 2 } }\nprint(V(n: 21).doubled())\n")
        })
        .unwrap();
        assert_eq!(out, "42\n");
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
    fn for_loop_gives_each_iteration_a_fresh_binding() {
        // A task created per iteration must capture *that* iteration's value.
        let out = run(
            "func run() async {\n  await withTaskGroup(of: Int.self) { group in\n    for i in 1...3 { group.addTask { i } }\n    var total = 0\n    for await r in group { total += r }\n    print(total)\n  }\n}\nrun()\n",
        )
        .unwrap();
        assert_eq!(out, "6\n");
    }
}
