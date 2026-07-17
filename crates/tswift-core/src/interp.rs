//! The `eval(node, env)` tree-walker.
//!
//! Control flow (`return`, and later `break`/`continue`/`throw`) unwinds through
//! the `Err` channel as a [`Signal`], so a `?` naturally propagates it up to the
//! construct that handles it — without panicking. Real interpreter failures ride
//! the same channel as [`Signal::Error`].

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::rc::Rc;

use tswift_frontend::{Analysis, Node, NodeKind, TypeRepr};

use crate::env::{Env, Scope};
use crate::fragment_cache::FragmentCache;
use crate::ops;
use crate::stdlib::{
    materialize_builtin_sequence, AlgoFn, Arg, BuiltinReceiver, ContextualPropertyFn, FreeFn,
    LabeledMethodEntry, MethodEntry, PropertyFn, PropertySetterFn, StaticFn, StdContext, StdError,
    StructMethodFn, TypedPropertyFn,
};
use std::cell::RefCell;
use std::rc::Rc as StdRc;

use crate::value::{ClassObj, EnumObj, IntValue, IntWidth, StructObj, SwiftValue};

mod coding;
mod concurrency;
mod dispatch;
mod nominal;
mod pattern;
mod sequence;
mod storage;
mod strings;

use strings::max_shorthand_in_interpolations;

use self::concurrency::Scheduler;

/// Maximum nested Swift call depth before the interpreter traps, converting
/// unbounded recursion into a catchable error instead of a native stack
/// overflow.
const MAX_CALL_DEPTH: usize = 5000;

/// Identity of a framework / language module that owns registered symbols
/// (`"Swift"`, `"Foundation"`, `"SwiftUI"`, `"Charts"`, …).
///
/// Module system (ADR-0020): every registration is stamped with a [`ModuleId`].
/// Phase B resolves the name-only struct-method seam by the receiver's owning
/// module via [`Interpreter::type_module`] + per-module candidates. Phase C
/// records program `import`s. Phase D2 enables **strict import-gating** (default
/// on): a framework-module symbol is resolvable only when that module is in the
/// program's import set. Base `"Swift"` (stdlib) is always implicit.
///
/// Module names are static literals installed by frameworks, so the id is a
/// cheap `Copy` handle over `&'static str` rather than cloning a `String` into
/// every registry entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleId(&'static str);

impl ModuleId {
    /// The always-present base module (stdlib). Also the default stamp when no
    /// [`Interpreter::module`] scope is active.
    pub const SWIFT: ModuleId = ModuleId("Swift");
    /// Foundation framework module.
    pub const FOUNDATION: ModuleId = ModuleId("Foundation");
    /// SwiftData framework module.
    pub const SWIFT_DATA: ModuleId = ModuleId("SwiftData");
    /// SwiftUI framework module.
    pub const SWIFTUI: ModuleId = ModuleId("SwiftUI");
    /// Charts framework module.
    pub const CHARTS: ModuleId = ModuleId("Charts");

    /// Create a module id from a static display name
    /// (`"Swift"` / `"Foundation"` / `"SwiftData"` / `"SwiftUI"` / `"Charts"`).
    pub fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// The base / default module.
    pub fn swift() -> Self {
        Self::SWIFT
    }

    /// Borrow the module name.
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

impl Default for ModuleId {
    fn default() -> Self {
        Self::swift()
    }
}

impl std::fmt::Display for ModuleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for ModuleId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Map an import path's leading component (or a host-seeded name) to a
/// [`ModuleId`]. Known framework names reuse the constants; any other name is
/// interned once via `Box::leak` so it can live as a `Copy` id (import sets
/// are tiny — one entry per imported module per program).
fn module_id_for_import_name(name: &str) -> ModuleId {
    match name {
        "Swift" => ModuleId::SWIFT,
        "Foundation" => ModuleId::FOUNDATION,
        "SwiftData" => ModuleId::SWIFT_DATA,
        "SwiftUI" => ModuleId::SWIFTUI,
        "Charts" => ModuleId::CHARTS,
        other => ModuleId::new(Box::leak(other.to_string().into_boxed_str())),
    }
}

/// Leading component of an import path (`SwiftUI` from `SwiftUI` or
/// `SwiftUI.Foo`). Submodule / `.` tails are ignored (Phase C).
fn import_path_leading_component(path: &str) -> &str {
    path.split('.').next().unwrap_or(path)
}

/// A value stamped with the module that registered it. Lookups check
/// [`Interpreter::module_symbol_visible`] under strict import-gating (Phase D2).
#[derive(Clone, Copy)]
struct ModuleTagged<T> {
    value: T,
    /// Owning module at registration time.
    module: ModuleId,
}

impl<T> ModuleTagged<T> {
    fn new(value: T, module: ModuleId) -> Self {
        Self { value, module }
    }
}

/// Restores [`Interpreter::current_module`] when dropped (normal return or
/// unwind). Used by [`Interpreter::module`] so a panic inside an install cannot
/// leak the module scope into subsequent installs.
struct ModuleScopeGuard {
    /// Points at the interpreter's `current_module` field for the duration of
    /// the scoped install. Valid because the guard lives only while the
    /// exclusive `&mut Interpreter` borrow of `module()` is active (including
    /// on unwind).
    slot: *mut ModuleId,
    previous: ModuleId,
}

impl Drop for ModuleScopeGuard {
    fn drop(&mut self) {
        // SAFETY: `slot` is the address of `Interpreter::current_module` taken
        // under an exclusive borrow in `module()`; the interpreter outlives
        // this guard; only this Drop writes through the pointer.
        unsafe {
            *self.slot = self.previous;
        }
    }
}

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

/// Render `value` in `radix` (2...36) for `String(_:radix:uppercase:)`.
/// A negative value is prefixed with `-` and formatted by magnitude, matching
/// Swift.
fn int_to_radix_string(value: i128, radix: u32, uppercase: bool) -> String {
    debug_assert!((2..=36).contains(&radix));
    if value == 0 {
        return "0".to_string();
    }
    let negative = value < 0;
    let mut n = value.unsigned_abs();
    let radix = radix as u128;
    let mut digits = Vec::new();
    while n > 0 {
        let d = (n % radix) as u32;
        let c = std::char::from_digit(d, 36).unwrap();
        digits.push(if uppercase { c.to_ascii_uppercase() } else { c });
        n /= radix;
    }
    if negative {
        digits.push('-');
    }
    digits.iter().rev().collect()
}

type Eval = Result<SwiftValue, Signal>;

/// One declared Swift parameter, precomputed from its `AST_PARAM` node.
/// A parameter signature for a *builtin* (Rust-implemented) free function or
/// struct method, supplied at registration so the dispatcher can push a
/// contextual type while evaluating each argument. This is what lets a
/// leading-dot member argument (`.center`, `.infinity`, `.horizontal`) resolve
/// against the builtin's declared parameter type — the same mechanism Swift and
/// msf use, where a typed API signature drives implicit-member resolution.
#[derive(Clone)]
pub struct BuiltinParam {
    /// The argument label (`alignment:`); `None` for an unlabeled parameter.
    pub label: Option<String>,
    /// The declared parameter type name used as the contextual hint
    /// (`Alignment`, `Edge.Set`, `CGFloat`, …).
    pub ty: String,
    /// A variadic parameter absorbs the remaining positional arguments.
    pub variadic: bool,
}

impl BuiltinParam {
    /// A labeled parameter of the given type (`alignment: Alignment`).
    pub fn labeled(label: &str, ty: &str) -> Self {
        Self {
            label: Some(label.to_string()),
            ty: ty.to_string(),
            variadic: false,
        }
    }

    /// An unlabeled (positional) parameter of the given type (`_ edges: Edge.Set`).
    pub fn positional(ty: &str) -> Self {
        Self {
            label: None,
            ty: ty.to_string(),
            variadic: false,
        }
    }

    /// Mark this parameter variadic.
    pub fn variadic(mut self) -> Self {
        self.variadic = true;
        self
    }

    /// Lower to the internal [`Param`] shape (type hint only; no body/default).
    fn into_param(self) -> Param {
        Param {
            label: self.label.clone(),
            name: self.label.unwrap_or_default(),
            ty: Some(self.ty),
            variadic: self.variadic,
            inout_: false,
            autoclosure: false,
            default: None,
        }
    }
}

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

/// A resolved method lookup: parameters, body, the class that declares it, and
/// its generic type parameters.
type MethodLookup = (Vec<Param>, Option<Node<'static>>, String, Vec<String>);

/// A resolved method overload: parameters, body, whether it is `mutating`, and
/// its generic type parameters.
type MethodOverload = (Vec<Param>, Option<Node<'static>>, bool, Vec<String>);

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
#[derive(Default)]
struct StructDef {
    /// Declaration attributes with their leading `@` stripped (`"Model"`, …),
    /// in source order. Surfaced generically via
    /// `StdContext::nominal_type_info`.
    attributes: Vec<String>,
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
    /// Integer generic parameter names (`struct Buf<let N: Int>`), in
    /// declaration order. A call-site specialization (`Buf<4>()`) binds each
    /// as an immutable stored field on the instance.
    value_generic_params: Vec<String>,
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
#[derive(Default)]
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
    /// All method overloads keyed by method name. When a class declares
    /// multiple methods with the same name (different parameter labels — e.g.
    /// the three `urlSession` delegate variants), the last-wins `methods` map
    /// loses them; this vec retains every definition for label-aware dispatch.
    method_overloads: std::collections::HashMap<String, Vec<MethodDef>>,
    init: Option<MethodDef>,
    /// All custom initializer overloads, selected by argument labels/types.
    init_overloads: Vec<MethodDef>,
    deinit: Option<Node<'static>>,
    /// A `static subscript`, addressed as `Type[index]`.
    static_subscript: Option<MethodDef>,
    /// Declaration attributes with their leading `@` stripped (`"Model"`,
    /// …), in source order. Surfaced generically via
    /// [`crate::StdContext::nominal_type_info`]; core assigns them no meaning.
    attributes: Vec<String>,
}

/// The interpreter's registry of user-declared nominal types — the `struct`,
/// `enum`, and `class` declarations hoisted from a program. Owning the three
/// maps behind one module keeps the "what types exist" question in a single
/// place instead of smeared across the interpreter's fields; the query methods
/// (`is_struct`, `struct_def`, …) are the read surface the dispatcher uses.
#[derive(Default)]
struct TypeTable {
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
    /// User extension methods declared on builtin types (`extension Int`,
    /// `extension Array`, …), keyed by the builtin type name then method name.
    builtin_ext_methods: HashMap<String, HashMap<String, MethodDef>>,
    /// User extension computed properties on builtin types, keyed the same way.
    builtin_ext_computed: HashMap<String, HashMap<String, ComputedProp>>,
    /// Names of enums installed via [`Interpreter::register_builtin_enum`].
    /// Excluded from the global unique-case shorthand fallback so they cannot
    /// shadow SwiftUI implicit statics (`.plain`, …); they still resolve when a
    /// contextual type names them or as a last-resort fallback.
    builtin_enums: std::collections::HashSet<String>,
}

impl TypeTable {
    fn is_struct(&self, name: &str) -> bool {
        self.structs.contains_key(name)
    }
    fn is_enum(&self, name: &str) -> bool {
        self.enums.contains_key(name)
    }
    fn is_class(&self, name: &str) -> bool {
        self.classes.contains_key(name)
    }
    /// Whether `name` is any user-declared nominal type (struct, enum, or class).
    fn is_nominal(&self, name: &str) -> bool {
        self.is_struct(name) || self.is_enum(name) || self.is_class(name)
    }
    fn struct_def(&self, name: &str) -> Option<&StructDef> {
        self.structs.get(name)
    }
    fn enum_def(&self, name: &str) -> Option<&EnumDef> {
        self.enums.get(name)
    }
    fn class_def(&self, name: &str) -> Option<&ClassDef> {
        self.classes.get(name)
    }
    fn struct_def_mut(&mut self, name: &str) -> Option<&mut StructDef> {
        self.structs.get_mut(name)
    }
    fn enum_def_mut(&mut self, name: &str) -> Option<&mut EnumDef> {
        self.enums.get_mut(name)
    }
    fn class_def_mut(&mut self, name: &str) -> Option<&mut ClassDef> {
        self.classes.get_mut(name)
    }
    fn insert_struct(&mut self, name: String, def: StructDef) {
        self.structs.insert(name, def);
    }
    fn insert_enum(&mut self, name: String, def: EnumDef) {
        self.enums.insert(name, def);
    }
    fn insert_class(&mut self, name: String, def: ClassDef) {
        self.classes.insert(name, def);
    }
    fn struct_names(&self) -> impl Iterator<Item = &String> {
        self.structs.keys()
    }
    fn enum_names(&self) -> impl Iterator<Item = &String> {
        self.enums.keys()
    }
    fn class_names(&self) -> impl Iterator<Item = &String> {
        self.classes.keys()
    }

    fn is_protocol(&self, name: &str) -> bool {
        self.protocols.contains_key(name)
    }
    fn protocol_def(&self, name: &str) -> Option<&ProtoDef> {
        self.protocols.get(name)
    }
    fn protocol_def_mut(&mut self, name: &str) -> Option<&mut ProtoDef> {
        self.protocols.get_mut(name)
    }
    /// Register a protocol declaration, keeping the first `inherited` list seen.
    fn ensure_protocol(&mut self, name: String, inherited: Vec<String>) {
        self.protocols.entry(name).or_insert_with(|| ProtoDef {
            inherited,
            methods: HashMap::new(),
            computed: HashMap::new(),
            optional_methods: Vec::new(),
            optional_properties: Vec::new(),
        });
    }
    fn add_protocol_alias(&mut self, name: String, components: Vec<String>) {
        self.protocol_aliases.insert(name, components);
    }
    /// Record that `type_name` directly conforms to `protocols`.
    fn record_conformance(&mut self, type_name: &str, protocols: Vec<String>) {
        if !protocols.is_empty() {
            self.conformances
                .entry(type_name.to_string())
                .or_default()
                .extend(protocols);
        }
    }
    fn has_builtin_ext_method(&self, type_name: &str, method: &str) -> bool {
        self.builtin_ext_methods
            .get(type_name)
            .is_some_and(|m| m.contains_key(method))
    }
    fn builtin_ext_method(&self, type_name: &str, method: &str) -> Option<&MethodDef> {
        self.builtin_ext_methods.get(type_name)?.get(method)
    }
    fn builtin_ext_computed(&self, type_name: &str, prop: &str) -> Option<&ComputedProp> {
        self.builtin_ext_computed.get(type_name)?.get(prop)
    }
    /// Record the members an `extension` adds to a builtin type.
    fn add_builtin_ext(
        &mut self,
        type_name: String,
        methods: HashMap<String, MethodDef>,
        computed: HashMap<String, ComputedProp>,
    ) {
        self.builtin_ext_methods
            .entry(type_name.clone())
            .or_default()
            .extend(methods);
        self.builtin_ext_computed
            .entry(type_name)
            .or_default()
            .extend(computed);
    }
    fn mark_builtin_enum(&mut self, name: &str) {
        self.builtin_enums.insert(name.to_string());
    }
    fn is_builtin_enum(&self, name: &str) -> bool {
        self.builtin_enums.contains(name)
    }
    fn builtin_enum_names(&self) -> impl Iterator<Item = &String> {
        self.builtin_enums.iter()
    }

    /// All protocols a type conforms to, transitively (including protocol
    /// inheritance and composition-typealias expansion), for
    /// default-implementation lookup.
    fn all_protocols(&self, type_name: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut stack: Vec<String> = self
            .conformances
            .get(type_name)
            .cloned()
            .unwrap_or_default();
        while let Some(p) = stack.pop() {
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
}

/// The runtime's registry of native members keyed by `(builtin receiver, name)`
/// — the type-specific methods, labelled overloads, computed properties, and
/// static methods installed by `tswift-std`/Foundation/SwiftUI. Owning the five
/// receiver-keyed maps behind one module keeps builtin-member storage in a
/// single place; the `add_*` methods are the install seam and the singular
/// query methods are the read surface the dispatcher uses.
///
/// Each entry is module-tagged (Phase A); lookups still return the bare value
/// so dispatch behavior is unchanged.
#[derive(Default)]
struct BuiltinMembers {
    intrinsics: HashMap<(BuiltinReceiver, String), ModuleTagged<MethodEntry>>,
    labeled_intrinsics: HashMap<(BuiltinReceiver, String), ModuleTagged<LabeledMethodEntry>>,
    properties: HashMap<(BuiltinReceiver, String), ModuleTagged<PropertyFn>>,
    typed_properties: HashMap<(BuiltinReceiver, String), ModuleTagged<TypedPropertyFn>>,
    contextual_properties: HashMap<(BuiltinReceiver, String), ModuleTagged<ContextualPropertyFn>>,
    static_methods: HashMap<(BuiltinReceiver, String), ModuleTagged<StaticFn>>,
    /// Built-in computed-property setters (validate + mutate the receiver).
    setters: HashMap<(BuiltinReceiver, String), ModuleTagged<PropertySetterFn>>,
}

impl BuiltinMembers {
    fn add_intrinsic(
        &mut self,
        recv: BuiltinReceiver,
        name: &str,
        entry: MethodEntry,
        module: ModuleId,
    ) {
        self.intrinsics
            .insert((recv, name.to_string()), ModuleTagged::new(entry, module));
    }
    fn add_labeled_intrinsic(
        &mut self,
        recv: BuiltinReceiver,
        name: &str,
        entry: LabeledMethodEntry,
        module: ModuleId,
    ) {
        self.labeled_intrinsics
            .insert((recv, name.to_string()), ModuleTagged::new(entry, module));
    }
    fn add_property(&mut self, recv: BuiltinReceiver, name: &str, f: PropertyFn, module: ModuleId) {
        self.properties
            .insert((recv, name.to_string()), ModuleTagged::new(f, module));
    }
    fn add_typed_property(
        &mut self,
        recv: BuiltinReceiver,
        name: &str,
        f: TypedPropertyFn,
        module: ModuleId,
    ) {
        self.typed_properties
            .insert((recv, name.to_string()), ModuleTagged::new(f, module));
    }
    fn add_contextual_property(
        &mut self,
        recv: BuiltinReceiver,
        name: &str,
        f: ContextualPropertyFn,
        module: ModuleId,
    ) {
        self.contextual_properties
            .insert((recv, name.to_string()), ModuleTagged::new(f, module));
    }
    fn add_static_method(
        &mut self,
        recv: BuiltinReceiver,
        name: &str,
        f: StaticFn,
        module: ModuleId,
    ) {
        self.static_methods
            .insert((recv, name.to_string()), ModuleTagged::new(f, module));
    }
    fn add_setter(
        &mut self,
        recv: BuiltinReceiver,
        name: &str,
        f: PropertySetterFn,
        module: ModuleId,
    ) {
        self.setters
            .insert((recv, name.to_string()), ModuleTagged::new(f, module));
    }

    fn intrinsic(&self, recv: BuiltinReceiver, name: &str) -> Option<&ModuleTagged<MethodEntry>> {
        self.intrinsics.get(&(recv, name.to_string()))
    }
    fn labeled_intrinsic(
        &self,
        recv: BuiltinReceiver,
        name: &str,
    ) -> Option<&ModuleTagged<LabeledMethodEntry>> {
        self.labeled_intrinsics.get(&(recv, name.to_string()))
    }
    fn has_labeled_intrinsic(&self, recv: BuiltinReceiver, name: &str) -> bool {
        self.labeled_intrinsics
            .contains_key(&(recv, name.to_string()))
    }
    fn property(&self, recv: BuiltinReceiver, name: &str) -> Option<&ModuleTagged<PropertyFn>> {
        self.properties.get(&(recv, name.to_string()))
    }
    fn typed_property(
        &self,
        recv: BuiltinReceiver,
        name: &str,
    ) -> Option<&ModuleTagged<TypedPropertyFn>> {
        self.typed_properties.get(&(recv, name.to_string()))
    }
    fn contextual_property(
        &self,
        recv: BuiltinReceiver,
        name: &str,
    ) -> Option<&ModuleTagged<ContextualPropertyFn>> {
        self.contextual_properties.get(&(recv, name.to_string()))
    }
    fn static_method(&self, recv: BuiltinReceiver, name: &str) -> Option<&ModuleTagged<StaticFn>> {
        self.static_methods.get(&(recv, name.to_string()))
    }
    fn setter(&self, recv: BuiltinReceiver, name: &str) -> Option<&ModuleTagged<PropertySetterFn>> {
        self.setters.get(&(recv, name.to_string()))
    }

    /// Every registered member as a `"Receiver.name"` string, for the
    /// coverage-facing key dump ([`Interpreter::registered_keys`]).
    fn qualified_names(&self) -> impl Iterator<Item = String> + '_ {
        self.intrinsics
            .keys()
            .chain(self.labeled_intrinsics.keys())
            .chain(self.properties.keys())
            .chain(self.contextual_properties.keys())
            .chain(self.static_methods.keys())
            .map(|(recv, name)| format!("{}.{}", recv.type_name(), name))
    }
}

/// The runtime's registry of name-keyed global members — native functions,
/// free-function intrinsics (through the [`StdContext`] seam), `Sequence`
/// algorithms, and generic struct-method intrinsics (the SwiftUI modifier
/// seam). Owning the four maps behind one module concentrates global-member
/// storage; the `add_*` methods are the install seam and the singular query
/// methods are the read surface `eval_call`/`eval_method_call` use.
///
/// Free-fn / struct-method entries carry a [`ModuleId`]; natives and
/// algorithms are module-tagged wrappers. Struct methods hold per-module
/// candidates (Phase B); free-fn / native lookup stays name-only.
#[derive(Default)]
struct GlobalMembers {
    natives: HashMap<String, ModuleTagged<NativeFn>>,
    free_fns: HashMap<String, FreeFnEntry>,
    algorithms: HashMap<String, ModuleTagged<AlgoFn>>,
    /// Name → per-module candidates (same module re-install replaces in place).
    struct_methods: HashMap<String, Vec<StructMethodEntry>>,
}

impl GlobalMembers {
    fn add_native(&mut self, name: &str, f: NativeFn, module: ModuleId) {
        self.natives
            .insert(name.to_string(), ModuleTagged::new(f, module));
    }
    fn add_free_fn(&mut self, name: &str, entry: FreeFnEntry) {
        self.free_fns.insert(name.to_string(), entry);
    }
    fn add_algorithm(&mut self, name: &str, f: AlgoFn, module: ModuleId) {
        self.algorithms
            .insert(name.to_string(), ModuleTagged::new(f, module));
    }
    /// Append a candidate for `entry.module`, or replace an existing same-
    /// (name, module) entry so re-install is idempotent.
    fn add_struct_method(&mut self, name: &str, entry: StructMethodEntry) {
        let list = self.struct_methods.entry(name.to_string()).or_default();
        if let Some(slot) = list.iter_mut().find(|e| e.module == entry.module) {
            *slot = entry;
        } else {
            list.push(entry);
        }
    }

    fn native(&self, name: &str) -> Option<&ModuleTagged<NativeFn>> {
        self.natives.get(name)
    }
    fn free_fn(&self, name: &str) -> Option<&FreeFnEntry> {
        self.free_fns.get(name)
    }
    fn algorithm(&self, name: &str) -> Option<&ModuleTagged<AlgoFn>> {
        self.algorithms.get(name)
    }
    fn struct_method_candidates(&self, name: &str) -> &[StructMethodEntry] {
        self.struct_methods
            .get(name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
    /// Resolve a struct-method candidate for `receiver_module` (see
    /// [`resolve_struct_method_candidate`]). Under strict import-gating only
    /// candidates from imported modules are considered; otherwise `imported`
    /// is a same-tier fallback preference (Phase C lenient).
    fn struct_method(
        &self,
        name: &str,
        receiver_module: Option<ModuleId>,
        imported: &HashSet<ModuleId>,
        strict: bool,
    ) -> Option<&StructMethodEntry> {
        resolve_struct_method_candidate(
            self.struct_method_candidates(name),
            receiver_module,
            imported,
            strict,
        )
    }
    fn free_fn_names(&self) -> impl Iterator<Item = &String> {
        self.free_fns.keys()
    }
    fn algorithm_names(&self) -> impl Iterator<Item = &String> {
        self.algorithms.keys()
    }
}

/// The runtime's table of core-internal, value-only builtin constructors —
/// generic collection ctors (`Array`/`Set`/`Dictionary`/…), scalar conversion
/// initializers, and the `JSONEncoder`/`JSONDecoder` markers — keyed by global
/// name. Fixed at construction (no registration seam) and consulted once by
/// `eval_call`; owning it behind a newtype keeps the raw map off the
/// interpreter and gives the lookup a named query method.
#[derive(Default)]
struct BuiltinCtors {
    table: HashMap<&'static str, BuiltinCtor>,
}

impl BuiltinCtors {
    fn ctor(&self, name: &str) -> Option<BuiltinCtor> {
        self.table.get(name).copied()
    }
}

/// A protocol declaration: its inherited protocols and any default member
/// implementations supplied through `extension Protocol { … }`.
struct ProtoDef {
    inherited: Vec<String>,
    methods: std::collections::HashMap<String, MethodDef>,
    computed: std::collections::HashMap<String, ComputedProp>,
    /// `@objc optional` method requirement names: a conformer may omit them,
    /// and a call on a non-implementing conformer resolves to `nil` (chained
    /// or not — the parser drops the `?`, so plain access nil-propagates too).
    optional_methods: Vec<String>,
    /// `@objc optional` property requirement names, resolving to `nil` on
    /// reads from a non-implementing conformer. Kept separate from methods so
    /// a property name never nils out a *call* miss and vice versa.
    optional_properties: Vec<String>,
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
    /// Synthetic closure injected by Foundation's delegate dispatcher for
    /// `urlSession(_:dataTask:didReceive:completionHandler:)`. When called with
    /// one `URLSession.ResponseDisposition` argument it writes the disposition
    /// to `Interpreter::response_disposition`; Foundation reads it back via
    /// `StdContext::take_response_disposition`.
    ///
    /// The embedded `token` matches `Interpreter::response_disposition_token`
    /// at the time the closure was allocated.  If the token has advanced (e.g.
    /// a new request started and called `allocate_response_disposition_closure`
    /// again), a late invocation of an old closure is silently ignored — it
    /// cannot poison the next request's disposition.
    ResponseDispositionCapture {
        token: u64,
    },
}

/// An assignable storage location: a root variable plus a field path.
#[derive(Debug, Clone)]
struct Place {
    root: String,
    path: Vec<String>,
}

/// Whether any identifier in `node`'s subtree references `name`. String
/// literals are checked textually because their `\( … )` interpolations are
/// re-parsed at evaluation time, so an embedded reference is invisible to the
/// node tree — a substring hit conservatively counts as a reference.
fn subtree_references_name(node: &Node<'static>, name: &str) -> bool {
    match node.kind() {
        NodeKind::IdentExpr if node.text().as_deref() == Some(name) => return true,
        NodeKind::StringLiteral if node.text().is_some_and(|t| t.contains(name)) => {
            return true;
        }
        _ => {}
    }
    node.children().any(|c| subtree_references_name(&c, name))
}

/// An integer value written into a scalar-annotated context re-types to match:
/// `Double`/`Float` makes it floating point (`let r: Double = 5`); an explicit
/// integer width re-types it. `None` when no coercion applies.
fn coerce_int_to_type_name(ty: &str, value: &SwiftValue) -> Option<SwiftValue> {
    let SwiftValue::Int(i) = value else {
        return None;
    };
    if matches!(ty, "Double" | "Float") {
        return Some(SwiftValue::Double(i.raw as f64));
    }
    IntWidth::from_type_name(ty).map(|w| SwiftValue::Int(IntValue::new(i.raw, w)))
}

/// A global/local variable declared with accessor bodies (TSPL "Global and
/// Local Variables"): either computed (`get`/`set`) or stored with observers
/// (`willSet`/`didSet`). Accessor bodies are stored as closures capturing the
/// declaration's scope chain; the variable's environment binding holds a
/// `SwiftValue::AccessorVar` marker pointing at this slot.
struct AccessorVarSlot {
    /// The variable's name, for diagnostics.
    name: String,
    /// The declared type annotation, used to coerce written integer values
    /// into a floating/width-specific scalar (`var f: Double { set }`).
    ty: Option<String>,
    /// Computed getter closure id.
    getter: Option<usize>,
    /// Computed setter closure id; its single parameter is the written value.
    setter: Option<usize>,
    /// `willSet` observer closure id; its parameter is the incoming value.
    will_set: Option<usize>,
    /// `didSet` observer closure id; its parameter is the replaced value.
    did_set: Option<usize>,
    /// Backing storage for an observed stored variable (`None` when computed).
    /// Shared so closures capturing the scope observe live updates.
    storage: Option<Rc<RefCell<SwiftValue>>>,
}

/// A single evaluated call argument: its label, value, and (for `inout`) the
/// caller location to write back to.
struct CallArg {
    label: Option<String>,
    value: SwiftValue,
    place: Option<Place>,
}

/// A core-internal constructor for a value-only global builtin (collection
/// ctors, scalar conversions, JSON coder markers). It keeps full `&mut
/// Interpreter` access (unlike the external `StdContext` seam) and internally
/// dispatches on the evaluated arguments. `Ok(None)` means "these arguments are
/// not mine" — the caller falls through the rest of the dispatch ladder.
type BuiltinCtor = fn(&mut Interpreter, &str, &[CallArg]) -> Result<Option<SwiftValue>, Signal>;

/// Drop the write-back [`Place`], keeping just the label and value — the shape
/// the stdlib seam ([`Arg`]) consumes.
impl From<CallArg> for Arg {
    fn from(arg: CallArg) -> Arg {
        Arg {
            label: arg.label,
            value: arg.value,
            static_ty: None,
        }
    }
}

impl From<&CallArg> for Arg {
    fn from(arg: &CallArg) -> Arg {
        Arg {
            label: arg.label.clone(),
            value: arg.value.clone(),
            static_ty: None,
        }
    }
}

/// A registered free-function intrinsic and its optional declared parameter
/// signature. Pairing the two keeps registration atomic: the function and the
/// contextual-type hints it implies can never drift apart.
struct FreeFnEntry {
    f: FreeFn,
    params: Option<Vec<Param>>,
    /// Module that registered this free function (also mirrored into
    /// `type_modules` for constructor names). Used by strict import-gating.
    module: ModuleId,
}

/// A registered generic struct-method intrinsic (SwiftUI modifier seam) and its
/// optional declared parameter signature.
struct StructMethodEntry {
    f: StructMethodFn,
    params: Option<Vec<Param>>,
    /// Module that registered this candidate (Phase B: pick by receiver module).
    module: ModuleId,
}

/// Simple static depends-on / re-export edges used when the receiver's own
/// module has no candidate for a shared name (ADR-0020 Phase B).
///
/// `Charts` → `SwiftUI` (ChartContent falls back to View modifiers);
/// every other framework → `Swift` base. Expand only when a real re-export
/// graph is needed (see ADR tripwires).
fn module_depends_on(module: ModuleId) -> &'static [ModuleId] {
    const CHARTS_DEPS: &[ModuleId] = &[ModuleId::SWIFTUI, ModuleId::SWIFT];
    const BASE_DEPS: &[ModuleId] = &[ModuleId::SWIFT];
    match module.as_str() {
        "Charts" => CHARTS_DEPS,
        "SwiftUI" | "SwiftData" | "Foundation" => BASE_DEPS,
        _ => BASE_DEPS,
    }
}

/// Pick a struct-method candidate for a receiver owned by `receiver_module`.
///
/// Under strict import-gating (Phase D2 default):
/// - If the receiver module owns an exact candidate and that module is
///   imported → use it.
/// - If the receiver module owns an exact candidate but the module is **not**
///   imported → return `None` (caller emits the import-hint diagnostic). Do
///   **not** fall through to a different module's handler for the same name
///   (e.g. `BarMark.foregroundStyle` must not steal SwiftUI's when Charts is
///   not imported).
/// - Fall through to depends-on / base / same-tier only when the receiver has
///   **no** own candidate at all, and only among imported modules.
///
/// Priority among remaining candidates (deterministic; never registration order):
/// 1. Exact match on the receiver's own module
/// 2. A module in the receiver module's depends-on / re-export chain
/// 3. Base language module [`ModuleId::SWIFT`]
/// 4. Same-tier preference: a candidate whose module is in `imported` (lenient
///    reorder when `strict` is false; `Swift` is always treated as imported)
/// 5. Stable tiebreak: alphabetical by module id among remaining candidates
///
/// Unknown / untagged receivers skip (1)–(2) and use (3)–(5).
fn resolve_struct_method_candidate<'a>(
    candidates: &'a [StructMethodEntry],
    receiver_module: Option<ModuleId>,
    imported: &HashSet<ModuleId>,
    strict: bool,
) -> Option<&'a StructMethodEntry> {
    if candidates.is_empty() {
        return None;
    }
    // Strict: receiver owns a candidate whose module is not imported → hard
    // miss (no cross-module steal). Only fall through when there is no own
    // candidate at all.
    if strict {
        if let Some(m) = receiver_module {
            if let Some(exact) = candidates.iter().find(|e| e.module == m) {
                if candidate_module_is_imported(m, imported) {
                    return Some(exact);
                }
                return None;
            }
        }
    }
    // Eligible pool: imported-only under strict; all candidates when lenient.
    let eligible: Vec<&StructMethodEntry> = if strict {
        candidates
            .iter()
            .filter(|e| candidate_module_is_imported(e.module, imported))
            .collect()
    } else {
        candidates.iter().collect()
    };
    if eligible.is_empty() {
        return None;
    }
    if let Some(m) = receiver_module {
        // Lenient path still prefers exact match first (strict already returned).
        if !strict {
            if let Some(e) = eligible.iter().copied().find(|e| e.module == m) {
                return Some(e);
            }
        }
        for dep in module_depends_on(m) {
            if let Some(e) = eligible.iter().copied().find(|e| e.module == *dep) {
                return Some(e);
            }
        }
    }
    // Base language module, if present among candidates.
    if let Some(e) = eligible
        .iter()
        .copied()
        .find(|e| e.module == ModuleId::SWIFT)
    {
        return Some(e);
    }
    // Same-tier: prefer an imported module's candidate, then alphabetical
    // (never insertion order). Under strict mode every remaining candidate is
    // already imported, so this only reorders.
    eligible.into_iter().min_by_key(|e| {
        let not_imported = !candidate_module_is_imported(e.module, imported);
        (not_imported, e.module.as_str())
    })
}

/// Whether `module` counts as imported. [`ModuleId::SWIFT`] is always imported
/// (stdlib implicit).
fn candidate_module_is_imported(module: ModuleId, imported: &HashSet<ModuleId>) -> bool {
    module == ModuleId::SWIFT || imported.contains(&module)
}

/// Diagnostic for a registered framework symbol gated out by strict imports.
fn not_in_scope_error(name: &str, module: ModuleId) -> EvalError {
    EvalError::Type(format!(
        "cannot find '{name}' in scope (did you forget to import {}?)",
        module.as_str()
    ))
}

/// Module owning a core-internal framework builtin constructor (`JSONEncoder`,
/// …). Stdlib conversion/collection ctors are untagged (always visible).
fn builtin_ctor_module(name: &str) -> Option<ModuleId> {
    match name {
        "JSONEncoder" | "JSONDecoder" | "PropertyListEncoder" => Some(ModuleId::FOUNDATION),
        _ => None,
    }
}

/// A resolved `Type.member` type reference: the concrete type name (after
/// `Self`/generic-alias substitution) and whether it names a user struct, class,
/// or enum (vs. a builtin type).
struct TypeReference {
    name: String,
    user_defined: bool,
}

/// The tree-walking interpreter.
pub struct Interpreter<'w> {
    out: &'w mut dyn Write,
    /// Core-internal value-only builtin constructors (collection ctors, scalar
    /// conversions, JSON coder markers), keyed by global name. Consulted once
    /// in `eval_call`, gated by [`Interpreter::is_unshadowed`], after user-type
    /// and binding dispatch so a same-named user type or binding wins.
    builtin_ctors: BuiltinCtors,
    /// Native members keyed by `(builtin receiver, name)` — method intrinsics,
    /// labelled overloads, computed properties, and static methods — behind one
    /// registry seam.
    builtins: BuiltinMembers,
    /// Name-keyed global members — native functions, free-function intrinsics,
    /// `Sequence` algorithms, and generic struct-method intrinsics — behind one
    /// registry seam.
    globals: GlobalMembers,
    /// Module currently receiving registrations (see [`Interpreter::module`]).
    /// Defaults to [`ModuleId::swift`]; framework `install()` scopes set this
    /// while registering their symbols.
    current_module: ModuleId,
    /// Modules the current program has imported (ADR-0020 Phase C/D2). Seeded
    /// with base [`ModuleId::SWIFT`] (stdlib always implicitly imported).
    /// Populated from top-level `ImportDecl` during hoist; hosts may also
    /// [`mark_module_imported`][Interpreter::mark_module_imported]. Under
    /// strict import-gating (default), only these modules' framework symbols
    /// resolve; when strict is off, the set only affects same-tier preference.
    imported_modules: HashSet<ModuleId>,
    /// When true (default, Phase D2), framework-module symbols resolve only if
    /// their module is imported. Base `"Swift"` is never gated. Toggle off via
    /// [`Interpreter::set_strict_imports`] for a specific lenient test.
    strict_imports: bool,
    /// Type / constructor name → owning module. Populated by free-fn, enum,
    /// and static-value registration. Phase B uses this to resolve the
    /// receiver's module for struct-method dispatch.
    type_modules: HashMap<String, ModuleId>,
    /// Module stamp for each `Type.member` key written by
    /// [`Interpreter::register_static_value`].
    static_modules: HashMap<String, ModuleId>,
    /// Module stamp for each host-native function registered via
    /// [`Interpreter::register_host_fn`]. Dispatch still goes through
    /// [`Self::host_bridge`] by name alone; module is used for gating.
    host_fn_modules: HashMap<String, ModuleId>,
    env: Env,
    funcs: Vec<FuncDef>,
    /// User-declared nominal types (`struct`/`enum`/`class`), behind one seam.
    types: TypeTable,
    closures: Vec<(ClosureDef, Vec<Scope>)>,
    /// Global/local variables with accessor bodies, indexed by
    /// `SwiftValue::AccessorVar` markers held in environment bindings.
    accessor_vars: Vec<AccessorVarSlot>,
    statics: HashMap<String, SwiftValue>,
    /// Stack of type names for the `static` methods currently executing, so an
    /// unqualified reference inside a `static func` resolves to a type-level
    /// (static) property of that type.
    static_ctx: Vec<String>,
    /// Stack of class names for the methods currently executing (for `super`).
    class_ctx: Vec<String>,
    /// Nesting depth of initializer bodies currently executing; `self.init`
    /// delegation is legal only when this is non-zero.
    init_ctx: usize,
    /// Stack of generic type-parameter substitutions for the calls currently
    /// executing, so a static reference through a generic placeholder
    /// (`T.zero()` where `T == Vec2`) resolves to the concrete type.
    type_bindings: Vec<HashMap<String, String>>,
    /// Per-scope stack of `defer` blocks, run LIFO on scope exit.
    defer_stack: Vec<Vec<Node<'static>>>,
    /// The `@main` entry type, if one was declared.
    main_type: Option<String>,
    /// The structured-concurrency state machine (ADR-0005): the task table, the
    /// running-task stack, task groups, and continuation slots, behind one
    /// seam. Driving methods on the interpreter delegate all task/group/
    /// continuation bookkeeping here; see [`Scheduler`].
    sched: Scheduler,
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
    /// Interpreter-owned cache of interpolation-fragment analyses (ADR-0007).
    /// Replaces the per-evaluation `Box::leak`: a repeated fragment is analyzed
    /// once, and the whole cache is reclaimed when the interpreter drops.
    fragment_cache: FragmentCache,
    /// Programs whose `Analysis` this interpreter *retains ownership of* (shared
    /// via `Rc`), set by [`Interpreter::run_retaining`]. Keeps the AST alive for
    /// the whole interpreter lifetime so every `Node<'static>` the run stores
    /// stays valid, while letting a warm-start cache evict its *own* `Rc`
    /// without freeing an AST an interpreter is still walking. Dropped when the
    /// interpreter drops (same soundness model as [`FragmentCache`]); a `Node`
    /// cursor carries no `Drop`, so field drop-order is irrelevant.
    retained_analyses: Vec<Rc<Analysis>>,
    /// The embedding-installed HTTP backend behind `URLSession` (see
    /// [`crate::http`]); `None` means network access is unavailable.
    http_transport: Option<Box<dyn crate::http::HttpTransport>>,
    /// The host-native function bridge (Epic #246): the registry of host
    /// functions callable from interpreted Swift, plus the shared trampoline
    /// that validates arguments, crosses the JSON boundary, and validates the
    /// result. See [`crate::host_bridge`].
    host_bridge: crate::host_bridge::HostBridge,
    /// Last response disposition captured by the `ResponseDispositionCapture`
    /// synthetic closure (Foundation M4 delegate dispatch). `true` = allow,
    /// `false` = cancel.  Reset to `None` by `allocate_response_disposition_closure`
    /// so that late invocations of a stale closure are detected by the token
    /// guard and silently ignored.  Consumed by
    /// `StdContext::take_response_disposition`.
    response_disposition: Option<bool>,
    /// Monotonically increasing counter.  Incremented each time
    /// `allocate_response_disposition_closure` is called.  The allocated
    /// closure carries the value at allocation time; a `ResponseDispositionCapture`
    /// invocation whose token does not match the current value is discarded,
    /// preventing a script-stored completionHandler from poisoning a later
    /// request's disposition.
    response_disposition_token: u64,
    /// Per-interpreter cache backing [`StdContext::singleton`], keyed by the
    /// caller-supplied opaque string. See that method's doc for why this is
    /// a separate table from `statics` (no bare `.name` shorthand fallback
    /// consults it).
    singletons: HashMap<String, SwiftValue>,
    /// Finalizer closures registered by frameworks via
    /// [`StdContext::register_finalizer`], run once each (in registration
    /// order) at interpreter teardown so a framework holding a native resource
    /// (e.g. an open database handle in a thread-local registry) can release it
    /// deterministically. Drained by [`Interpreter::teardown`], which the
    /// `Drop` impl also calls, so each finalizer runs exactly once.
    /// Module-tagged for registry uniformity (always-active; not import-gated).
    finalizers: Vec<ModuleTagged<crate::stdlib::Finalizer>>,
    /// Render-scope hook pairs registered by frameworks via
    /// [`Interpreter::register_view_scope`]. The SwiftUI renderer brackets each
    /// custom `View`'s `body` evaluation with `view_scope_enter`/`_exit`, which
    /// invoke every pair's enter (registration order) / exit (reverse order) so
    /// a framework can push and restore subtree-scoped state a modifier carries.
    /// Core assigns the view value no meaning.
    /// Module-tagged for registry uniformity (always-active; not import-gated).
    view_scopes: Vec<ModuleTagged<(crate::stdlib::ViewScopeFn, crate::stdlib::ViewScopeFn)>>,
    /// Freestanding-macro handlers registered by frameworks via
    /// [`Interpreter::register_macro`], keyed by the macro name (`"Predicate"`
    /// for `#Predicate`). Consulted by [`Interpreter::eval_macro`] before the
    /// builtin macros. Core assigns the name no meaning. Module-tagged (Phase A);
    /// lookup still returns the bare handler.
    macros: HashMap<String, ModuleTagged<crate::stdlib::MacroFn>>,
    /// A process-unique, monotonically-assigned identity for this interpreter,
    /// handed out at construction from [`NEXT_INTERPRETER_ID`]. Exposed
    /// generically via [`StdContext::interpreter_id`] so a framework holding
    /// per-interpreter native state in a shared (e.g. thread-local) registry
    /// can scope its bucket to exactly this interpreter instead of colliding
    /// with other interpreters sharing the thread. Core assigns it no further
    /// meaning.
    id: u64,
}

/// Source of process-unique [`Interpreter::id`] values. A relaxed atomic
/// counter — identities need only be distinct, not ordered across threads.
static NEXT_INTERPRETER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

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

fn now_unix_seconds() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0.0
    }
}

impl Drop for Interpreter<'_> {
    fn drop(&mut self) {
        // Release framework-held native resources deterministically at end of
        // session. Idempotent: `teardown` drains the finalizer list, so an
        // explicit `teardown()` before drop leaves this a no-op.
        self.teardown();
    }
}

impl<'w> Interpreter<'w> {
    /// Create an interpreter that writes program output to `out`.
    pub fn new(out: &'w mut dyn Write) -> Self {
        Interpreter {
            out,
            builtin_ctors: Self::builtin_ctor_table(),
            builtins: BuiltinMembers::default(),
            globals: GlobalMembers::default(),
            current_module: ModuleId::swift(),
            imported_modules: HashSet::from([ModuleId::SWIFT]),
            strict_imports: true,
            type_modules: HashMap::new(),
            static_modules: HashMap::new(),
            host_fn_modules: HashMap::new(),
            env: Env::new(),
            type_bindings: Vec::new(),
            funcs: Vec::new(),
            types: TypeTable::default(),
            closures: Vec::new(),
            accessor_vars: Vec::new(),
            init_ctx: 0,
            statics: HashMap::new(),
            static_ctx: Vec::new(),
            class_ctx: Vec::new(),
            defer_stack: Vec::new(),
            main_type: None,
            sched: Scheduler::default(),
            filename: "main.swift".into(),
            depth: 0,
            // SplitMix64 tolerates any seed (including 0), so the wall-clock
            // nanos are used as-is rather than forcing the low bit.
            rng_state: initial_rng_seed(),
            type_hint: Vec::new(),
            fragment_cache: FragmentCache::default(),
            retained_analyses: Vec::new(),
            http_transport: None,
            host_bridge: crate::host_bridge::HostBridge::default(),
            response_disposition: None,
            response_disposition_token: 0,
            singletons: HashMap::new(),
            finalizers: Vec::new(),
            view_scopes: Vec::new(),
            macros: HashMap::new(),
            id: NEXT_INTERPRETER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Run `f` with `name` as the active registration module. Every
    /// `register_*` call inside stamps its entry with that module. The previous
    /// module is restored when the scope ends, including if `f` panics (RAII
    /// [`ModuleScopeGuard`]), so a caught unwind cannot leak the module scope
    /// into subsequent installs. Nesting is supported.
    pub fn module<R>(&mut self, name: &'static str, f: impl FnOnce(&mut Self) -> R) -> R {
        let previous = std::mem::replace(&mut self.current_module, ModuleId::new(name));
        // SAFETY: exclusive `&mut self` borrow is held for this whole call
        // (including unwind). The guard only writes `previous` back on Drop.
        let _guard = ModuleScopeGuard {
            slot: std::ptr::addr_of_mut!(self.current_module),
            previous,
        };
        f(self)
    }

    /// Set the active registration module without a scope (pair with
    /// [`end_module`][Interpreter::end_module]). Prefer [`module`] when possible.
    pub fn begin_module(&mut self, name: &'static str) -> ModuleId {
        std::mem::replace(&mut self.current_module, ModuleId::new(name))
    }

    /// Restore a module previously returned by [`begin_module`].
    pub fn end_module(&mut self, previous: ModuleId) {
        self.current_module = previous;
    }

    /// The module currently receiving registrations.
    pub fn current_module(&self) -> ModuleId {
        self.current_module
    }

    /// Owning module of a registered type / constructor name, if any.
    pub fn type_module(&self, name: &str) -> Option<&str> {
        self.type_modules.get(name).map(|m| m.as_str())
    }

    /// Modules that registered a struct-method candidate named `name`
    /// (registration order). Empty when none.
    pub fn struct_method_modules(&self, name: &str) -> Vec<&'static str> {
        self.globals
            .struct_method_candidates(name)
            .iter()
            .map(|e| e.module.as_str())
            .collect()
    }

    /// Module of the candidate that would be selected for a receiver owned by
    /// `receiver_type` (via [`Self::type_module`]), or the deterministic
    /// fallback when the type is unknown / untagged. Honors strict import-gating.
    pub fn struct_method_module_for(&self, name: &str, receiver_type: &str) -> Option<&str> {
        let recv_mod = self.type_modules.get(receiver_type).copied();
        self.globals
            .struct_method(name, recv_mod, &self.imported_modules, self.strict_imports)
            .map(|e| e.module.as_str())
    }

    /// Module of the fallback-resolved candidate for `name` (no receiver type).
    /// Prefer [`Self::struct_method_module_for`] when the receiver is known.
    pub fn struct_method_module(&self, name: &str) -> Option<&str> {
        self.globals
            .struct_method(name, None, &self.imported_modules, self.strict_imports)
            .map(|e| e.module.as_str())
    }

    /// Modules the current program has imported (includes base `"Swift"`).
    pub fn imported_modules(&self) -> &HashSet<ModuleId> {
        &self.imported_modules
    }

    /// Whether strict import-gating is enabled (default `true`).
    pub fn strict_imports(&self) -> bool {
        self.strict_imports
    }

    /// Enable or disable strict import-gating. Default is `true` (Phase D2).
    /// Tests that need the Phase C lenient behaviour can pass `false`.
    pub fn set_strict_imports(&mut self, strict: bool) {
        self.strict_imports = strict;
    }

    /// Whether module `name` is imported. Base `"Swift"` (stdlib) is always
    /// true. Leading component only — `name` may be a bare module id.
    pub fn is_module_imported(&self, name: &str) -> bool {
        if name == ModuleId::SWIFT.as_str() {
            return true;
        }
        self.imported_modules.iter().any(|m| m.as_str() == name)
    }

    /// Whether a symbol owned by `module` is resolvable under the current
    /// import set and strict-gating flag. Base `"Swift"` is always visible.
    /// When strict imports is off, every installed module is visible.
    pub(crate) fn module_symbol_visible(&self, module: ModuleId) -> bool {
        if !self.strict_imports {
            return true;
        }
        candidate_module_is_imported(module, &self.imported_modules)
    }

    /// Central import gate: `Ok(())` when `module`'s symbols are visible, else
    /// the standard "cannot find '{name}' in scope (did you forget to import
    /// M?)" diagnostic. All framework resolution seams should call this so the
    /// diagnostic is uniform.
    pub(crate) fn gate_module_symbol(&self, name: &str, module: ModuleId) -> Result<(), EvalError> {
        if self.module_symbol_visible(module) {
            Ok(())
        } else {
            Err(not_in_scope_error(name, module))
        }
    }

    /// Gate a type / constructor name via [`Self::type_modules`]. Untagged
    /// (user) names always pass.
    fn gate_type_name(&self, name: &str) -> Result<(), EvalError> {
        match self.type_modules.get(name).copied() {
            Some(module) => self.gate_module_symbol(name, module),
            None => Ok(()),
        }
    }

    /// Gate a registered static key (`Type.member`). `display_name` is the
    /// name shown in the diagnostic (member or full key). Untagged user
    /// statics always pass.
    fn gate_static_key(&self, key: &str, display_name: &str) -> Result<(), EvalError> {
        match self.static_modules.get(key).copied() {
            Some(module) => self.gate_module_symbol(display_name, module),
            None => Ok(()),
        }
    }

    /// Whether a registered static key (`Type.member`) is visible under import
    /// gating. User-declared statics (no module stamp) are always visible.
    fn static_key_visible(&self, key: &str) -> bool {
        match self.static_modules.get(key) {
            Some(module) => self.module_symbol_visible(*module),
            None => true,
        }
    }

    /// Whether a type / constructor name's owning module is visible. Names with
    /// no module stamp (user types) are always visible.
    fn type_name_visible(&self, name: &str) -> bool {
        match self.type_modules.get(name) {
            Some(module) => self.module_symbol_visible(*module),
            None => true,
        }
    }

    /// If the receiver module owns a struct-method candidate for `method` but
    /// that module is not imported under strict gating, return the standard
    /// import-hint error. Used when candidate selection returns `None` so the
    /// miss is not a generic "unsupported method".
    fn gated_struct_method_error(
        &self,
        method: &str,
        receiver_module: Option<ModuleId>,
    ) -> Option<EvalError> {
        if !self.strict_imports {
            return None;
        }
        let m = receiver_module?;
        let owns = self
            .globals
            .struct_method_candidates(method)
            .iter()
            .any(|e| e.module == m);
        if owns && !self.module_symbol_visible(m) {
            Some(not_in_scope_error(method, m))
        } else {
            None
        }
    }

    /// Record that `name` is imported (host/test pre-seed, or hoist of
    /// `ImportDecl`). Accepts a full import path; only the leading component
    /// is stored (`SwiftUI.Foo` → `SwiftUI`). Idempotent.
    pub fn mark_module_imported(&mut self, name: &str) {
        let leading = import_path_leading_component(name);
        if leading.is_empty() {
            return;
        }
        // Avoid re-leaking unknown names on repeated marks.
        if self.imported_modules.iter().any(|m| m.as_str() == leading) {
            return;
        }
        self.imported_modules
            .insert(module_id_for_import_name(leading));
    }

    /// Module that registered the host-native function named `name`, if any.
    pub fn host_fn_module(&self, name: &str) -> Option<&str> {
        self.host_fn_modules.get(name).map(|m| m.as_str())
    }

    /// Module that registered the freestanding macro named `name`, if any.
    pub fn macro_module(&self, name: &str) -> Option<&str> {
        self.macros.get(name).map(|t| t.module.as_str())
    }

    /// Record that `name` is a type/constructor owned by the current module.
    fn record_type_module(&mut self, name: &str) {
        self.type_modules
            .insert(name.to_string(), self.current_module);
    }

    /// Run all registered finalizers (in registration order) and clear them, so
    /// a subsequent call — including the one the `Drop` impl makes — is a no-op.
    /// Frameworks register finalizers via [`StdContext::register_finalizer`] to
    /// release native resources (e.g. close open database handles) at end of
    /// session. Safe to call explicitly for a deterministic teardown ahead of
    /// drop.
    pub fn teardown(&mut self) {
        let finalizers = std::mem::take(&mut self.finalizers);
        for tagged in finalizers {
            (tagged.value)(self);
        }
    }

    /// Set the source file name reported by `#file`.
    pub fn set_filename(&mut self, name: &str) {
        self.filename = name.to_string();
    }

    /// Install the HTTP transport backing `URLSession` (see [`crate::http`]).
    /// Absent a transport, `URLSession` requests report an unsupported-feature
    /// error rather than touching the network.
    pub fn set_http_transport(&mut self, transport: Box<dyn crate::http::HttpTransport>) {
        self.http_transport = Some(transport);
    }

    /// Install the default handler servicing host-native functions (Epic #246).
    /// Mirrors [`set_http_transport`][Interpreter::set_http_transport]: host
    /// functions registered without their own handler route through this one.
    pub fn set_host_call_handler(
        &mut self,
        handler: std::sync::Arc<dyn crate::host_bridge::HostCallHandler>,
    ) {
        self.host_bridge.set_handler(handler);
    }

    /// Register a host-native function from its signature JSON, callable from
    /// interpreted Swift by the signature's `name`. `handler` services this
    /// function; pass `None` to use the default handler installed via
    /// [`set_host_call_handler`][Interpreter::set_host_call_handler]. Returns
    /// the registered name (see [`crate::host_bridge`] for the schema).
    pub fn register_host_fn(
        &mut self,
        signature_json: &str,
        handler: Option<std::sync::Arc<dyn crate::host_bridge::HostCallHandler>>,
    ) -> Result<String, String> {
        let name = self.host_bridge.register(signature_json, handler)?;
        self.host_fn_modules
            .insert(name.clone(), self.current_module);
        Ok(name)
    }

    /// Run the shared host-call trampoline for a registered host function.
    /// `args` are the already-evaluated `(label, value)` call arguments. Maps
    /// the bridge outcome onto the interpreter's control-flow channel: a
    /// validated value, a thrown (catchable) Swift error, or a runtime type
    /// error naming the function.
    fn call_host_fn(&self, name: &str, args: &[(Option<String>, SwiftValue)]) -> Eval {
        use crate::host_bridge::HostCallOutcome;
        match self.host_bridge.invoke(name, args) {
            Ok(HostCallOutcome::Value(v)) => Ok(v),
            Ok(HostCallOutcome::Thrown(message)) => {
                Err(Signal::Throw(SwiftValue::Struct(Rc::new(StructObj {
                    type_name: "HostError".into(),
                    fields: vec![("message".into(), SwiftValue::Str(message))],
                }))))
            }
            Err(msg) => Err(EvalError::Type(msg).into()),
        }
    }

    /// Whether `name` is a registered, unshadowed host function.
    fn is_host_fn(&self, name: &str) -> bool {
        self.host_bridge.contains(name)
    }

    /// Register a native function callable from Swift source by `name`.
    pub fn register_native(&mut self, name: &str, f: NativeFn) {
        self.globals.add_native(name, f, self.current_module);
    }

    /// Register a free-function intrinsic served through the [`StdContext`] seam.
    pub fn register_free_fn(&mut self, name: &str, f: FreeFn) {
        self.globals.add_free_fn(
            name,
            FreeFnEntry {
                f,
                params: None,
                module: self.current_module,
            },
        );
        self.record_type_module(name);
    }

    /// Register a freestanding-macro handler served through the [`StdContext`]
    /// seam, keyed by the macro name without its leading `#` (`"Predicate"`
    /// handles `#Predicate`). Consulted by the macro evaluator before the
    /// builtin macros, so a framework can give `#Name<T> { … }` custom
    /// semantics (e.g. compiling a predicate closure to SQL). Core assigns the
    /// name and node shape no meaning. Stamped with the current module (Phase A).
    pub fn register_macro(&mut self, name: &str, f: crate::stdlib::MacroFn) {
        self.macros
            .insert(name.to_string(), ModuleTagged::new(f, self.current_module));
    }

    /// Register a free-function intrinsic together with a declared parameter
    /// signature. The signature is used only to push a contextual type while
    /// each argument is evaluated, so a leading-dot member argument resolves
    /// against the parameter type (`VStack(alignment: .leading)`).
    pub fn register_free_fn_typed(&mut self, name: &str, f: FreeFn, params: Vec<BuiltinParam>) {
        let params = params.into_iter().map(BuiltinParam::into_param).collect();
        self.globals.add_free_fn(
            name,
            FreeFnEntry {
                f,
                params: Some(params),
                module: self.current_module,
            },
        );
        self.record_type_module(name);
    }

    /// Register a simple builtin enum so shorthand `.case` members resolve to it
    /// (e.g. `Calendar.Component`). Cases carry no raw value and no payload; the
    /// enum is skipped if a declaration with the same name already exists.
    pub fn register_builtin_enum(&mut self, name: &str, cases: &[&str]) {
        if self.types.is_enum(name) {
            return;
        }
        let cases = cases
            .iter()
            .map(|case| EnumCaseDef {
                name: (*case).to_string(),
                raw: None,
                payload_types: Vec::new(),
            })
            .collect();
        self.types.insert_enum(
            name.to_string(),
            EnumDef {
                cases,
                methods: std::collections::HashMap::new(),
                computed: std::collections::HashMap::new(),
            },
        );
        self.types.mark_builtin_enum(name);
        self.record_type_module(name);
    }

    /// Register a builtin enum whose cases may carry positional associated
    /// values (e.g. `JSONEncoder.NonConformingFloatEncodingStrategy` with
    /// `.convertToString(positiveInfinity:negativeInfinity:nan:)`). Each entry
    /// is `(case_name, &[payload_type_spelling])`; labels are dropped (payload
    /// is positional). Like [`register_builtin_enum`], leading-dot resolution
    /// falls back to these by unique case name.
    pub fn register_builtin_enum_with_payloads(&mut self, name: &str, cases: &[(&str, &[&str])]) {
        if self.types.is_enum(name) {
            return;
        }
        let cases = cases
            .iter()
            .map(|(case, payloads)| EnumCaseDef {
                name: (*case).to_string(),
                raw: None,
                payload_types: payloads.iter().map(|t| Some((*t).to_string())).collect(),
            })
            .collect();
        self.types.insert_enum(
            name.to_string(),
            EnumDef {
                cases,
                methods: std::collections::HashMap::new(),
                computed: std::collections::HashMap::new(),
            },
        );
        self.types.mark_builtin_enum(name);
        self.record_type_module(name);
    }

    /// The keys of every registered standard-library entry, for coverage
    /// tooling. Free functions are bare names; method/property intrinsics are
    /// `Type.member`; sequence algorithms are `Sequence.member`. Sorted and
    /// deduplicated so the output is stable.
    pub fn registered_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        keys.extend(self.globals.free_fn_names().cloned());
        keys.extend(self.builtins.qualified_names());
        for name in self.globals.algorithm_names() {
            keys.push(format!("Sequence.{name}"));
        }
        keys.sort();
        keys.dedup();
        keys
    }

    /// Register a computed-property intrinsic on a builtin receiver type.
    pub fn register_property(&mut self, recv: BuiltinReceiver, name: &str, f: PropertyFn) {
        self.builtins
            .add_property(recv, name, f, self.current_module);
    }

    /// Register a computed-property intrinsic that also receives the receiver's
    /// static type spelling when the dispatch site can recover it.
    pub fn register_typed_property(
        &mut self,
        recv: BuiltinReceiver,
        name: &str,
        f: TypedPropertyFn,
    ) {
        self.builtins
            .add_typed_property(recv, name, f, self.current_module);
    }

    /// Register a computed-property **setter** on a builtin receiver type.
    ///
    /// When the interpreter sees `recv.name = value` for a struct whose type
    /// matches `recv`, the setter is called with `(current_struct, new_value)`
    /// and the returned struct replaces the binding.
    pub fn register_property_setter(
        &mut self,
        recv: BuiltinReceiver,
        name: &str,
        f: PropertySetterFn,
    ) {
        self.builtins.add_setter(recv, name, f, self.current_module);
    }

    /// Register a context-aware computed-property intrinsic on a builtin type.
    pub fn register_contextual_property(
        &mut self,
        recv: BuiltinReceiver,
        name: &str,
        f: ContextualPropertyFn,
    ) {
        self.builtins
            .add_contextual_property(recv, name, f, self.current_module);
    }

    /// Register a static *property value* on a (possibly non-builtin) type, so
    /// `Type.name` and implicit `.name` resolve to it (e.g. `UnitLength.meters`).
    pub fn register_static_value(&mut self, type_name: &str, name: &str, value: SwiftValue) {
        let key = format!("{type_name}.{name}");
        self.statics.insert(key.clone(), value);
        self.static_modules.insert(key, self.current_module);
        self.record_type_module(type_name);
    }

    /// Register a static (type-level) method intrinsic on a builtin type.
    pub fn register_static(&mut self, recv: BuiltinReceiver, name: &str, f: StaticFn) {
        self.builtins
            .add_static_method(recv, name, f, self.current_module);
        self.record_type_module(recv.type_name());
    }

    /// Register a `Sequence`/`Collection` algorithm by method name.
    pub fn register_algorithm(&mut self, name: &str, f: AlgoFn) {
        self.globals.add_algorithm(name, f, self.current_module);
    }

    /// Register a generic method intrinsic dispatched on any struct receiver by
    /// name (the SwiftUI view-modifier seam). Tried only after user-declared
    /// methods and builtin-receiver intrinsics fail to match, so a user method
    /// of the same name always wins.
    pub fn register_struct_method(&mut self, name: &str, f: StructMethodFn) {
        self.globals.add_struct_method(
            name,
            StructMethodEntry {
                f,
                params: None,
                module: self.current_module,
            },
        );
    }

    /// Register a generic struct-method intrinsic together with a declared
    /// parameter signature (the typed SwiftUI modifier seam). The signature
    /// pushes a contextual type while each modifier argument is evaluated so a
    /// leading-dot member resolves against the parameter type
    /// (`.frame(maxWidth: .infinity, alignment: .center)`).
    pub fn register_struct_method_typed(
        &mut self,
        name: &str,
        f: StructMethodFn,
        params: Vec<BuiltinParam>,
    ) {
        let params = params.into_iter().map(BuiltinParam::into_param).collect();
        self.globals.add_struct_method(
            name,
            StructMethodEntry {
                f,
                params: Some(params),
                module: self.current_module,
            },
        );
    }

    /// Register a render-scope hook pair (the generic subtree-scoping seam). The
    /// SwiftUI renderer brackets each custom `View`'s `body` evaluation with a
    /// matched call to `enter` (registration order) and `exit` (reverse order),
    /// each receiving the view value, so a framework can push subtree-scoped
    /// state a modifier carries and restore it afterwards (nearest-ancestor
    /// wins, no leakage across siblings). Core assigns the view value no
    /// meaning — SwiftData uses it to publish/withdraw the environment's
    /// `ModelContext` for `@Query`.
    /// Always-active hook (not import-gated); tagged with the current module
    /// for registry uniformity / future per-module hook management.
    pub fn register_view_scope(
        &mut self,
        enter: crate::stdlib::ViewScopeFn,
        exit: crate::stdlib::ViewScopeFn,
    ) {
        self.view_scopes
            .push(ModuleTagged::new((enter, exit), self.current_module));
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
        self.builtins
            .add_intrinsic(recv, name, entry, self.current_module);
    }

    /// Register a label-aware method intrinsic on a builtin receiver type.
    pub fn register_labeled_intrinsic(
        &mut self,
        recv: BuiltinReceiver,
        name: &str,
        entry: LabeledMethodEntry,
    ) {
        self.builtins
            .add_labeled_intrinsic(recv, name, entry, self.current_module);
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

    /// Predeclare `Result<Success, Failure>` as a two-case enum so
    /// `.success`/`.failure` construct and `.get()` can throw.
    fn register_builtin_result(&mut self) {
        self.types
            .enums
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

    /// Evaluate a program whose [`Analysis`] the interpreter *retains ownership
    /// of* (shared via `Rc`) instead of a caller-leaked `&'static`.
    ///
    /// This is the memory-bounded entry point for a warm-start cache: the cache
    /// can hand the interpreter an `Rc<Analysis>` and later evict (drop) its
    /// own `Rc` freely, because the AST lives exactly as long as the last
    /// interpreter (or SwiftUI session) still using it — no permanent
    /// `Box::leak`. When every holder drops, the AST is freed.
    ///
    /// # Safety model
    ///
    /// Identical to [`FragmentCache`]: the `&'static Analysis` derived here does
    /// **not** live for the process lifetime; it lives as long as the `Rc`
    /// pushed onto `self.retained_analyses`, which is dropped only when the
    /// interpreter drops. `Rc` never moves its pointee, so the address every
    /// stored `Node<'static>` references is stable; the retained `Rc` is never
    /// removed before drop; and `Node` cursors carry no `Drop`, so drop-order
    /// against other interpreter fields is irrelevant.
    pub fn run_retaining(&mut self, analysis: Rc<Analysis>) -> Result<(), EvalError> {
        // SAFETY: see the doc comment above — the retained `Rc` keeps this
        // allocation alive (at a stable address) for the interpreter's entire
        // lifetime, which outlives every `Node<'static>` derived from it.
        let static_ref: &'static Analysis = unsafe { &*Rc::as_ptr(&analysis) };
        self.retained_analyses.push(analysis);
        self.run(static_ref)
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
            | NodeKind::MacroDecl
            | NodeKind::ImportDecl => Ok(SwiftValue::Void), // hoisted (import set); no runtime effect
            NodeKind::ClosureExpr => self.eval_closure(node),
            NodeKind::CastExpr => self.eval_cast(node),
            NodeKind::AwaitExpr => self.eval_await(node),
            NodeKind::ReturnStmt => {
                let value = match node.first_child() {
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
                if let Some(block) = node.first_child() {
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
        let body = node.find_child(NodeKind::Block);
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

        // A variable with accessor bodies — computed `get`/`set` or observers
        // `willSet`/`didSet` — routes reads and writes through those bodies
        // (TSPL: computed properties and observers are also available to
        // global and local variables).
        let acc = node.var_accessors();
        if acc.is_computed || acc.will_set_body.is_some() || acc.did_set_body.is_some() {
            return self.declare_accessor_var(&name, &acc, init_expr, node);
        }

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
        let decl_ty = node
            .children()
            .find(|c| c.kind() == NodeKind::TypeRef)
            .and_then(|c| c.text())
            // No explicit annotation: infer the static type from the
            // initializer, but record it only when optional so declared-type
            // aware dispatch (`x.take()`) and optional-aware printing see it
            // without over-tagging plain scalars.
            .or_else(|| {
                init_expr.and_then(|init| {
                    self.static_type_of(&init)
                        .filter(|ty| TypeRepr::parse(ty).is_optional())
                })
            });
        self.env.declare(&name, value, mutable);
        if let Some(ty) = decl_ty {
            self.env.set_declared_type(&name, ty);
        }
        Ok(SwiftValue::Void)
    }

    /// Declare a global/local variable whose reads/writes run accessor bodies:
    /// computed (`get`/`set`) or stored with observers (`willSet`/`didSet`).
    /// Each body becomes a closure capturing the declaration's scope chain, so
    /// it sees surrounding variables live (and, for observers, the variable
    /// itself through its marker binding).
    fn declare_accessor_var(
        &mut self,
        name: &str,
        acc: &tswift_frontend::VarAccessors<'static>,
        init_expr: Option<Node<'static>>,
        node: &Node<'static>,
    ) -> Eval {
        let getter = acc.getter_body.map(|b| self.accessor_closure(None, &b));
        let setter = acc.setter_body.map(|b| {
            self.accessor_closure(Some(acc.setter_param.as_deref().unwrap_or("newValue")), &b)
        });
        let will_set = acc.will_set_body.map(|b| {
            self.accessor_closure(
                Some(acc.will_set_param.as_deref().unwrap_or("newValue")),
                &b,
            )
        });
        let did_set = acc.did_set_body.map(|b| {
            self.accessor_closure(Some(acc.did_set_param.as_deref().unwrap_or("oldValue")), &b)
        });

        // An observed stored variable evaluates its initializer up front;
        // observers do not fire for the initial value (TSPL). A computed
        // variable has no storage.
        let storage = if acc.is_computed {
            None
        } else {
            let value = match init_expr {
                Some(init) => {
                    let v = self.eval(&init)?;
                    let v = self.coerce_to_literal_type(node, v)?;
                    self.coerce_to_decl_type(node, v)
                }
                None => SwiftValue::Void,
            };
            Some(Rc::new(RefCell::new(value)))
        };

        let ty = node
            .children()
            .find(|c| c.kind() == NodeKind::TypeRef)
            .and_then(|c| c.text());
        let idx = self.accessor_vars.len();
        self.accessor_vars.push(AccessorVarSlot {
            name: name.to_string(),
            ty,
            getter,
            setter,
            will_set,
            did_set,
            storage,
        });
        self.env.declare(name, SwiftValue::AccessorVar(idx), true);
        Ok(SwiftValue::Void)
    }

    /// Build a closure for an accessor body: zero parameters for a getter, one
    /// (the accessor's named or implicit parameter) otherwise.
    fn accessor_closure(&mut self, param: Option<&str>, body: &Node<'static>) -> usize {
        let params = param
            .map(|p| {
                vec![Param {
                    label: None,
                    name: p.to_string(),
                    ty: None,
                    variadic: false,
                    inout_: false,
                    autoclosure: false,
                    default: None,
                }]
            })
            .unwrap_or_default();
        let body = expand_directive_list(body.children().collect());
        let id = self.closures.len();
        self.closures
            .push((ClosureDef::User { params, body }, self.env.capture()));
        id
    }

    /// Read a variable bound to an accessor slot: run the computed getter, or
    /// return the observed variable's backing storage.
    fn read_accessor_var(&mut self, idx: usize) -> Eval {
        if let Some(g) = self.accessor_vars[idx].getter {
            return self.call_closure(g, Vec::new());
        }
        if let Some(st) = &self.accessor_vars[idx].storage {
            return Ok(st.borrow().clone());
        }
        let name = self.accessor_vars[idx].name.clone();
        Err(EvalError::Unsupported(format!("variable `{name}` has no getter")).into())
    }

    /// Write a variable bound to an accessor slot: run the computed setter, or
    /// update the observed variable's storage firing `willSet`/`didSet`.
    fn write_accessor_var(&mut self, idx: usize, value: SwiftValue) -> Result<(), Signal> {
        let slot = &self.accessor_vars[idx];
        let (setter, will_set, did_set, storage) = (
            slot.setter,
            slot.will_set,
            slot.did_set,
            slot.storage.clone(),
        );
        // The written value adopts the variable's annotated scalar type
        // (`fahrenheit = 212` passes `newValue` as 212.0 when `: Double`).
        let value = match &slot.ty {
            Some(ty) => coerce_int_to_type_name(ty, &value).unwrap_or(value),
            None => value,
        };
        if let Some(st) = storage {
            let old = st.borrow().clone();
            if let Some(w) = will_set {
                self.call_closure(w, vec![value.clone()])?;
            }
            *st.borrow_mut() = value;
            if let Some(d) = did_set {
                self.call_closure(d, vec![old])?;
            }
            return Ok(());
        }
        match setter {
            Some(s) => {
                self.call_closure(s, vec![value])?;
                Ok(())
            }
            None => {
                let name = self.accessor_vars[idx].name.clone();
                Err(EvalError::Unsupported(format!(
                    "cannot assign to `{name}`: it is a get-only computed variable"
                ))
                .into())
            }
        }
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
        let repr = TypeRepr::parse(&ty);
        // An optional annotation (`T?`): a `nil` literal stays the absent
        // optional rather than constructing `T(nilLiteral:)`.
        let optional = repr.is_optional();
        if optional && init_kind == NodeKind::NilLiteral {
            return Ok(value);
        }
        let ty = repr.strip_optionals().text();
        let is_user_type = self.types.is_struct(ty) || self.types.is_class(ty);
        if !is_user_type {
            return Ok(value);
        }
        self.coerce_literal_value(ty, init_kind, value)
    }

    /// Convert a literal value into the contextual user type named by `ty` when
    /// that type declares the matching `ExpressibleBy*Literal` conformance.
    fn coerce_literal_value(&mut self, ty: &str, init_kind: NodeKind, value: SwiftValue) -> Eval {
        let is_user_type = self.types.is_struct(ty) || self.types.is_class(ty);
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
        if self.types.is_struct(ty) {
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
        for child in node.children() {
            if child.kind() == NodeKind::TypeRef {
                if let Some(coerced) = child
                    .text()
                    .and_then(|ty| coerce_int_to_type_name(&ty, &value))
                {
                    return coerced;
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
            if let SwiftValue::AccessorVar(idx) = v {
                return self.read_accessor_var(idx);
            }
            return self.upgrade_reference(v, &name);
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
            if let SwiftValue::AccessorVar(idx) = v {
                return self.read_accessor_var(idx);
            }
            return self.upgrade_reference(v, &name);
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

    /// Reading a non-strong reference binding resolves it at use: a `weak`
    /// binding zeroes to `nil` once its referent deallocates; an `unowned`
    /// binding traps instead (matching Swift's dangling-unowned crash).
    fn upgrade_reference(&self, value: SwiftValue, name: &str) -> Eval {
        match value {
            SwiftValue::Weak(w) => Ok(w
                .upgrade()
                .map(SwiftValue::Object)
                .unwrap_or(SwiftValue::Nil)),
            SwiftValue::Unowned(w) => w.upgrade().map(SwiftValue::Object).ok_or_else(|| {
                trap(format!(
                    "attempted to read an unowned reference `{name}` but the object was already deallocated"
                ))
            }),
            v => Ok(v),
        }
    }

    /// If executing inside a `static` method, read `name` as a static property
    /// of the enclosing type (`Type.name`), if such a static exists.
    fn implicit_static_member(&self, name: &str) -> Option<SwiftValue> {
        let ty = self.static_ctx.last()?;
        let key = format!("{ty}.{name}");
        if !self.static_key_visible(&key) {
            return None;
        }
        self.statics.get(&key).cloned()
    }

    /// The `statics` key for an unqualified `name` referencing a static property
    /// of the enclosing `static` method's type, when one exists.
    fn implicit_static_key(&self, name: &str) -> Option<String> {
        let ty = self.static_ctx.last()?;
        let key = format!("{ty}.{name}");
        (self.statics.contains_key(&key) && self.static_key_visible(&key)).then_some(key)
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
                    if let Some((func, module)) = self
                        .builtins
                        .contextual_property(kind, name)
                        .map(|t| (t.value, t.module))
                    {
                        if self.module_symbol_visible(module) {
                            return func(self, this)
                                .map(Some)
                                .map_err(Self::std_error_to_signal);
                        }
                    }
                    if let Some((func, module)) = self
                        .builtins
                        .property(kind, name)
                        .map(|t| (t.value, t.module))
                    {
                        if self.module_symbol_visible(module) {
                            return func(this).map(Some).map_err(Self::std_error_to_signal);
                        }
                    }
                }
                let tn = this.type_name();
                if let Some(body) = self
                    .types
                    .builtin_ext_computed(&tn, name)
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

    /// Whether `name` is free to resolve to a builtin function: no local value
    /// binding shadows it. A user binding of the same name always wins, so this
    /// guards every hardcoded builtin (`swap`, `type(of:)`, collection
    /// constructors, …) against being mistaken for a user value.
    fn is_unshadowed(&self, name: &str) -> bool {
        self.env.get(name).is_none()
    }

    /// Whether `name` is a user-declared nominal type (struct, class, or enum).
    /// Such a name dispatches to its own initializer, so it must not inherit a
    /// builtin of the same spelling.
    fn is_user_nominal_type(&self, name: &str) -> bool {
        self.types.is_nominal(name)
    }

    /// Resolve an identifier used as a type in `Type.member` / `Type.method(...)`
    /// position. Applies `Self`-keyword and generic-placeholder substitution,
    /// then gates on shadowing: a name bound to a local value (other than the
    /// `Self` keyword, which is never a value) is not a usable type reference
    /// and yields `None`. On success the resolved concrete name is returned with
    /// a flag for whether it names a user struct/class/enum.
    ///
    /// Shared by [`Self::eval_member`] and `eval_method_call` so both dispatch
    /// paths derive the type reference identically.
    fn resolve_type_reference(&self, text: &str) -> Option<TypeReference> {
        let is_self_kw = text == "Self";
        let name = self.resolve_self_keyword(text.to_string());
        let name = self.resolve_type_alias(&name).unwrap_or(name);
        if is_self_kw || self.is_unshadowed(&name) {
            let user_defined = self.is_user_nominal_type(&name);
            Some(TypeReference { name, user_defined })
        } else {
            None
        }
    }

    /// Whether `name` spells a known type — a user struct/class/enum/protocol
    /// or a builtin value type — so `Type.self` resolves to a metatype.
    fn is_type_name(&self, name: &str) -> bool {
        self.types.is_struct(name)
            || self.types.is_class(name)
            || self.types.is_enum(name)
            || self.types.is_protocol(name)
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
        let payload_types = self.types.enum_def(type_name).and_then(|d| {
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

    /// The statically inferred type *spelling* of an expression node, when one
    /// can be recovered cheaply — enough for type-directed `print` rendering to
    /// know an argument was optional. Returns `None` when nothing optional is
    /// in play (callers then fall back to plain value rendering).
    ///
    /// Covered: identifiers (via the declared-type map), literals, `Optional(x)`
    /// calls, and array literals whose elements are statically optional.
    fn static_type_of(&self, node: &Node<'static>) -> Option<String> {
        match node.kind() {
            NodeKind::NilLiteral => Some("_?".to_string()),
            NodeKind::StringLiteral => Some("String".to_string()),
            NodeKind::IntegerLiteral => Some("Int".to_string()),
            NodeKind::FloatLiteral => Some("Double".to_string()),
            NodeKind::BoolLiteral => Some("Bool".to_string()),
            NodeKind::IdentExpr => node.text().and_then(|n| self.env.declared_type(&n)),
            NodeKind::CallExpr => self.static_type_of_call(node),
            NodeKind::ArrayLiteral => self.static_type_of_array(node),
            NodeKind::MemberExpr if node.is_optional_chain() => {
                // Optional chaining yields an optional result (`s?.count` is
                // `Int?`), so print renders it `Optional(...)`. Use the member's
                // own declared type when known, else the unknown-optional base.
                let base = self
                    .static_type_of_member(node)
                    .map(|t| TypeRepr::parse(&t).strip_optionals().text().to_string())
                    .unwrap_or_else(|| "_".to_string());
                Some(format!("{base}?"))
            }
            NodeKind::MemberExpr => self.static_type_of_member(node),
            _ => None,
        }
    }

    /// Static type of a `base.member` access, recovered from `base`'s
    /// struct/class stored-property declaration (whose written TypeRef spelling
    /// — e.g. `Int?` — is preserved on the `StoredProp`). Lets `b.x.take()` and
    /// `self.x.take()` enter Optional dispatch. Plain (non-`?.`) access only.
    fn static_type_of_member(&self, node: &Node<'static>) -> Option<String> {
        let member = self.member_name(node)?;
        let base = node.first_child()?;
        let type_name = self.receiver_type_name(&base)?;
        let stored_ty = |props: &[StoredProp]| {
            props
                .iter()
                .find(|p| p.name == member)
                .and_then(|p| p.ty.clone())
        };
        if let Some(def) = self.types.struct_def(&type_name) {
            return stored_ty(&def.stored);
        }
        if let Some(def) = self.types.class_def(&type_name) {
            return stored_ty(&def.stored);
        }
        None
    }

    /// The runtime type name of a receiver expression, when it can be read
    /// without side effects — an in-scope binding (`b`, `self`) or a nested
    /// member whose declared type names a known struct/class.
    fn receiver_type_name(&self, node: &Node<'static>) -> Option<String> {
        match node.kind() {
            NodeKind::IdentExpr => Some(self.env.get(&node.text()?)?.type_name()),
            NodeKind::MemberExpr => {
                let ty = self.static_type_of_member(node)?;
                let stripped = TypeRepr::parse(&ty).strip_optionals().text().to_string();
                Some(stripped)
            }
            _ => None,
        }
    }

    /// Static type of a call expression — `Optional(x)` (wrapped type made
    /// optional) and `opt.take()` (returns `Wrapped?`, i.e. the receiver's own
    /// optional type), so `let t = x.take()` renders as `Optional(...)`.
    fn static_type_of_call(&self, node: &Node<'static>) -> Option<String> {
        let children: Vec<Node<'static>> = node.children().collect();
        let callee = children.first()?;
        if callee.kind() == NodeKind::IdentExpr && callee.text().as_deref() == Some("Optional") {
            let inner = children.get(1);
            let base = inner
                .and_then(|n| self.static_type_of(n))
                .unwrap_or_else(|| "_".to_string());
            return Some(format!("{base}?"));
        }
        // `receiver.take()` yields the receiver's own optional type.
        if callee.kind() == NodeKind::MemberExpr
            && self.member_name(callee).as_deref() == Some("take")
        {
            if let Some(recv) = callee.first_child() {
                let ty = self.static_type_of(&recv)?;
                if TypeRepr::parse(&ty).is_optional() {
                    return Some(ty);
                }
            }
        }
        None
    }

    /// The member name of a `MemberExpr`, resolving the bare-`.` operator slot.
    fn member_name(&self, member: &Node<'static>) -> Option<String> {
        match member.text() {
            Some(name) if name == "." => member.op_text(),
            other => other,
        }
    }

    /// Whether a receiver expression's static type is a (top-level) optional.
    /// Gates declared-type-aware `Optional` member dispatch (#242).
    fn receiver_is_optional(&self, base: &Node<'static>) -> bool {
        self.static_type_of(base)
            .is_some_and(|ty| TypeRepr::parse(&ty).is_optional())
    }

    /// Static type of an array literal. Prefers a contextual array type; else
    /// infers `[Base?]` when any element is statically optional (a `nil`
    /// literal, an `Optional(…)` call, …). Returns `None` for a plainly
    /// non-optional array so it renders unchanged.
    fn static_type_of_array(&self, node: &Node<'static>) -> Option<String> {
        if let Some(hint) = self.contextual_type() {
            let repr = TypeRepr::parse(hint);
            if repr.array_element().is_some() {
                return Some(hint.to_string());
            }
        }
        // Merge the element spellings. `any_direct_optional` means an element
        // is itself optional (a `nil` / `Optional(…)` — append a `?` to the
        // base); `any_optional_anywhere` also fires when the optionality is
        // nested inside an element (e.g. `[Int?]` inside an outer array), which
        // is already carried by the element spelling, so no extra `?`.
        let mut any_direct_optional = false;
        let mut any_optional_anywhere = false;
        let mut base: Option<String> = None;
        for elem in node.children() {
            if let Some(t) = self.static_type_of(&elem) {
                let repr = TypeRepr::parse(&t);
                if repr.is_optional() {
                    any_direct_optional = true;
                }
                if repr.contains_optional() {
                    any_optional_anywhere = true;
                }
                let stripped = repr.strip_optionals().text();
                if stripped != "_" {
                    base = Some(stripped.to_string());
                }
            }
        }
        if !any_optional_anywhere {
            return None;
        }
        let base = base.as_deref().unwrap_or("_");
        if any_direct_optional {
            Some(format!("[{base}?]"))
        } else {
            Some(format!("[{base}]"))
        }
    }

    /// Resolve the enum type for a shorthand `.case` member from the resolved
    /// type or call-site contextual type, falling back to the unique enum
    /// declaring that case. Framework builtin enums honor strict import-gating
    /// (`type_name_visible`).
    fn resolve_member_enum(&self, member: &Node<'static>, case: &str) -> Option<String> {
        // The member's resolved type (the enum or a function returning it), then
        // the call-site contextual type; match a registered enum name within.
        for ty in member
            .type_name()
            .into_iter()
            .chain(self.contextual_type().map(String::from))
        {
            for name in self.types.enum_names() {
                // A plain type spelling (`Style`) matches a split token;
                // a dotted builtin-enum spelling (`URLRequest.NetworkServiceType`)
                // only ever appears as the *whole* contextual type string, since
                // dots split it into separate tokens above.
                if (ty == name.as_str()
                    || ty
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .any(|t| t == name))
                    && self.enum_has_case(name, case)
                    && self.type_name_visible(name)
                {
                    return Some(name.clone());
                }
            }
        }
        // Fall back: a single *user-declared* enum declaring this case name.
        // Builtin enums are excluded here so they cannot shadow SwiftUI
        // implicit statics; they get a later, lower-priority fallback.
        let mut found = None;
        for (name, def) in &self.types.enums {
            if self.types.is_builtin_enum(name) {
                continue;
            }
            if def.cases.iter().any(|c| c.name == case) {
                if found.is_some() {
                    return None; // ambiguous
                }
                found = Some(name.clone());
            }
        }
        found
    }

    /// If a contextual type names a framework enum that owns `case` but is
    /// gated out by strict imports, return the standard import-hint diagnostic
    /// (instead of a generic "unresolved type" error).
    fn gated_contextual_enum_error(&self, member: &Node<'static>, case: &str) -> Option<EvalError> {
        if !self.strict_imports {
            return None;
        }
        for ty in member
            .type_name()
            .into_iter()
            .chain(self.contextual_type().map(String::from))
        {
            for name in self.types.enum_names() {
                // Plain token match or whole dotted builtin-enum spelling.
                let matches = ty == name.as_str()
                    || ty
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .any(|t| t == name);
                if matches && self.enum_has_case(name, case) {
                    if let Some(module) = self.type_modules.get(name.as_str()).copied() {
                        if !self.module_symbol_visible(module) {
                            return Some(not_in_scope_error(case, module));
                        }
                    }
                }
            }
        }
        None
    }

    /// Last-resort shorthand resolution over builtin enums only (`.year`,
    /// `.gregorian`, `.plain` for `Decimal.RoundingMode`). Runs after implicit
    /// statics so SwiftUI styles keep priority.
    ///
    /// When several builtin enums share a case name (e.g. `.none` on both
    /// `DateFormatter.Style` and `NumberFormatter.Style`), the lexicographically
    /// smallest type name is chosen deterministically. Builtin consumers match
    /// on the case string, not the enum type, so this stays correct for the
    /// shared style/`none` cases while keeping `==` results stable.
    fn resolve_builtin_member_enum(&self, case: &str) -> Option<String> {
        self.types
            .builtin_enum_names()
            .filter(|name| self.enum_has_case(name, case) && self.type_name_visible(name))
            .min()
            .cloned()
    }

    /// Resolve an implicit-member `.name` to a static property. Prefers the
    /// member node's inferred contextual type; otherwise accepts a unique
    /// registered static whose member name matches.
    ///
    /// When the contextual type owns a registered static that is gated out by
    /// strict imports, returns the standard import-hint diagnostic — does
    /// **not** fall through to another module's same-named static.
    fn resolve_implicit_static(
        &self,
        node: &Node<'static>,
        name: &str,
    ) -> Result<Option<SwiftValue>, EvalError> {
        // The node's inferred type, then the call-site contextual type.
        for ty in node
            .type_name()
            .into_iter()
            .chain(self.contextual_type().map(String::from))
        {
            for type_name in ty.split(|c: char| !c.is_alphanumeric() && c != '_') {
                let key = format!("{type_name}.{name}");
                if self.statics.contains_key(&key) {
                    // Contextual type owns this static: gate it (do not steal
                    // a visible same-named static from another module).
                    self.gate_static_key(&key, name)?;
                    if let Some(v) = self.statics.get(&key) {
                        return Ok(Some(v.clone()));
                    }
                }
            }
        }
        // Otherwise, a unique `Type.name` static across all registered types.
        let suffix = format!(".{name}");
        let mut found: Option<&SwiftValue> = None;
        for (key, value) in &self.statics {
            if key.ends_with(&suffix) && self.static_key_visible(key) {
                if found.is_some() {
                    return Ok(None); // ambiguous
                }
                found = Some(value);
            }
        }
        Ok(found.cloned())
    }

    /// Resolve an implicit-member call `.m(...)` to the contextual type that
    /// declares a `static`/`class` method named `m`. Prefers the node's
    /// inferred type; otherwise accepts a unique struct/class/enum that
    /// declares such a static method.
    fn resolve_implicit_static_method(&self, node: &Node<'static>, method: &str) -> Option<String> {
        let declares = |type_name: &str| -> bool {
            let m = self
                .types
                .struct_def(type_name)
                .map(|d| &d.methods)
                .or_else(|| self.types.class_def(type_name).map(|d| &d.methods))
                .or_else(|| self.types.enum_def(type_name).map(|d| &d.methods));
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
            .types
            .struct_names()
            .chain(self.types.class_names())
            .chain(self.types.enum_names());
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
        self.types
            .enum_def(type_name)
            .is_some_and(|d| d.cases.iter().any(|c| c.name == name))
    }

    /// The `rawValue` of an enum case (precomputed at registration).
    fn enum_raw_value(&mut self, type_name: &str, case: &str) -> Eval {
        let raw = self
            .types
            .enum_def(type_name)
            .and_then(|d| d.cases.iter().find(|c| c.name == case))
            .and_then(|c| c.raw.clone());
        raw.ok_or_else(|| EvalError::Type(format!("{type_name}.{case} has no raw value")).into())
    }

    /// All cases of an enum as an array (`CaseIterable.allCases`).
    fn enum_all_cases(&mut self, type_name: &str) -> Eval {
        let names: Vec<String> = self
            .types
            .enum_def(type_name)
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
            .types
            .struct_def(type_name)
            .map(|d| &d.computed)
            .or_else(|| self.types.class_def(type_name).map(|d| &d.computed))
            .or_else(|| self.types.enum_def(type_name).map(|d| &d.computed))
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

    // ----- Classes (reference semantics + ARC) -----

    /// The class inheritance chain, root superclass first.
    fn class_chain(&self, class_name: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut current = Some(class_name.to_string());
        while let Some(name) = current {
            if !self.types.is_class(&name) {
                break;
            }
            current = self.types.class_def(&name).unwrap().superclass.clone();
            chain.push(name);
        }
        chain.reverse();
        chain
    }

    /// Whether `sub` is `super` or a descendant of it.
    fn class_is(&self, sub: &str, super_: &str) -> bool {
        self.class_chain(sub).iter().any(|c| c == super_)
    }

    /// Instantiate a class: lay out fields from the whole chain, then run init.
    fn instantiate_class(&mut self, class_name: &str, args: Vec<CallArg>) -> Eval {
        let chain = self.class_chain(class_name);
        let mut fields: Vec<(String, SwiftValue)> = Vec::new();
        for cls in &chain {
            let props: Vec<(String, Option<Node<'static>>)> = self
                .types
                .class_def(cls)
                .unwrap()
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

        // Select the initializer: the closest class (most-derived first) whose
        // declared initializers label-match the call — finding an *inherited*
        // convenience/designated init when the subclass declares its own with
        // different labels — falling back to the most-derived class with any
        // initializer.
        let selected = chain.iter().rev().find_map(|c| {
            let def = self.types.class_def(c)?;
            select_labeled_overload(&def.init_overloads, &args)
                .map(|m| (c.clone(), clone_params(&m.params), m.body))
        });
        let init_owner = selected.as_ref().map(|(c, ..)| c.clone()).or_else(|| {
            chain
                .iter()
                .rev()
                .find(|c| self.types.class_def(c).unwrap().init.is_some())
                .cloned()
        });
        if let Some(owner) = init_owner {
            let (params, body) = match selected {
                Some((_, params, body)) => (params, body),
                None => {
                    let m = self.types.class_def(&owner).unwrap().init.as_ref().unwrap();
                    (clone_params(&m.params), m.body)
                }
            };
            self.class_ctx.push(owner);
            let saved_env = self.env.enter_isolated();
            self.env.declare("self", value.clone(), false);
            let bound = self.bind_params(&params, args);
            self.init_ctx += 1;
            let result = match bound {
                Ok(_) => match body {
                    Some(b) => self.eval(&b),
                    None => Ok(SwiftValue::Void),
                },
                Err(e) => Err(e),
            };
            self.init_ctx -= 1;
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
            if let Some(body) = self.types.class_def(&cls).and_then(|d| d.deinit) {
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
        // Names captured `weak`/`unowned`, whose strong bindings must not be
        // retained through the captured scope chain.
        let mut nonstrong_names: Vec<String> = Vec::new();
        // Source bindings of alias captures (`[weak owner = self]` → "self"),
        // prunable when the body does not itself reference them.
        let mut alias_sources: Vec<String> = Vec::new();
        for child in node.children() {
            match child.kind() {
                NodeKind::Param => {
                    // Closure params have no external label; the name is the
                    // first token (param_info's two-name heuristic mistakes the
                    // trailing `in` keyword for an internal name).
                    let name = child.text().unwrap_or_default();
                    let info = child.param_info();
                    let ty = child
                        .children()
                        .find(|c| c.kind() == NodeKind::TypeRef)
                        .and_then(|c| c.text());
                    params.push(Param {
                        label: None,
                        name,
                        ty,
                        variadic: info.variadic,
                        inout_: info.is_inout,
                        autoclosure: false,
                        default: None,
                    });
                    last_param = Some(child);
                }
                NodeKind::ClosureCapture => {
                    if let Some(name) = child.text() {
                        let v = match child.first_child() {
                            Some(expr) => self.eval(&expr)?,
                            // A capture list snapshots the *current value*; an
                            // accessor-backed variable snapshots via its getter
                            // or storage, never the marker itself.
                            None => match self.env.get(&name) {
                                Some(SwiftValue::AccessorVar(idx)) => {
                                    self.read_accessor_var(idx)?
                                }
                                other => other.unwrap_or(SwiftValue::Nil),
                            },
                        };
                        // `[weak x]` / `[unowned x]` capture without retaining:
                        // the binding holds a non-strong reference, upgraded
                        // (or zeroed / trapped) at each read.
                        let mods = child.modifier_names();
                        let nonstrong = mods.contains(&"weak") || mods.contains(&"unowned");
                        let v = match &v {
                            SwiftValue::Object(rc) if mods.contains(&"weak") => {
                                nonstrong_names.push(name.clone());
                                SwiftValue::Weak(StdRc::downgrade(rc))
                            }
                            SwiftValue::Object(rc) if mods.contains(&"unowned") => {
                                nonstrong_names.push(name.clone());
                                SwiftValue::Unowned(StdRc::downgrade(rc))
                            }
                            _ => v,
                        };
                        // An alias capture `[weak owner = self]` names its
                        // source binding, which the chain would still retain;
                        // remember the source so it can be pruned too when the
                        // body never references it directly.
                        if nonstrong {
                            if let Some(src) = child
                                .first_child()
                                .filter(|e| e.kind() == NodeKind::IdentExpr)
                                .and_then(|e| e.text())
                            {
                                alias_sources.push(src);
                            }
                        }
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

        // An alias capture's source is prunable only when the body never
        // references it — a direct reference is an implicit *strong* capture
        // (Swift retains it) and must keep the shared chain binding.
        for src in alias_sources {
            if !body.iter().any(|n| subtree_references_name(n, &src)) {
                nonstrong_names.push(src);
            }
        }

        let mut captured = self.env.capture();
        // A `weak`/`unowned` capture must not retain the instance through the
        // captured scope chain (or `[weak self]` would never zero: the chain's
        // strong `self` would keep the object alive as long as the closure).
        // A scope binding such a name is replaced in this closure's chain by a
        // clone of the scope *map* without it — every other binding keeps live
        // mutation sharing through its own cell, and the executing method's
        // original scope is untouched.
        if !nonstrong_names.is_empty() {
            for scope in captured.iter_mut() {
                let has_pruned = scope
                    .borrow()
                    .keys()
                    .any(|k| nonstrong_names.iter().any(|n| n == k));
                if has_pruned {
                    let pruned: HashMap<String, crate::env::BindingCell> = scope
                        .borrow()
                        .iter()
                        .filter(|(k, _)| !nonstrong_names.iter().any(|n| n == *k))
                        .map(|(k, c)| (k.clone(), c.clone()))
                        .collect();
                    *scope = StdRc::new(RefCell::new(pruned));
                }
            }
        }
        if !captured_overrides.is_empty() {
            let scope: Scope = Default::default();
            for (name, v) in captured_overrides {
                scope.borrow_mut().insert(
                    name,
                    StdRc::new(RefCell::new(crate::env::Binding {
                        value: v,
                        mutable: false,
                        declared_ty: None,
                    })),
                );
            }
            captured.push(scope);
        }
        let id = self.closures.len();
        self.closures
            .push((ClosureDef::User { params, body }, captured));
        Ok(SwiftValue::Closure(id))
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
        self.bind_closure_args(&params, &body, &args);
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
        // A cast to an *optional* type (`b as A?`, `5 as Int?`) checks against
        // the wrapped type; nil always casts to an optional type (a present
        // optional is its wrapped value in this runtime's model).
        let repr = TypeRepr::parse(&ty);
        let target = repr.strip_optionals().text();
        let matches = (repr.is_optional() && matches!(value, SwiftValue::Nil))
            || self.value_is_type(&value, target);
        // `as?` yields an optional; `is` yields Bool.
        let optional = node.is_optional_cast();
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
            // Substring is NOT a subtype of String in Swift; String(sub) is
            // required for conversion.  Only the exact "Substring" name matches.
            SwiftValue::Substring { .. } => type_name == "Substring",
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
                cur = n.first_child();
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
        // A framework-registered freestanding macro (`#Predicate`, …) takes
        // priority: strip a leading `#` from the directive spelling and look it
        // up. The handler inspects the node's children (generic type args +
        // trailing-closure body) itself.
        let macro_name = which.strip_prefix('#').unwrap_or(&which);
        if let Some((handler, module)) = self.macros.get(macro_name).map(|t| (t.value, t.module)) {
            self.gate_module_symbol(macro_name, module)?;
            return handler(self, node).map_err(Self::std_error_to_signal);
        }
        match which.as_str() {
            "file" | "filePath" | "fileID" => Ok(SwiftValue::Str(self.filename.clone())),
            "line" => Ok(SwiftValue::int(node.line() as i128)),
            "column" => Ok(SwiftValue::int(0)),
            // `#selector(Type.method)` yields the method name (Swift prints a
            // selector as its name); `#keyPath(Type.a.b)` yields the dotted key
            // path string relative to the root type.
            "selector" => {
                let chain = node.first_child().map(|c| Self::member_chain(&c));
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

    /// `value!` — force-unwrap an optional, trapping on nil.
    fn eval_force_unwrap(&mut self, node: &Node<'static>) -> Eval {
        let inner = node
            .children()
            .next()
            .ok_or_else(|| EvalError::Unsupported("force-unwrap without operand".into()))?;
        let v = self.eval(&inner)?;
        // A user-declared postfix operator (`90.0°`) is a function named
        // after it, applied to the single operand.
        let op = node.text();
        if let Some(op) = op.filter(|o| o != "!") {
            if let Some(SwiftValue::Function(id)) = self.env.get(&op) {
                return self.call_function(
                    id,
                    vec![CallArg {
                        label: None,
                        value: v,
                        place: None,
                    }],
                );
            }
            return Err(EvalError::UnknownFunction(format!("postfix {op}")).into());
        }
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
        // When the contextual type is an array `[T]`, evaluate each element
        // under the element type `T` so a leading-dot member resolves against it
        // (`columns: [.flexible(), .fixed(80)]` → `GridItem`).
        let elem_hint = self
            .contextual_type()
            .and_then(array_element_type)
            .map(str::to_string);
        let mut items = Vec::new();
        for child in node.children() {
            if let Some(ref ty) = elem_hint {
                self.type_hint.push(Some(ty.clone()));
            }
            let item = self.eval(&child);
            if elem_hint.is_some() {
                self.type_hint.pop();
            }
            items.push(item?);
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

    /// Whether `value` is an instance of a `@dynamicCallable` struct type.
    fn is_dynamic_callable(&self, value: &SwiftValue) -> bool {
        matches!(value, SwiftValue::Struct(obj)
            if self.types.struct_def(&obj.type_name).is_some_and(|d| d.dynamic_callable))
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
            .types
            .struct_def(&type_name)
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
        let def = self.types.struct_def(type_name)?;
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
        self.instantiate_struct_specialized(type_name, args, &[])
    }

    /// Build a struct instance, additionally binding integer generic
    /// parameters (`Buf<let N: Int>` specialized as `Buf<4>()`): each value
    /// becomes an immutable stored field, and is in scope while stored-property
    /// defaults evaluate.
    fn instantiate_struct_specialized(
        &mut self,
        type_name: &str,
        args: &[(Option<String>, SwiftValue)],
        type_values: &[(String, SwiftValue)],
    ) -> Eval {
        // A custom initializer runs against a fresh empty value, binding `self`.
        let custom_init = self.select_struct_init(type_name, args);
        if let Some((params, body)) = custom_init {
            // Stored properties with a default are initialized before the
            // initializer body runs (Swift gives each such property its default
            // value first; the body may then reassign it).
            let defaults: Vec<(String, Node<'static>)> = self
                .types
                .struct_def(type_name)
                .map(|d| {
                    d.stored
                        .iter()
                        .filter(|p| !p.lazy)
                        .filter_map(|p| p.default.map(|def| (p.name.clone(), def)))
                        .collect()
                })
                .unwrap_or_default();
            let mut fields: Vec<(String, SwiftValue)> = type_values.to_vec();
            let defaults_result = self.with_type_values(type_values, |me| {
                let mut built: Vec<(String, SwiftValue)> = Vec::new();
                for (pname, def) in defaults {
                    let value = me.eval(&def)?;
                    // Wrap `@propertyWrapper` fields in their wrapper instance,
                    // the same way the memberwise initializer does.
                    let wrapper = me
                        .types
                        .struct_def(type_name)
                        .and_then(|d| d.wrappers.get(&pname))
                        .cloned();
                    let value = match wrapper {
                        Some(wt) => {
                            me.instantiate_struct(&wt, &[(Some("wrappedValue".into()), value)])?
                        }
                        None => value,
                    };
                    built.push((pname, value));
                }
                Ok(built)
            });
            fields.extend(defaults_result?);
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
            // Depth-guarded: `self.init` delegation re-enters here, and a
            // self-recursive delegate must trap instead of overflowing the
            // native stack.
            self.depth += 1;
            if self.depth > MAX_CALL_DEPTH {
                self.depth -= 1;
                return Err(trap(
                    "stack overflow: initializer delegation too deep".into(),
                ));
            }
            let saved_env = self.env.enter_isolated();
            self.env.declare("self", this, true);
            let bound = self.bind_params(&params, call_args);
            self.init_ctx += 1;
            let result = match bound {
                Ok(_) => match body {
                    Some(b) => self.eval(&b),
                    None => Ok(SwiftValue::Void),
                },
                Err(e) => Err(e),
            };
            self.init_ctx -= 1;
            let built = self.env.get("self").unwrap_or(SwiftValue::Void);
            self.env.restore(saved_env);
            self.depth -= 1;
            return match result {
                // A failable initializer that runs `return nil` produces the
                // absent optional rather than the half-built value.
                Err(Signal::Return(SwiftValue::Nil)) => Ok(SwiftValue::Nil),
                Ok(_) | Err(Signal::Return(_)) => Ok(built),
                Err(e) => Err(e),
            };
        }

        let plan: Vec<(String, Option<String>, bool, Option<Node<'static>>)> = self
            .types
            .struct_def(type_name)
            .map(|d| {
                d.stored
                    .iter()
                    .map(|p| (p.name.clone(), p.ty.clone(), p.lazy, p.default))
                    .collect()
            })
            .unwrap_or_default();

        let mut fields: Vec<(String, SwiftValue)> = type_values.to_vec();
        let built = self.with_type_values(type_values, |me| {
            let mut built: Vec<(String, SwiftValue)> = Vec::new();
            let mut positional = args.iter().filter(|(l, _)| l.is_none());
            for (pname, field_ty, lazy, default) in plan {
                let labeled = args
                    .iter()
                    .find(|(l, _)| l.as_deref() == Some(pname.as_str()))
                    .map(|(_, v)| v.clone());
                // The `@propertyWrapper` type backing this field, if any.
                let wrapper = me
                    .types
                    .struct_def(type_name)
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
                    me.eval(&def)?
                } else if let Some(wt) = &wrapper {
                    // A wrapped property with no provided value and no default
                    // (e.g. `@EnvironmentObject var x: T`) is synthesized via
                    // the wrapper's own no-argument `init()` — its value is
                    // injected later (by the environment) rather than here.
                    let synthesized = me.instantiate_struct(wt, &[])?;
                    built.push((pname, synthesized));
                    continue;
                } else {
                    return Err(EvalError::Type(format!(
                        "missing value for property `{pname}` of {type_name}"
                    ))
                    .into());
                };
                // Wrap `@propertyWrapper` fields in their wrapper instance.
                let value = match &wrapper {
                    Some(wt) => {
                        me.instantiate_struct(wt, &[(Some("wrappedValue".into()), value)])?
                    }
                    None => value,
                };
                built.push((pname, value));
            }
            Ok(built)
        });
        fields.extend(built?);
        Ok(SwiftValue::Struct(Rc::new(StructObj {
            type_name: type_name.to_string(),
            fields,
        })))
    }

    /// Run `body` with integer generic parameter values bound in a fresh
    /// scope (so stored-property defaults can reference them), popping the
    /// scope regardless of the outcome.
    fn with_type_values<T>(
        &mut self,
        type_values: &[(String, SwiftValue)],
        body: impl FnOnce(&mut Self) -> Result<T, Signal>,
    ) -> Result<T, Signal> {
        if type_values.is_empty() {
            return body(self);
        }
        self.env.push();
        for (n, v) in type_values {
            self.env.declare(n, v.clone(), false);
        }
        let result = body(self);
        self.env.pop();
        result
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
        // Structs whose `==`/`!=` are defined by the operator table rather than
        // field-wise comparison (`Measurement`: base-unit value; `Decimal`:
        // numeric value incl. NaN ≠ NaN).
        fn uses_operator_equality(value: &SwiftValue) -> bool {
            matches!(value, SwiftValue::Struct(o)
                if o.type_name == "Measurement" || o.type_name == "Decimal")
        }
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

        // Pattern-match operator `~=`: range-contains or equality.
        if op == "~=" {
            let pattern = self.eval(&lhs)?;
            let subject = self.eval(&rhs)?;
            let matched = match &pattern {
                SwiftValue::Range { lo, hi, inclusive } => match &subject {
                    SwiftValue::Int(v) => {
                        v.raw >= *lo
                            && (if *inclusive {
                                v.raw <= *hi
                            } else {
                                v.raw < *hi
                            })
                    }
                    _ => false,
                },
                _ => pattern == subject,
            };
            return Ok(SwiftValue::Bool(matched));
        }

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
        // `lhs == .caseName` / `lhs != .caseName`: give the implicit-member
        // shorthand on the rhs the lhs's enum type as a contextual hint, so it
        // resolves against that enum even when a same-named static value is
        // also registered elsewhere (e.g. `req.networkServiceType == .default`
        // must not resolve `.default` to `URLSessionConfiguration.default`).
        let enum_hint = match (&op[..], &l) {
            ("==" | "!=", SwiftValue::Enum(e)) => Some(e.type_name.clone()),
            _ => None,
        };
        if let Some(ty) = &enum_hint {
            self.type_hint.push(Some(ty.clone()));
        }
        let r = self.eval(&rhs);
        if enum_hint.is_some() {
            self.type_hint.pop();
        }
        let r = r?;
        // Equality against nil / reference / compound values goes through the
        // structural comparison rather than the scalar operator table.
        // `Measurement`/`Decimal` define `==` via the operator table (1 km ==
        // 1000 m; NaN ≠ NaN), not field-wise comparison.
        let operator_eq = uses_operator_equality(&l) || uses_operator_equality(&r);
        if (op == "==" || op == "!=")
            && !operator_eq
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
                    let pattern = cond.first_child().ok_or_else(|| {
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
                                    // Shorthand reads resolve like any other
                                    // identifier: accessor-backed variables run
                                    // their getter, and a `weak`/`unowned`
                                    // binding upgrades (so `if let self` fails
                                    // once the referent is gone).
                                    let raw = self
                                        .env
                                        .get(&name)
                                        .ok_or_else(|| EvalError::UnknownVariable(name.clone()))?;
                                    match raw {
                                        SwiftValue::AccessorVar(idx) => {
                                            self.read_accessor_var(idx)?
                                        }
                                        v => self.upgrade_reference(v, &name)?,
                                    }
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
        let body = node
            .first_child()
            .ok_or_else(|| EvalError::Unsupported("repeat without body".into()))?;
        let cond = node
            .last_child()
            .ok_or_else(|| EvalError::Unsupported("repeat without condition".into()))?;
        let label = node.loop_label();
        loop {
            if let LoopFlow::Break = self.run_loop_body(&body, &label)? {
                break;
            }
            if !self.eval_condition(&cond)? {
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
            // The reader half of `makeStream(of:)`: drain its producer's buffer.
            SwiftValue::AsyncStreamHandle(sid) => self.drain_async_stream_handle(*sid)?,
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

    /// Drive a custom `AsyncSequence`'s iterator to completion, collecting every
    /// element into a `Vec` (our cooperative executor runs the producer eagerly,
    /// ADR-0005). Backs the async-sequence algorithms (`reduce`/`map`/…), which
    /// materialise the sequence and then reuse the eager array machinery.
    fn collect_async_sequence(&mut self, seq: &SwiftValue) -> Result<Vec<SwiftValue>, Signal> {
        // The reader half of `makeStream(of:)` drains its producer buffer
        // directly rather than driving an iterator protocol.
        if let SwiftValue::AsyncStreamHandle(sid) = seq {
            return self.drain_async_stream_handle(*sid);
        }
        const ITER: &str = "$asynccollect";
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
        let mut items = Vec::new();
        let outcome = loop {
            let current = self.env.get(ITER).unwrap_or(SwiftValue::Nil);
            let place = Place {
                root: ITER.into(),
                path: Vec::new(),
            };
            match self.call_struct_method(current, &iter_ty, "next", Vec::new(), Some(place)) {
                Ok(SwiftValue::Nil) => break Ok(()),
                Ok(v) => items.push(v),
                Err(e) => break Err(e),
            }
        };
        self.env.pop();
        outcome.map(|()| items)
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
        materialize_builtin_sequence(seq).ok_or_else(|| {
            EvalError::Type(format!("cannot iterate over {}", seq.type_name())).into()
        })
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
        // `copy x`, and `borrow x` evaluate to the operand's value — as does a
        // pack expansion `repeat each x` (a pack is the collected array).
        if matches!(op.as_str(), "consume" | "copy" | "borrow" | "repeat each") {
            return Ok(v);
        }
        match ops::unary(&op, &v) {
            Ok(result) => Ok(result),
            Err(e) => {
                // A user-declared prefix operator is a function named after it.
                if let Some(SwiftValue::Function(id)) = self.env.get(&op) {
                    return self.call_function(
                        id,
                        vec![CallArg {
                            label: None,
                            value: v,
                            place: None,
                        }],
                    );
                }
                Err(trap(e))
            }
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
            let repr = TypeRepr::parse(ty);
            let ty = repr.text();
            // `[T]` — bind T to the element type of an array argument.
            if let Some(inner) = repr.array_element().map(TypeRepr::text) {
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
                let decl_ty = p.ty.clone();
                self.env.declare(&p.name, value, p.inout_);
                if let Some(ty) = decl_ty {
                    self.env.set_declared_type(&p.name, ty);
                }
                if p.inout_ {
                    if let Some(place) = arg.place.clone() {
                        inout_binds.push((p.name.clone(), place));
                    }
                }
                ai += 1;
            } else if let Some(def) = p.default {
                let v = self.eval(&def)?;
                self.env.declare(&p.name, v, false);
                if let Some(ty) = p.ty.clone() {
                    self.env.set_declared_type(&p.name, ty);
                }
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

    /// Attempt a numeric/string conversion `Type(value)`. Returns `Ok(None)` if
    /// `name` is not a known conversion type.
    /// Whether a call's `ArrayLiteral`/`DictLiteral` callee is a *type* literal
    /// (`[Int]`, `[String: Int]`, `[[Int]]`) rather than a value literal
    /// (`[1, 2]`). Every element must name a type: an identifier that is not
    /// bound to a value in the current scope, or a nested type literal.
    fn is_type_literal_callee(&self, callee: &Node<'static>) -> bool {
        let children: Vec<Node<'static>> = callee.children().collect();
        let arity = if callee.kind() == NodeKind::DictLiteral {
            2
        } else {
            1
        };
        if children.len() != arity {
            return false;
        }
        children.iter().all(|c| self.is_type_element(c))
    }

    /// Whether `node` denotes a type within a type-literal callee: a type-naming
    /// identifier (not shadowed by a value binding) or a nested type literal.
    fn is_type_element(&self, node: &Node<'static>) -> bool {
        match node.kind() {
            NodeKind::IdentExpr => node
                .text()
                .is_some_and(|name| self.env.get(&name).is_none()),
            NodeKind::ArrayLiteral | NodeKind::DictLiteral => self.is_type_literal_callee(node),
            _ => false,
        }
    }

    /// Build the core-internal value-only builtin constructor table, consulted
    /// once per call in `eval_call`. Each entry arg-dispatches internally and
    /// returns `Ok(None)` when the arguments do not match its initializer, so
    /// the call falls through the rest of the dispatch ladder.
    fn builtin_ctor_table() -> BuiltinCtors {
        let mut t: HashMap<&'static str, BuiltinCtor> = HashMap::new();
        // JSON/plist coder markers (value-only opaque structs).
        t.insert("JSONEncoder", Self::ctor_json_coder);
        t.insert("JSONDecoder", Self::ctor_json_coder);
        t.insert("PropertyListEncoder", Self::ctor_json_coder);
        // Generic collection constructors.
        t.insert("Array", Self::ctor_array);
        t.insert("Set", Self::ctor_set);
        t.insert("Dictionary", Self::ctor_dictionary);
        t.insert("ContiguousArray", Self::ctor_conversion);
        t.insert("ArraySlice", Self::ctor_conversion);
        t.insert("CollectionOfOne", Self::ctor_conversion);
        t.insert("EmptyCollection", Self::ctor_empty_collection);
        // `Optional(x)` — wraps a single value; identity in the flattened model.
        t.insert("Optional", Self::ctor_optional);
        // Scalar conversion initializers (one argument each).
        for name in [
            "Int", "Int8", "Int16", "Int32", "Int64", "UInt", "UInt8", "UInt16", "UInt32",
            "UInt64", "Double", "Float", "Bool",
        ] {
            t.insert(name, Self::ctor_conversion);
        }
        // `String` has extra stdlib multi-argument initializers
        // (`repeating:count:`, `_:radix:uppercase:`) on top of scalar
        // conversion, so it gets a dedicated constructor.
        t.insert("String", Self::ctor_string);
        // `Substring(_:)` — build a full-range view over a StringProtocol arg.
        t.insert("Substring", Self::ctor_substring);
        BuiltinCtors { table: t }
    }

    /// `JSONEncoder()` / `JSONDecoder()` / `PropertyListEncoder()` —
    /// class-backed Objects so that `let encoder = JSONEncoder();`
    /// `encoder.outputFormatting = .prettyPrinted` is legal (reference
    /// semantics; `set_object_field` mutates in place without write-back).
    fn ctor_json_coder(
        _interp: &mut Interpreter,
        name: &str,
        _args: &[CallArg],
    ) -> Result<Option<SwiftValue>, Signal> {
        // Seed `userInfo` with an empty `[CodingUserInfoKey: Any]` dictionary
        // so `coder.userInfo[key] = …` works before any explicit assignment
        // (Foundation's default is an empty dictionary, not an unset member).
        Ok(Some(SwiftValue::Object(StdRc::new(RefCell::new(
            ClassObj {
                class_name: name.to_string(),
                fields: vec![(
                    "userInfo".to_string(),
                    SwiftValue::Dict(Rc::new(Vec::new())),
                )],
            },
        )))))
    }

    /// `Array<T>()` / `Array(repeating:count:)` / `Array(sequence)`.
    fn ctor_array(
        interp: &mut Interpreter,
        name: &str,
        args: &[CallArg],
    ) -> Result<Option<SwiftValue>, Signal> {
        if args.is_empty() {
            return Ok(Some(SwiftValue::Array(Rc::new(Vec::new()))));
        }
        if let Some(v) = interp.array_repeating_count(args)? {
            return Ok(Some(v));
        }
        if args.len() == 1 {
            return interp.try_conversion(name, &args[0].value);
        }
        Ok(None)
    }

    /// `Set<T>()` / `Set(sequence)`.
    fn ctor_set(
        interp: &mut Interpreter,
        name: &str,
        args: &[CallArg],
    ) -> Result<Option<SwiftValue>, Signal> {
        if args.is_empty() {
            return Ok(Some(SwiftValue::Set(Rc::new(Vec::new()))));
        }
        if args.len() == 1 {
            return interp.try_conversion(name, &args[0].value);
        }
        Ok(None)
    }

    /// `Dictionary<K,V>()` / `Dictionary(uniqueKeysWithValues:)` /
    /// `Dictionary(grouping:by:)`.
    fn ctor_dictionary(
        interp: &mut Interpreter,
        _name: &str,
        args: &[CallArg],
    ) -> Result<Option<SwiftValue>, Signal> {
        if args.is_empty() {
            return Ok(Some(SwiftValue::Dict(Rc::new(Vec::new()))));
        }
        interp.build_dictionary(args)
    }

    /// `EmptyCollection<T>()` — zero-argument ctor returning the empty-collection struct.
    fn ctor_empty_collection(
        _interp: &mut Interpreter,
        _name: &str,
        _args: &[CallArg],
    ) -> Result<Option<SwiftValue>, Signal> {
        Ok(Some(SwiftValue::Struct(StdRc::new(StructObj {
            type_name: "EmptyCollection".into(),
            fields: vec![],
        }))))
    }

    /// `Optional(x)` — wraps one value in an `Optional`.
    ///
    /// In the flattened-value model a present optional *is* its wrapped value,
    /// so `Optional(x)` is the identity function on the argument.  The
    /// zero-argument form (`Optional<Int>()`) is not valid Swift and is rejected.
    fn ctor_optional(
        _interp: &mut Interpreter,
        _name: &str,
        args: &[CallArg],
    ) -> Result<Option<SwiftValue>, Signal> {
        match args {
            [arg] => Ok(Some(arg.value.clone())),
            _ => Ok(None),
        }
    }

    /// Single-argument scalar/sequence conversion initializers (integer widths,
    /// `Double`/`Float`/`String`/`Bool`, `ContiguousArray`, `CollectionOfOne`).
    /// `String` initializers beyond scalar conversion:
    /// `String(repeating:count:)` (repeat a String or Character `count` times)
    /// and `String(_:radix:uppercase:)` (integer-to-string in base 2...36).
    /// Any shape this does not recognise falls through to `ctor_conversion`
    /// (single-argument scalar/`describing:` forms) or, failing that, `Ok(None)`
    /// so a framework-registered `String(label:)` free fn (e.g.
    /// `data:encoding:`) still gets its turn.
    fn ctor_string(
        interp: &mut Interpreter,
        name: &str,
        args: &[CallArg],
    ) -> Result<Option<SwiftValue>, Signal> {
        // `String(repeating: <String|Character>, count: Int)`
        if args.len() == 2
            && args[0].label.as_deref() == Some("repeating")
            && args[1].label.as_deref() == Some("count")
        {
            let unit = match &args[0].value {
                SwiftValue::Str(s) => s.clone(),
                other => {
                    return Err(EvalError::Type(format!(
                        "String(repeating:count:) expects a String, got {}",
                        other.type_name()
                    ))
                    .into())
                }
            };
            let count = match &args[1].value {
                SwiftValue::Int(i) => i.raw,
                other => {
                    return Err(EvalError::Type(format!(
                        "String(repeating:count:) expects an Int count, got {}",
                        other.type_name()
                    ))
                    .into())
                }
            };
            if count < 0 {
                return Err(trap(
                    "String(repeating:count:) requires a non-negative count".into(),
                ));
            }
            return Ok(Some(SwiftValue::Str(unit.repeat(count as usize))));
        }
        // `String(_ value: some BinaryInteger, radix: Int, uppercase: Bool = false)`
        if (args.len() == 2 || args.len() == 3)
            && args[0].label.is_none()
            && args[1].label.as_deref() == Some("radix")
        {
            let value = match &args[0].value {
                SwiftValue::Int(i) => i.raw,
                other => {
                    return Err(EvalError::Type(format!(
                        "String(_:radix:) expects an integer, got {}",
                        other.type_name()
                    ))
                    .into())
                }
            };
            let radix = match &args[1].value {
                SwiftValue::Int(i) => i.raw,
                other => {
                    return Err(EvalError::Type(format!(
                        "String(_:radix:) expects an Int radix, got {}",
                        other.type_name()
                    ))
                    .into())
                }
            };
            if !(2..=36).contains(&radix) {
                return Err(trap("String(_:radix:) radix must be in 2...36".into()));
            }
            let uppercase = match args.get(2).map(|a| &a.value) {
                Some(SwiftValue::Bool(b)) => *b,
                Some(other) => {
                    return Err(EvalError::Type(format!(
                        "String(_:radix:uppercase:) expects a Bool, got {}",
                        other.type_name()
                    ))
                    .into())
                }
                None => false,
            };
            return Ok(Some(SwiftValue::Str(int_to_radix_string(
                value,
                radix as u32,
                uppercase,
            ))));
        }
        Self::ctor_conversion(interp, name, args)
    }

    /// `Substring(_:)` — a full-range view over a `String` or `Substring`.
    /// `Substring()` is the empty substring.
    fn ctor_substring(
        _interp: &mut Interpreter,
        _name: &str,
        args: &[CallArg],
    ) -> Result<Option<SwiftValue>, Signal> {
        match args {
            [] => Ok(Some(SwiftValue::Substring {
                base: Rc::new(String::new()),
                start: 0,
                end: 0,
            })),
            [arg] => match &arg.value {
                SwiftValue::Str(s) => {
                    let end = crate::graphemes(s).len();
                    Ok(Some(SwiftValue::Substring {
                        base: Rc::new(s.clone()),
                        start: 0,
                        end,
                    }))
                }
                sub @ SwiftValue::Substring { .. } => Ok(Some(sub.clone())),
                other => Err(EvalError::Type(format!(
                    "Substring(_:) expects a String or Substring, got {}",
                    other.type_name()
                ))
                .into()),
            },
            _ => Ok(None),
        }
    }

    fn ctor_conversion(
        interp: &mut Interpreter,
        name: &str,
        args: &[CallArg],
    ) -> Result<Option<SwiftValue>, Signal> {
        if args.len() == 1 {
            // `String(describing:)`/`String(reflecting:)` (and the unlabelled
            // form) stringify their argument via this scalar-conversion path.
            // Any *other* single-argument label on `String` (e.g.
            // `contentsOfFile:`/`contentsOf:`, registered by a framework as a
            // labelled free fn) is not a scalar conversion — fall through so
            // the free-fn dispatch further down `eval_call`'s ladder gets a
            // chance instead of silently stringifying the raw argument.
            if name == "String"
                && !matches!(
                    args[0].label.as_deref(),
                    None | Some("describing") | Some("reflecting")
                )
            {
                return Ok(None);
            }
            return interp.try_conversion(name, &args[0].value);
        }
        Ok(None)
    }

    /// `[T](...)` array constructor: empty for no arguments, otherwise it must be
    /// the `repeating:count:` form (any other shape is an error).
    fn construct_array_literal_ctor(&self, args: &[CallArg]) -> Eval {
        if args.is_empty() {
            return Ok(SwiftValue::Array(Rc::new(Vec::new())));
        }
        match self.array_repeating_count(args)? {
            Some(v) => Ok(v),
            None => Err(EvalError::Type(
                "[T](...) takes either no arguments or `repeating:count:`".into(),
            )
            .into()),
        }
    }

    /// `repeating:count:` array construction, shared by `Array(repeating:count:)`
    /// and `[T](repeating:count:)`. `Ok(None)` when no `repeating:` label is
    /// present (not this initializer); an error when `count:` is missing or
    /// negative.
    fn array_repeating_count(&self, args: &[CallArg]) -> Result<Option<SwiftValue>, Signal> {
        let Some(repeating) = args
            .iter()
            .find(|a| a.label.as_deref() == Some("repeating"))
            .map(|a| a.value.clone())
        else {
            return Ok(None);
        };
        match args.iter().find(|a| a.label.as_deref() == Some("count")) {
            Some(CallArg {
                value: SwiftValue::Int(i),
                ..
            }) if i.raw >= 0 => Ok(Some(SwiftValue::Array(Rc::new(vec![
                repeating;
                i.raw as usize
            ])))),
            Some(CallArg {
                value: SwiftValue::Int(_),
                ..
            }) => Err(trap(
                "Array(repeating:count:) requires a non-negative count".into(),
            )),
            _ => Err(EvalError::Type("Array(repeating:count:) requires a count".into()).into()),
        }
    }

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

        if let Some(seq) = labeled("grouping") {
            let elements = materialize_sequence(&seq.value)
                .ok_or_else(|| EvalError::Type("grouping: expects a sequence".into()))?;
            // The discriminator closure arrives either labelled `by:` or as a
            // single trailing (unlabelled) closure. Reject any other argument so
            // a stray value or a second closure cannot be silently dropped or
            // mistaken for the discriminator.
            let mut by: Option<SwiftValue> = None;
            for a in args {
                match a.label.as_deref() {
                    Some("grouping") => {}
                    Some("by") | None if matches!(a.value, SwiftValue::Closure(_)) => {
                        if by.replace(a.value.clone()).is_some() {
                            return Err(EvalError::Type(
                                "Dictionary(grouping:by:) takes a single discriminator closure"
                                    .into(),
                            )
                            .into());
                        }
                    }
                    other => {
                        return Err(EvalError::Type(format!(
                            "Dictionary(grouping:by:) called with unexpected argument {}",
                            other.unwrap_or("_")
                        ))
                        .into())
                    }
                }
            }
            let SwiftValue::Closure(id) =
                by.ok_or_else(|| EvalError::Type("grouping expects a `by:` closure".into()))?
            else {
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
            // `ArraySlice(seq)` is an array in this model (promotes to a full slice).
            "ArraySlice" => Ok(materialize_sequence(value).map(|v| {
                let count = v.len();
                SwiftValue::ArraySlice {
                    base: StdRc::new(v),
                    start: 0,
                    end: count,
                }
            })),
            // `CollectionOfOne(x)` wraps a single element.
            "CollectionOfOne" => Ok(Some(SwiftValue::Struct(StdRc::new(StructObj {
                type_name: "CollectionOfOne".into(),
                fields: vec![("_element".into(), value.clone())],
            })))),
            _ => Ok(None),
        }
    }
}

/// Eagerly materialize a builtin sequence value into a `Vec` of its elements,
/// or `None` if the value is not a sequence the tree-walker can expand.
fn materialize_sequence(value: &SwiftValue) -> Option<Vec<SwiftValue>> {
    materialize_builtin_sequence(value)
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
            .types
            .struct_def(&obj.type_name)
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

    fn now_unix_seconds(&mut self) -> f64 {
        now_unix_seconds()
    }

    fn http_start(
        &mut self,
        req: &crate::http::HttpRequest,
    ) -> Result<crate::http::HttpTaskHandle, crate::http::HttpError> {
        match &mut self.http_transport {
            Some(transport) => transport.start(req),
            None => Err(crate::http::HttpError::Unavailable),
        }
    }

    fn http_next_event(&mut self, h: crate::http::HttpTaskHandle) -> crate::http::HttpEvent {
        match &mut self.http_transport {
            Some(transport) => transport.next_event(h),
            None => crate::http::HttpEvent::Failed {
                code: "unsupported".into(),
                message: "HTTP transport unavailable".into(),
            },
        }
    }

    fn http_cancel(&mut self, h: crate::http::HttpTaskHandle) {
        if let Some(transport) = &mut self.http_transport {
            transport.cancel(h);
        }
    }

    fn perform_http(
        &mut self,
        req: &crate::http::HttpRequest,
    ) -> Result<crate::http::HttpResponse, crate::http::HttpError> {
        // Optimised path: call the transport's one-shot `perform` directly,
        // avoiding the start/next_event loop overhead for callers that don't
        // need event-level access. Foundation (M3+) uses http_start /
        // http_next_event / http_cancel instead.
        match &mut self.http_transport {
            Some(transport) => transport.perform(req),
            None => Err(crate::http::HttpError::Unavailable),
        }
    }

    fn current_task_cancelled(&self) -> bool {
        // Delegate to the Interpreter's own inherent method (pub(super),
        // defined in concurrency.rs) which reads the scheduler's flag.
        // Calling self.current_task_cancelled() here resolves to the inherent
        // method (inherent methods take priority over trait methods in Rust
        // method resolution), so there is no infinite recursion.
        Interpreter::current_task_cancelled(self)
    }

    fn call_host_fn(
        &mut self,
        name: &str,
        args: Vec<(Option<String>, SwiftValue)>,
    ) -> crate::stdlib::StdResult {
        // Explicit qualification picks the inherent `call_host_fn` (private,
        // defined above) rather than recursing into this trait method.
        match Interpreter::call_host_fn(self, name, &args) {
            Ok(v) => Ok(v),
            Err(sig) => Err(Self::signal_to_std_error(sig)),
        }
    }

    fn is_host_fn(&self, name: &str) -> bool {
        Interpreter::is_host_fn(self, name)
    }

    fn key_path_components(&self, value: &SwiftValue) -> Option<Vec<String>> {
        self.keypath_components(value)
    }

    fn eval_node(&mut self, node: &Node<'static>) -> crate::stdlib::StdResult {
        self.eval(node).map_err(Self::signal_to_std_error)
    }

    fn nominal_type_info(&self, type_name: &str) -> Option<crate::stdlib::NominalTypeInfo> {
        use crate::stdlib::{NominalProperty, NominalTypeInfo};
        if let Some(def) = self.types.class_def(type_name) {
            return Some(NominalTypeInfo {
                attributes: def.attributes.clone(),
                stored: def
                    .stored
                    .iter()
                    .map(|p| NominalProperty {
                        name: p.name.clone(),
                        declared_type: p.ty.clone(),
                    })
                    .collect(),
            });
        }
        if let Some(def) = self.types.struct_def(type_name) {
            return Some(NominalTypeInfo {
                attributes: def.attributes.clone(),
                stored: def
                    .stored
                    .iter()
                    .map(|p| NominalProperty {
                        name: p.name.clone(),
                        declared_type: p.ty.clone(),
                    })
                    .collect(),
            });
        }
        None
    }

    fn singleton(&mut self, key: &str, init: fn() -> SwiftValue) -> SwiftValue {
        if let Some(v) = self.singletons.get(key) {
            return v.clone();
        }
        let v = init();
        self.singletons.insert(key.to_string(), v.clone());
        v
    }

    fn register_finalizer(&mut self, finalizer: crate::stdlib::Finalizer) {
        // Always-active teardown hook (not import-gated); tagged with the
        // current module for registry uniformity / future per-module hooks.
        self.finalizers
            .push(ModuleTagged::new(finalizer, self.current_module));
    }

    fn view_scope_enter(&mut self, view: &SwiftValue) {
        // Snapshot the fn pointers first (they are `Copy`) so we don't hold a
        // borrow of `self` across the callback, which takes `self` as context.
        let enters: Vec<crate::stdlib::ViewScopeFn> =
            self.view_scopes.iter().map(|t| t.value.0).collect();
        for enter in enters {
            enter(self, view);
        }
    }

    fn view_scope_exit(&mut self, view: &SwiftValue) {
        // Reverse registration order so exits unwind the matched enters.
        let exits: Vec<crate::stdlib::ViewScopeFn> =
            self.view_scopes.iter().rev().map(|t| t.value.1).collect();
        for exit in exits {
            exit(self, view);
        }
    }

    fn interpreter_id(&self) -> u64 {
        self.id
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
                let indices = self.types.enum_def(&ea.type_name).and_then(|def| {
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

    fn call_method_on(
        &mut self,
        receiver: SwiftValue,
        method: &str,
        args: Vec<crate::stdlib::Arg>,
    ) -> crate::stdlib::StdResult {
        let class_name = match &receiver {
            SwiftValue::Object(o) => o.borrow().class_name.clone(),
            _ => return Ok(SwiftValue::Void),
        };
        let call_args: Vec<CallArg> = args
            .into_iter()
            .map(|a| CallArg {
                label: a.label,
                value: a.value,
                place: None,
            })
            .collect();
        match self.dispatch_class_method(receiver, &class_name, method, call_args) {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(sig) => Err(Self::signal_to_std_error(sig)),
        }
    }

    fn has_method_on(
        &self,
        receiver: &SwiftValue,
        method: &str,
        call_args: &[crate::stdlib::Arg],
    ) -> bool {
        let class_name = match receiver {
            SwiftValue::Object(o) => o.borrow().class_name.clone(),
            _ => return false,
        };
        // Walk the inheritance chain looking for any overload whose parameter
        // labels match the argument labels we intend to pass.
        //
        // ⚠ Known limitation (ADR-0011 §Known limitations): protocol-extension
        // *default* implementations are not visible here.  A class that
        // conforms to `URLSessionDataDelegate` but does NOT explicitly override
        // a protocol method (relying instead on the default no-op provided by
        // the Swift stdlib via a protocol extension) will have that method
        // absent from its `methods`/`method_overloads` maps, so `has_method_on`
        // returns `false` and the callback is skipped.  In the tswift runtime
        // this is correct behaviour — optional delegate methods that the
        // script does not explicitly implement should be silently skipped —
        // but it diverges from a world where protocol-extension defaults
        // could have observable side-effects.  Fixing this would require the
        // interpreter to index protocol-extension bodies (non-trivial;
        // deferred).
        let mut current = Some(class_name);
        while let Some(cls) = current {
            let Some(def) = self.types.class_def(&cls) else {
                break;
            };
            if let Some(overloads) = def.method_overloads.get(method) {
                // Use label-match: each call_arg's label should match the
                // effective param label (explicit label or param name).
                for ov in overloads {
                    if overload_labels_match(&ov.params, call_args) {
                        return true;
                    }
                }
            } else if def.methods.contains_key(method) {
                return true;
            }
            current = def.superclass.clone();
        }
        false
    }

    fn allocate_response_disposition_closure(&mut self) -> usize {
        // Advance the token so any previously-allocated closure whose script
        // reference is stored and called late will be silently ignored.
        self.response_disposition_token = self.response_disposition_token.wrapping_add(1);
        let token = self.response_disposition_token;
        // Clear any stale disposition from a previous request.  A well-behaved
        // delegate calls the completionHandler synchronously; the reset here
        // is a belt-and-suspenders guard so a late call from an old handler
        // can never bleed into this request.
        self.response_disposition = None;
        let id = self.closures.len();
        self.closures
            .push((ClosureDef::ResponseDispositionCapture { token }, Vec::new()));
        id
    }

    fn take_response_disposition(&mut self) -> bool {
        self.response_disposition.take().unwrap_or(true)
    }
}

/// Check whether the argument labels we plan to pass match an overload's
/// parameter labels, used by `has_method_on` and class-method overload
/// selection.  A `None` arg label matches any parameter (positional call).
///
/// ## Intentional divergence from `args_select_params`
///
/// `args_select_params` (used in `select_labeled_overload` for call dispatch)
/// handles defaults and variadics: it can match a call with fewer arguments
/// than parameters if the remaining params have defaults, and it advances the
/// argument cursor through a variadic span.  `overload_labels_match` is
/// intentionally **strict** (`len ==`).  It is only used by `has_method_on`
/// to decide whether a delegate class *implements* a particular overload,
/// where Foundation always passes the *exact* arg count for that overload
/// (the probe args are synthesised with the correct label count in
/// `delegate_probe_args`).  Divergence is safe because:
///   1. Foundation-internal delegate calls never use defaults or variadics;
///   2. The probe is produced by `delegate_probe_args` with precisely the
///      labels that the dispatch will actually supply.
///
/// If Foundation ever dispatches a variadic delegate method, the probe must
/// be updated to supply a representative arg count and this comment updated.
fn overload_labels_match(params: &[Param], args: &[crate::stdlib::Arg]) -> bool {
    if params.len() != args.len() {
        return false;
    }
    for (param, arg) in params.iter().zip(args.iter()) {
        let effective = param.label.as_deref().unwrap_or(param.name.as_str());
        if let Some(label) = &arg.label {
            if label.as_str() != effective && effective != "_" {
                return false;
            }
        }
    }
    true
}

/// Extract `T` from a metatype argument node `T.self`.
fn metatype_name(node: &Node<'static>) -> Option<String> {
    if node.kind() == NodeKind::MemberExpr && node.text().as_deref() == Some("self") {
        node.first_child().and_then(|b| b.text())
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
    let repr = TypeRepr::parse(spelling);
    // Peel one optional, one array layer, then a second optional — so
    // `[User]`/`Role?`/`User`/`[User]?` all yield the nominal element type.
    let repr = repr.unwrap_optional();
    let repr = repr.array_element().unwrap_or(repr);
    repr.unwrap_optional().text()
}

fn array_element_type(name: &str) -> Option<&str> {
    // Only homogeneous element arrays `[T]`; a dictionary `[K: V]` is not one.
    TypeRepr::parse(name).array_element().map(TypeRepr::text)
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
    let Some(gp) = node.find_child(NodeKind::GenericParam) else {
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
            node.first_child().and_then(|child| match child.kind() {
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
    match value {
        SwiftValue::Range { .. }
        | SwiftValue::Array(_)
        | SwiftValue::ArraySlice { .. }
        | SwiftValue::Str(_)
        | SwiftValue::Substring { .. }
        | SwiftValue::Dict(_)
        | SwiftValue::Set(_) => true,
        // `Data` iterates as its byte elements.
        SwiftValue::Struct(obj) if obj.type_name == "Data" => true,
        // Small collection types.
        SwiftValue::Struct(obj)
            if matches!(
                obj.type_name.as_str(),
                "ReversedCollection" | "CollectionOfOne" | "EmptyCollection"
            ) =>
        {
            true
        }
        _ => false,
    }
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

/// Scan a string literal's *interpolation* segments (`\( … )`) for shorthand
/// argument references (`$0`, `$1`, …) and return the greatest index found.
/// Only text inside `\(…)` is considered, so a literal `"$1"` outside an
/// interpolation is ignored.
/// Extract and range-check the `_offset` field from a `Set.Index` or
/// `Dictionary.Index` struct.  Does NOT check the anchor — callers that need
/// stale-index detection must do that separately.
fn read_opaque_index(type_name: &str, idx: &SwiftValue, len: usize) -> Result<usize, Signal> {
    let obj = match idx {
        SwiftValue::Struct(o) if o.type_name == type_name => o,
        _ => return Err(EvalError::Type(format!("expected a {type_name}")).into()),
    };
    let offset = match obj.get("_offset") {
        Some(SwiftValue::Int(i)) if i.raw >= 0 => i.raw as usize,
        _ => return Err(trap(format!("invalid {type_name}: bad _offset"))),
    };
    if offset >= len {
        return Err(trap(format!("{type_name} out of range (endIndex)")));
    }
    Ok(offset)
}

fn subscript_index(indices: &[SwiftValue]) -> Result<usize, Signal> {
    match indices.first() {
        Some(SwiftValue::Int(i)) if i.raw >= 0 => Ok(i.raw as usize),
        Some(SwiftValue::Int(i)) => Err(trap(format!("negative index {}", i.raw))),
        _ => Err(EvalError::Type("subscript index must be an integer".into()).into()),
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stdlib::Arg;

    #[test]
    fn int_to_radix_string_matches_swift() {
        assert_eq!(int_to_radix_string(255, 16, false), "ff");
        assert_eq!(int_to_radix_string(255, 16, true), "FF");
        assert_eq!(int_to_radix_string(10, 2, false), "1010");
        assert_eq!(int_to_radix_string(-42, 2, false), "-101010");
        assert_eq!(int_to_radix_string(0, 16, false), "0");
        assert_eq!(int_to_radix_string(35, 36, false), "z");
        assert_eq!(
            int_to_radix_string(i128::MIN, 10, false),
            i128::MIN.to_string()
        );
    }

    #[test]
    fn type_table_queries_reflect_registered_declarations() {
        let mut types = TypeTable::default();
        assert!(!types.is_nominal("Point"));
        types.insert_struct("Point".into(), StructDef::default());
        types.insert_enum("Dir".into(), EnumDef::default());
        assert!(types.is_struct("Point"));
        assert!(types.is_enum("Dir"));
        assert!(!types.is_class("Point"));
        assert!(types.is_nominal("Point") && types.is_nominal("Dir"));
        assert!(types.struct_def("Point").is_some());
        assert!(types.enum_def("Point").is_none());
        let names: Vec<&String> = types.struct_names().collect();
        assert_eq!(names, vec![&"Point".to_string()]);
    }

    #[test]
    fn module_scope_stamps_registrations_and_type_module() {
        fn free(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> crate::stdlib::StdResult {
            Ok(SwiftValue::Void)
        }
        fn method(
            _ctx: &mut dyn StdContext,
            recv: SwiftValue,
            _args: Vec<Arg>,
        ) -> crate::stdlib::StdResult {
            Ok(recv)
        }
        fn macro_handler(
            _ctx: &mut dyn StdContext,
            _node: &tswift_frontend::Node<'static>,
        ) -> crate::stdlib::StdResult {
            Ok(SwiftValue::Void)
        }

        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        // Untagged (default base module).
        interp.register_free_fn("BaseType", free);
        assert_eq!(interp.type_module("BaseType"), Some("Swift"));
        assert_eq!(interp.current_module().as_str(), "Swift");

        interp.module("SwiftUI", |i| {
            i.register_free_fn("Text", free);
            i.register_struct_method("padding", method);
        });
        interp.module("Charts", |i| {
            i.register_free_fn("BarMark", free);
            // Per-module candidate coexists with SwiftUI's (Phase B).
            i.register_struct_method("padding", method);
        });
        interp.module("SwiftData", |i| {
            i.register_macro("Predicate", macro_handler);
            let handler = std::sync::Arc::new(MockHostHandler::new(|_n, _a| Ok("null".into())));
            i.set_host_call_handler(handler);
            i.register_host_fn(r#"{"name":"deviceName","returns":"Void"}"#, None)
                .expect("register host fn");
        });

        assert_eq!(interp.type_module("Text"), Some("SwiftUI"));
        assert_eq!(interp.type_module("BarMark"), Some("Charts"));
        // Both modules keep a `padding` candidate; receiver routes selection.
        let padding_mods = interp.struct_method_modules("padding");
        assert!(padding_mods.contains(&"SwiftUI"));
        assert!(padding_mods.contains(&"Charts"));
        // Import both so strict gating does not hide candidates under query.
        interp.mark_module_imported("SwiftUI");
        interp.mark_module_imported("Charts");
        assert_eq!(
            interp.struct_method_module_for("padding", "Text"),
            Some("SwiftUI")
        );
        assert_eq!(
            interp.struct_method_module_for("padding", "BarMark"),
            Some("Charts")
        );
        assert_eq!(interp.macro_module("Predicate"), Some("SwiftData"));
        assert_eq!(interp.host_fn_module("deviceName"), Some("SwiftData"));
        // Scope restored after nested modules.
        assert_eq!(interp.current_module().as_str(), "Swift");
    }

    #[test]
    fn module_scope_restores_current_module_on_panic() {
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        assert_eq!(interp.current_module().as_str(), "Swift");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            interp.module("Charts", |_i| {
                panic!("install blew up");
            });
        }));
        assert!(result.is_err());
        // RAII guard must restore even when the install panics (caught unwind).
        assert_eq!(interp.current_module().as_str(), "Swift");
    }

    #[test]
    fn struct_method_candidates_resolve_by_receiver_module() {
        fn swiftui_fg(
            _ctx: &mut dyn StdContext,
            recv: SwiftValue,
            _args: Vec<Arg>,
        ) -> crate::stdlib::StdResult {
            Ok(SwiftValue::Str(format!(
                "swiftui:{}",
                match &recv {
                    SwiftValue::Struct(o) => o.type_name.as_str(),
                    _ => "?",
                }
            )))
        }
        fn charts_fg(
            _ctx: &mut dyn StdContext,
            recv: SwiftValue,
            _args: Vec<Arg>,
        ) -> crate::stdlib::StdResult {
            Ok(SwiftValue::Str(format!(
                "charts:{}",
                match &recv {
                    SwiftValue::Struct(o) => o.type_name.as_str(),
                    _ => "?",
                }
            )))
        }
        fn free(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> crate::stdlib::StdResult {
            Ok(SwiftValue::Void)
        }

        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        // Register Charts first, then SwiftUI — old last-wins would leave SwiftUI only.
        interp.module("Charts", |i| {
            i.register_free_fn("BarMark", free);
            i.register_struct_method("foregroundStyle", charts_fg);
        });
        interp.module("SwiftUI", |i| {
            i.register_free_fn("Text", free);
            i.register_struct_method("foregroundStyle", swiftui_fg);
        });
        interp.mark_module_imported("Charts");
        interp.mark_module_imported("SwiftUI");

        assert_eq!(
            interp.struct_method_module_for("foregroundStyle", "Text"),
            Some("SwiftUI")
        );
        assert_eq!(
            interp.struct_method_module_for("foregroundStyle", "BarMark"),
            Some("Charts")
        );
        // Unknown receiver → base Swift miss → alphabetical (Charts < SwiftUI).
        assert_eq!(
            interp.struct_method_module_for("foregroundStyle", "UserWidget"),
            Some("Charts")
        );

        // Same-module re-install is idempotent (still one Charts candidate).
        interp.module("Charts", |i| {
            i.register_struct_method("foregroundStyle", charts_fg);
        });
        assert_eq!(
            interp.struct_method_modules("foregroundStyle").len(),
            2,
            "re-install must replace, not duplicate"
        );
    }

    /// Fallback branch (no exact match, no depends-on hit) is install-order
    /// independent: alphabetical by module id, never `candidates.first()`.
    #[test]
    fn struct_method_fallback_is_install_order_independent() {
        fn alpha_handler(
            _ctx: &mut dyn StdContext,
            _recv: SwiftValue,
            _args: Vec<Arg>,
        ) -> crate::stdlib::StdResult {
            Ok(SwiftValue::Str("alpha".into()))
        }
        fn zebra_handler(
            _ctx: &mut dyn StdContext,
            _recv: SwiftValue,
            _args: Vec<Arg>,
        ) -> crate::stdlib::StdResult {
            Ok(SwiftValue::Str("zebra".into()))
        }
        fn free(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> crate::stdlib::StdResult {
            Ok(SwiftValue::Void)
        }

        // Install Zebra first, then Alpha — insertion order would pick Zebra.
        let mut sink = std::io::sink();
        let mut zebra_first = Interpreter::new(&mut sink);
        zebra_first.module("Zebra", |i| {
            i.register_free_fn("ZebraType", free);
            i.register_struct_method("twist", zebra_handler);
        });
        zebra_first.module("Alpha", |i| {
            i.register_free_fn("AlphaType", free);
            i.register_struct_method("twist", alpha_handler);
        });
        // Foundation receiver: depends-on is only Swift (no Alpha/Zebra) → fallback.
        zebra_first.module("Foundation", |i| {
            i.register_free_fn("Date", free);
        });
        zebra_first.mark_module_imported("Alpha");
        zebra_first.mark_module_imported("Zebra");
        zebra_first.mark_module_imported("Foundation");

        // Opposite install order.
        let mut sink2 = std::io::sink();
        let mut alpha_first = Interpreter::new(&mut sink2);
        alpha_first.module("Alpha", |i| {
            i.register_free_fn("AlphaType", free);
            i.register_struct_method("twist", alpha_handler);
        });
        alpha_first.module("Zebra", |i| {
            i.register_free_fn("ZebraType", free);
            i.register_struct_method("twist", zebra_handler);
        });
        alpha_first.module("Foundation", |i| {
            i.register_free_fn("Date", free);
        });
        alpha_first.mark_module_imported("Alpha");
        alpha_first.mark_module_imported("Zebra");
        alpha_first.mark_module_imported("Foundation");

        // Exact match still wins when the receiver owns a candidate.
        assert_eq!(
            zebra_first.struct_method_module_for("twist", "ZebraType"),
            Some("Zebra")
        );
        assert_eq!(
            alpha_first.struct_method_module_for("twist", "AlphaType"),
            Some("Alpha")
        );

        // Fallback: Foundation has no `twist` and depends only on Swift →
        // alphabetical Alpha < Zebra, regardless of install order.
        assert_eq!(
            zebra_first.struct_method_module_for("twist", "Date"),
            Some("Alpha"),
            "zebra-first install must still fall back alphabetically to Alpha"
        );
        assert_eq!(
            alpha_first.struct_method_module_for("twist", "Date"),
            Some("Alpha"),
            "alpha-first install must still fall back alphabetically to Alpha"
        );
        // Unknown receiver uses the same fallback branch.
        assert_eq!(
            zebra_first.struct_method_module_for("twist", "UserWidget"),
            Some("Alpha")
        );
        assert_eq!(
            alpha_first.struct_method_module_for("twist", "UserWidget"),
            Some("Alpha")
        );
    }

    /// Phase C: `import` decls seed `imported_modules`; base Swift is always on.
    #[test]
    fn import_decl_collects_modules_lenient() {
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        assert!(interp.is_module_imported("Swift"));
        assert!(!interp.is_module_imported("SwiftUI"));
        assert!(!interp.is_module_imported("Charts"));
        // Set always contains base Swift at construction.
        assert!(interp
            .imported_modules()
            .iter()
            .any(|m| *m == ModuleId::SWIFT));

        let src = "import SwiftUI\nlet x = 1\n";
        let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        interp.run(analysis).expect("run");

        assert!(interp.is_module_imported("SwiftUI"));
        assert!(interp.is_module_imported("Swift"));
        assert!(!interp.is_module_imported("Charts"));
        // Submodule path records only the leading component.
        let mut sink2 = std::io::sink();
        let mut interp2 = Interpreter::new(&mut sink2);
        let src2 = "import Foundation.Data\n";
        let a2 = Analysis::analyze(src2, "test.swift").expect("analyze");
        let a2: &'static Analysis = Box::leak(Box::new(a2));
        interp2.run(a2).expect("run");
        assert!(interp2.is_module_imported("Foundation"));
        assert!(!interp2.is_module_imported("Foundation.Data"));
    }

    /// Host-prepended PRELUDE has no imports; only user `import`s are recorded.
    #[test]
    fn import_collection_ignores_prelude_picks_user_imports() {
        // Stand-in for host PRELUDE (no import lines).
        let prelude = "struct Visibility { static let hidden = 0 }\n";
        let with_import = format!("{prelude}import Charts\nlet y = 1\n");
        let without_import = format!("{prelude}let y = 1\n");

        let mut sink = std::io::sink();
        let mut with = Interpreter::new(&mut sink);
        let a = Analysis::analyze(&with_import, "test.swift").expect("analyze");
        let a: &'static Analysis = Box::leak(Box::new(a));
        with.run(a).expect("run");
        assert!(with.is_module_imported("Charts"));
        assert!(with.is_module_imported("Swift"));
        assert!(!with.is_module_imported("SwiftUI"));

        let mut sink2 = std::io::sink();
        let mut without = Interpreter::new(&mut sink2);
        let a2 = Analysis::analyze(&without_import, "test.swift").expect("analyze");
        let a2: &'static Analysis = Box::leak(Box::new(a2));
        without.run(a2).expect("run");
        assert!(!without.is_module_imported("Charts"));
        assert!(without.is_module_imported("Swift"));
    }

    /// Host/test pre-seed API for framework injection without a source import.
    #[test]
    fn mark_module_imported_preseeds_set() {
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        assert!(!interp.is_module_imported("SwiftUI"));
        interp.mark_module_imported("SwiftUI");
        assert!(interp.is_module_imported("SwiftUI"));
        // Idempotent; dotted path also works.
        interp.mark_module_imported("SwiftUI.View");
        assert!(interp.is_module_imported("SwiftUI"));
        assert_eq!(
            interp
                .imported_modules()
                .iter()
                .filter(|m| m.as_str() == "SwiftUI")
                .count(),
            1
        );
    }

    /// Under strict import-gating (default), import-less programs cannot resolve
    /// framework struct-method candidates. Opt into lenient for the Phase C
    /// alphabetical fallback.
    #[test]
    fn import_less_program_strict_gates_framework_modifiers() {
        fn swiftui_fg(
            _ctx: &mut dyn StdContext,
            recv: SwiftValue,
            _args: Vec<Arg>,
        ) -> crate::stdlib::StdResult {
            Ok(recv)
        }
        fn charts_fg(
            _ctx: &mut dyn StdContext,
            recv: SwiftValue,
            _args: Vec<Arg>,
        ) -> crate::stdlib::StdResult {
            Ok(recv)
        }
        fn free(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> crate::stdlib::StdResult {
            Ok(SwiftValue::Void)
        }

        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        assert!(interp.strict_imports());
        interp.module("Charts", |i| {
            i.register_free_fn("BarMark", free);
            i.register_struct_method("foregroundStyle", charts_fg);
        });
        interp.module("SwiftUI", |i| {
            i.register_free_fn("Text", free);
            i.register_struct_method("foregroundStyle", swiftui_fg);
        });

        let src = "let n = 1\n";
        let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        interp.run(analysis).expect("run");
        assert!(!interp.is_module_imported("SwiftUI"));
        assert!(!interp.is_module_imported("Charts"));

        // Strict: no framework candidates without import.
        assert_eq!(
            interp.struct_method_module_for("foregroundStyle", "Text"),
            None
        );
        assert_eq!(
            interp.struct_method_module_for("foregroundStyle", "BarMark"),
            None
        );
        assert_eq!(
            interp.struct_method_module_for("foregroundStyle", "UserWidget"),
            None
        );

        // Lenient opt-out restores Phase C behaviour.
        interp.set_strict_imports(false);
        assert_eq!(
            interp.struct_method_module_for("foregroundStyle", "Text"),
            Some("SwiftUI")
        );
        assert_eq!(
            interp.struct_method_module_for("foregroundStyle", "BarMark"),
            Some("Charts")
        );
        assert_eq!(
            interp.struct_method_module_for("foregroundStyle", "UserWidget"),
            Some("Charts")
        );
    }

    /// Same-tier fallback prefers an imported module before alphabetical
    /// (lenient path; under strict only imported candidates remain).
    #[test]
    fn same_tier_fallback_prefers_imported_module() {
        fn handler(
            _ctx: &mut dyn StdContext,
            recv: SwiftValue,
            _args: Vec<Arg>,
        ) -> crate::stdlib::StdResult {
            Ok(recv)
        }

        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        interp.module("Charts", |i| {
            i.register_struct_method("foregroundStyle", handler);
        });
        interp.module("SwiftUI", |i| {
            i.register_struct_method("foregroundStyle", handler);
        });
        // Strict without import: gated out.
        assert_eq!(
            interp.struct_method_module_for("foregroundStyle", "UserWidget"),
            None
        );
        // Lenient without import: alphabetical → Charts.
        interp.set_strict_imports(false);
        assert_eq!(
            interp.struct_method_module_for("foregroundStyle", "UserWidget"),
            Some("Charts")
        );
        // With SwiftUI imported: prefer imported over alphabetical.
        interp.mark_module_imported("SwiftUI");
        assert_eq!(
            interp.struct_method_module_for("foregroundStyle", "UserWidget"),
            Some("SwiftUI")
        );
        // Strict + only SwiftUI imported → SwiftUI only.
        interp.set_strict_imports(true);
        assert_eq!(
            interp.struct_method_module_for("foregroundStyle", "UserWidget"),
            Some("SwiftUI")
        );
    }

    /// Phase D2: framework free-fn constructors require their module import.
    #[test]
    fn strict_import_gates_framework_free_fns_with_clear_diagnostic() {
        fn free(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> crate::stdlib::StdResult {
            Ok(SwiftValue::Void)
        }

        // Text without import SwiftUI.
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("SwiftUI", |i| i.register_free_fn("Text", free));
            let src = "let t = Text(\"hi\")\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            let err = interp.run(analysis).expect_err("must fail without import");
            let msg = err.to_string();
            assert!(
                msg.contains("cannot find 'Text' in scope") && msg.contains("import SwiftUI"),
                "unexpected diagnostic: {msg}"
            );
        }
        // Text with import SwiftUI.
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("SwiftUI", |i| i.register_free_fn("Text", free));
            let src = "import SwiftUI\nlet t = Text(\"hi\")\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            interp.run(analysis).expect("import makes Text resolvable");
        }
        // BarMark without / with Charts.
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Charts", |i| i.register_free_fn("BarMark", free));
            let src = "let m = BarMark()\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            let err = interp.run(analysis).expect_err("must fail without import");
            let msg = err.to_string();
            assert!(
                msg.contains("cannot find 'BarMark' in scope") && msg.contains("import Charts"),
                "unexpected diagnostic: {msg}"
            );
        }
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Charts", |i| i.register_free_fn("BarMark", free));
            let src = "import Charts\nlet m = BarMark()\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            interp
                .run(analysis)
                .expect("import makes BarMark resolvable");
        }
        // URL without / with Foundation.
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| i.register_free_fn("URL", free));
            let src = "let u = URL()\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            let err = interp.run(analysis).expect_err("must fail without import");
            let msg = err.to_string();
            assert!(
                msg.contains("cannot find 'URL' in scope") && msg.contains("import Foundation"),
                "unexpected diagnostic: {msg}"
            );
        }
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| i.register_free_fn("URL", free));
            let src = "import Foundation\nlet u = URL()\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            interp.run(analysis).expect("import makes URL resolvable");
        }
        // Stdlib free-fn never gated.
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Swift", |i| i.register_free_fn("print", free));
            let src = "print()\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            interp
                .run(analysis)
                .expect("Swift stdlib is always imported");
        }
    }

    /// Phase D2 hole fix: when the receiver module owns a struct-method
    /// candidate that is gated out, resolution must NOT fall through to an
    /// imported different-module handler — it must error with the import hint
    /// for the *receiver's* module.
    #[test]
    fn strict_import_gates_receiver_owned_struct_method_no_cross_module_steal() {
        fn charts_fg(
            _ctx: &mut dyn StdContext,
            recv: SwiftValue,
            _args: Vec<Arg>,
        ) -> crate::stdlib::StdResult {
            Ok(recv)
        }
        fn swiftui_fg(
            _ctx: &mut dyn StdContext,
            recv: SwiftValue,
            _args: Vec<Arg>,
        ) -> crate::stdlib::StdResult {
            Ok(recv)
        }
        // Stdlib-owned factory that mints a Charts-typed receiver so we can
        // exercise the *method* gate without also tripping the free_fn gate
        // on `BarMark()` itself.
        fn make_bar(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> crate::stdlib::StdResult {
            Ok(SwiftValue::Struct(Rc::new(StructObj {
                type_name: "BarMark".into(),
                fields: vec![],
            })))
        }
        fn free(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> crate::stdlib::StdResult {
            Ok(SwiftValue::Struct(Rc::new(StructObj {
                type_name: "BarMark".into(),
                fields: vec![],
            })))
        }

        // Charts owns BarMark + foregroundStyle; SwiftUI also has foregroundStyle.
        // Only SwiftUI imported → must not steal SwiftUI's handler.
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Charts", |i| {
                i.register_free_fn("BarMark", free);
                i.register_struct_method("foregroundStyle", charts_fg);
            });
            interp.module("SwiftUI", |i| {
                i.register_struct_method("foregroundStyle", swiftui_fg);
            });
            interp.module("Swift", |i| {
                i.register_free_fn("makeBar", make_bar);
            });
            // Query API: receiver-owned candidate is gated → None (no steal).
            interp.mark_module_imported("SwiftUI");
            assert_eq!(
                interp.struct_method_module_for("foregroundStyle", "BarMark"),
                None,
                "must not fall through to SwiftUI when Charts owns the candidate"
            );
            let src = "import SwiftUI\nlet m = makeBar()\n_ = m.foregroundStyle()\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            let err = interp.run(analysis).expect_err("must fail without Charts");
            let msg = err.to_string();
            assert!(
                msg.contains("cannot find 'foregroundStyle' in scope")
                    && msg.contains("import Charts"),
                "unexpected diagnostic: {msg}"
            );
        }
        // With Charts imported: resolves to Charts' handler.
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Charts", |i| {
                i.register_free_fn("BarMark", free);
                i.register_struct_method("foregroundStyle", charts_fg);
            });
            interp.module("SwiftUI", |i| {
                i.register_struct_method("foregroundStyle", swiftui_fg);
            });
            assert_eq!(
                {
                    interp.mark_module_imported("Charts");
                    interp.struct_method_module_for("foregroundStyle", "BarMark")
                },
                Some("Charts")
            );
            let src = "import Charts\nlet m = BarMark()\n_ = m.foregroundStyle()\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            interp
                .run(analysis)
                .expect("import Charts makes BarMark.foregroundStyle resolvable");
        }
        // Receiver has NO own candidate → legitimate fall-through to imported
        // SwiftUI modifier (UserWidget.padding with only SwiftUI imported).
        {
            fn pad(
                _ctx: &mut dyn StdContext,
                recv: SwiftValue,
                _args: Vec<Arg>,
            ) -> crate::stdlib::StdResult {
                Ok(recv)
            }
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("SwiftUI", |i| {
                i.register_struct_method("padding", pad);
            });
            interp.mark_module_imported("SwiftUI");
            assert_eq!(
                interp.struct_method_module_for("padding", "UserWidget"),
                Some("SwiftUI")
            );
        }
    }

    /// Phase D2 hole fix: JSONEncoder/JSONDecoder/PropertyListEncoder require
    /// `import Foundation` (core-internal framework ctors).
    #[test]
    fn strict_import_gates_json_coder_builtin_ctors() {
        for name in ["JSONEncoder", "JSONDecoder", "PropertyListEncoder"] {
            // Without import.
            {
                let mut sink = std::io::sink();
                let mut interp = Interpreter::new(&mut sink);
                let src = format!("let _ = {name}()\n");
                let analysis = Analysis::analyze(&src, "test.swift").expect("analyze");
                let analysis: &'static Analysis = Box::leak(Box::new(analysis));
                let err = interp
                    .run(analysis)
                    .expect_err("must fail without Foundation");
                let msg = err.to_string();
                assert!(
                    msg.contains(&format!("cannot find '{name}' in scope"))
                        && msg.contains("import Foundation"),
                    "unexpected diagnostic for {name}: {msg}"
                );
            }
            // With import.
            {
                let mut sink = std::io::sink();
                let mut interp = Interpreter::new(&mut sink);
                let src = format!("import Foundation\nlet _ = {name}()\n");
                let analysis = Analysis::analyze(&src, "test.swift").expect("analyze");
                let analysis: &'static Analysis = Box::leak(Box::new(analysis));
                interp
                    .run(analysis)
                    .unwrap_or_else(|e| panic!("import Foundation makes {name} resolvable: {e}"));
            }
        }
    }

    /// Phase D2 hole fix: framework builtin-enum cases require their module.
    #[test]
    fn strict_import_gates_framework_builtin_enum_cases() {
        // Without Foundation: qualified `Type.case` fails with import hint.
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| {
                i.register_builtin_enum("RoundingMode", &["plain", "down", "up", "bankers"]);
            });
            let src = "let _ = RoundingMode.plain\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            let err = interp
                .run(analysis)
                .expect_err("must fail without Foundation");
            let msg = err.to_string();
            assert!(
                msg.contains("cannot find") && msg.contains("import Foundation"),
                "unexpected diagnostic: {msg}"
            );
        }
        // With Foundation: resolves.
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| {
                i.register_builtin_enum("RoundingMode", &["plain", "down", "up", "bankers"]);
            });
            let src = "import Foundation\nlet _ = RoundingMode.plain\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            interp
                .run(analysis)
                .expect("import Foundation makes enum case resolvable");
        }
        // Leading-dot under a typed contextual parameter. Free-fn is Swift-
        // owned so only the enum case is gated — proves contextual enum
        // resolution honors import visibility with the clear diagnostic.
        {
            fn take(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> crate::stdlib::StdResult {
                Ok(args
                    .into_iter()
                    .next()
                    .map(|a| a.value)
                    .unwrap_or(SwiftValue::Void))
            }
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| {
                i.register_builtin_enum("RoundingMode", &["plain", "down", "up", "bankers"]);
            });
            interp.module("Swift", |i| {
                i.register_free_fn_typed(
                    "useMode",
                    take,
                    vec![BuiltinParam::positional("RoundingMode")],
                );
            });
            let src = "let _ = useMode(.plain)\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            let err = interp
                .run(analysis)
                .expect_err("must fail without Foundation");
            let msg = err.to_string();
            assert!(
                msg.contains("cannot find") && msg.contains("import Foundation"),
                "unexpected diagnostic: {msg}"
            );
        }
        {
            fn take(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> crate::stdlib::StdResult {
                Ok(args
                    .into_iter()
                    .next()
                    .map(|a| a.value)
                    .unwrap_or(SwiftValue::Void))
            }
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| {
                i.register_builtin_enum("RoundingMode", &["plain", "down", "up", "bankers"]);
            });
            interp.module("Swift", |i| {
                i.register_free_fn_typed(
                    "useMode",
                    take,
                    vec![BuiltinParam::positional("RoundingMode")],
                );
            });
            let src = "import Foundation\nlet _ = useMode(.plain)\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            interp
                .run(analysis)
                .expect("import + leading-dot enum case resolves");
        }
    }

    /// Phase D2 hole fix: framework static reads/writes require their module.
    #[test]
    fn strict_import_gates_framework_statics() {
        // Static read without import.
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| {
                i.register_static_value("URLSession", "shared", SwiftValue::int(1));
            });
            let src = "let _ = URLSession.shared\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            let err = interp
                .run(analysis)
                .expect_err("must fail without Foundation");
            let msg = err.to_string();
            assert!(
                msg.contains("cannot find") && msg.contains("import Foundation"),
                "unexpected diagnostic: {msg}"
            );
        }
        // Static read with import.
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| {
                i.register_static_value("URLSession", "shared", SwiftValue::int(1));
            });
            let src = "import Foundation\nlet _ = URLSession.shared\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            interp
                .run(analysis)
                .expect("import Foundation makes static readable");
        }
        // Static write without / with import.
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| {
                i.register_static_value("Widget", "count", SwiftValue::int(0));
            });
            let src = "Widget.count = 2\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            let err = interp
                .run(analysis)
                .expect_err("must fail without Foundation");
            let msg = err.to_string();
            assert!(
                msg.contains("cannot find") && msg.contains("import Foundation"),
                "unexpected diagnostic: {msg}"
            );
        }
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| {
                i.register_static_value("Widget", "count", SwiftValue::int(0));
            });
            let src = "import Foundation\nWidget.count = 2\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            interp
                .run(analysis)
                .expect("import Foundation makes static writable");
        }
    }

    /// Phase D2 hole fix: framework enum `rawValue:` init requires the module.
    #[test]
    fn strict_import_gates_framework_enum_rawvalue_init() {
        // Without import: `FrameworkEnum(rawValue:)` is gated (not silent Nil).
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| {
                i.register_builtin_enum("RoundingMode", &["plain", "down", "up", "bankers"]);
            });
            let src = "let _ = RoundingMode(rawValue: 0)\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            let err = interp
                .run(analysis)
                .expect_err("must fail without Foundation");
            let msg = err.to_string();
            assert!(
                msg.contains("cannot find 'RoundingMode' in scope")
                    && msg.contains("import Foundation"),
                "unexpected diagnostic: {msg}"
            );
        }
        // With import: rawValue init is allowed (no matching raw → Nil is fine).
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| {
                i.register_builtin_enum("RoundingMode", &["plain", "down", "up", "bankers"]);
            });
            let src = "import Foundation\nlet _ = RoundingMode(rawValue: 0)\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            interp
                .run(analysis)
                .expect("import Foundation makes enum rawValue: init resolvable");
        }
    }

    /// Phase D2 hole fix: framework type `.self` metatype requires its module.
    #[test]
    fn strict_import_gates_framework_type_self_metatype() {
        // Builtin framework enum is a real `is_type_name` type stamped with a
        // module — `.self` must honor the import gate (not emit a metatype
        // without the owning module).
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| {
                i.register_builtin_enum("RoundingMode", &["plain", "down"]);
            });
            let src = "let _ = RoundingMode.self\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            let err = interp
                .run(analysis)
                .expect_err("must fail without Foundation");
            let msg = err.to_string();
            assert!(
                msg.contains("cannot find 'RoundingMode' in scope")
                    && msg.contains("import Foundation"),
                "unexpected diagnostic: {msg}"
            );
        }
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| {
                i.register_builtin_enum("RoundingMode", &["plain", "down"]);
            });
            let src = "import Foundation\nlet _ = RoundingMode.self\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            interp
                .run(analysis)
                .expect("import Foundation makes Type.self resolvable");
        }
        // Framework BuiltinReceivers (`Data`/`UUID`) are not nominal but still
        // carry type_modules stamps — `.self` must gate with the import hint.
        for type_name in ["Data", "UUID"] {
            {
                let mut sink = std::io::sink();
                let mut interp = Interpreter::new(&mut sink);
                interp.module("Foundation", |i| {
                    i.register_free_fn(type_name, |_, _| Ok(SwiftValue::Void));
                });
                let src = format!("let _ = {type_name}.self\n");
                let analysis = Analysis::analyze(&src, "test.swift").expect("analyze");
                let analysis: &'static Analysis = Box::leak(Box::new(analysis));
                let err = interp
                    .run(analysis)
                    .expect_err("must fail without Foundation");
                let msg = err.to_string();
                assert!(
                    msg.contains(&format!("cannot find '{type_name}' in scope"))
                        && msg.contains("import Foundation"),
                    "unexpected diagnostic for {type_name}.self: {msg}"
                );
            }
            {
                let mut sink = std::io::sink();
                let mut interp = Interpreter::new(&mut sink);
                interp.module("Foundation", |i| {
                    i.register_free_fn(type_name, |_, _| Ok(SwiftValue::Void));
                });
                let src = format!("import Foundation\nlet _ = {type_name}.self\n");
                let analysis = Analysis::analyze(&src, "test.swift").expect("analyze");
                let analysis: &'static Analysis = Box::leak(Box::new(analysis));
                interp.run(analysis).unwrap_or_else(|e| {
                    panic!("import Foundation makes {type_name}.self resolvable: {e}")
                });
            }
        }
        // Core Swift builtins stay ungated even when a Foundation free_fn
        // collides on the same type name (`String(data:encoding:)`).
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| {
                i.register_free_fn("String", |_, _| Ok(SwiftValue::Void));
            });
            let src = "let _ = String.self\nlet _ = Int.self\nlet _ = Array.self\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            interp
                .run(analysis)
                .expect("core Swift Type.self must not require Foundation");
        }
    }

    /// Phase D2 hole fix: gated struct-method (modifier) must not evaluate
    /// arguments before the import gate — no side effects, import-hint only.
    #[test]
    fn strict_import_gates_struct_method_before_arg_evaluation() {
        fn charts_fg(
            _ctx: &mut dyn StdContext,
            recv: SwiftValue,
            _args: Vec<Arg>,
        ) -> crate::stdlib::StdResult {
            Ok(recv)
        }
        fn swiftui_fg(
            _ctx: &mut dyn StdContext,
            recv: SwiftValue,
            _args: Vec<Arg>,
        ) -> crate::stdlib::StdResult {
            Ok(recv)
        }
        fn make_bar(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> crate::stdlib::StdResult {
            Ok(SwiftValue::Struct(Rc::new(StructObj {
                type_name: "BarMark".into(),
                fields: vec![],
            })))
        }
        fn free(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> crate::stdlib::StdResult {
            Ok(SwiftValue::Struct(Rc::new(StructObj {
                type_name: "BarMark".into(),
                fields: vec![],
            })))
        }
        fn boom(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> crate::stdlib::StdResult {
            // Would mutate state if evaluated; register_static_value counter is
            // written only when this free_fn runs.
            Err(crate::stdlib::StdError::Error(EvalError::Type(
                "boom side effect ran".into(),
            )))
        }

        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        interp.module("Charts", |i| {
            i.register_free_fn("BarMark", free);
            i.register_struct_method("foregroundStyle", charts_fg);
        });
        interp.module("SwiftUI", |i| {
            i.register_struct_method("foregroundStyle", swiftui_fg);
        });
        interp.module("Swift", |i| {
            i.register_free_fn("makeBar", make_bar);
            i.register_free_fn("boom", boom);
        });
        // Only SwiftUI imported: Charts-owned candidate is gated. `boom()` must
        // not run (would yield "boom side effect ran" instead of import hint).
        let src = "import SwiftUI\nlet m = makeBar()\n_ = m.foregroundStyle(boom())\n";
        let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        let err = interp.run(analysis).expect_err("must fail without Charts");
        let msg = err.to_string();
        assert!(
            msg.contains("cannot find 'foregroundStyle' in scope") && msg.contains("import Charts"),
            "unexpected diagnostic (args must not run): {msg}"
        );
        assert!(
            !msg.contains("boom side effect"),
            "arguments were evaluated before the import gate: {msg}"
        );
    }

    /// Phase D2 hole fix: gated contextual static must not fall through to
    /// another module's same-named static.
    #[test]
    fn strict_import_gates_contextual_static_no_cross_module_steal() {
        fn take(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> crate::stdlib::StdResult {
            Ok(args
                .into_iter()
                .next()
                .map(|a| a.value)
                .unwrap_or(SwiftValue::Void))
        }
        // Foundation owns Style.plain; SwiftUI owns Color.plain. Contextual
        // parameter is Style — without Foundation, must not steal Color.plain.
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| {
                i.register_static_value("Style", "plain", SwiftValue::int(1));
            });
            interp.module("SwiftUI", |i| {
                i.register_static_value("Color", "plain", SwiftValue::int(99));
            });
            interp.module("Swift", |i| {
                i.register_free_fn_typed("useStyle", take, vec![BuiltinParam::positional("Style")]);
            });
            let src = "import SwiftUI\nlet _ = useStyle(.plain)\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            let err = interp
                .run(analysis)
                .expect_err("must fail without Foundation");
            let msg = err.to_string();
            assert!(
                msg.contains("cannot find") && msg.contains("import Foundation"),
                "unexpected diagnostic: {msg}"
            );
        }
        // With Foundation imported: resolves to Style.plain (value 1).
        {
            let mut sink = std::io::sink();
            let mut interp = Interpreter::new(&mut sink);
            interp.module("Foundation", |i| {
                i.register_static_value("Style", "plain", SwiftValue::int(1));
            });
            interp.module("SwiftUI", |i| {
                i.register_static_value("Color", "plain", SwiftValue::int(99));
            });
            interp.module("Swift", |i| {
                i.register_free_fn_typed("useStyle", take, vec![BuiltinParam::positional("Style")]);
            });
            let src = "import Foundation\nimport SwiftUI\nlet _ = useStyle(.plain)\n";
            let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
            let analysis: &'static Analysis = Box::leak(Box::new(analysis));
            interp
                .run(analysis)
                .expect("import Foundation makes contextual Style.plain resolvable");
        }
    }

    #[test]
    fn global_members_registry_stores_free_fns_and_algorithms() {
        fn id(_: &mut dyn StdContext, _: Vec<Arg>) -> crate::stdlib::StdResult {
            Ok(SwiftValue::Void)
        }
        let mut g = GlobalMembers::default();
        assert!(g.free_fn("abs").is_none());
        g.add_free_fn(
            "abs",
            FreeFnEntry {
                f: id,
                params: None,
                module: ModuleId::swift(),
            },
        );
        assert!(g.free_fn("abs").is_some());
        assert!(g.algorithm("map").is_none());
        let names: Vec<String> = g.free_fn_names().cloned().collect();
        assert_eq!(names, vec!["abs".to_string()]);
    }

    #[test]
    fn builtin_members_registry_stores_and_looks_up_by_receiver() {
        fn count(recv: SwiftValue) -> crate::stdlib::StdResult {
            Ok(recv)
        }
        let mut b = BuiltinMembers::default();
        assert!(b.property(BuiltinReceiver::Array, "count").is_none());
        b.add_property(BuiltinReceiver::Array, "count", count, ModuleId::swift());
        assert!(b.property(BuiltinReceiver::Array, "count").is_some());
        // Distinct receivers do not collide on the same member name.
        assert!(b.property(BuiltinReceiver::String, "count").is_none());
        assert!(!b.has_labeled_intrinsic(BuiltinReceiver::Array, "count"));
        let names: Vec<String> = b.qualified_names().collect();
        assert_eq!(names, vec!["Array.count".to_string()]);
    }

    #[test]
    fn type_table_all_protocols_is_transitive() {
        let mut types = TypeTable::default();
        // Equatable inherits nothing; Hashable inherits Equatable.
        types.ensure_protocol("Equatable".into(), vec![]);
        types.ensure_protocol("Hashable".into(), vec!["Equatable".into()]);
        types.record_conformance("Point", vec!["Hashable".into()]);
        let mut protos = types.all_protocols("Point");
        protos.sort();
        assert_eq!(
            protos,
            vec!["Equatable".to_string(), "Hashable".to_string()]
        );
        // Composition alias expands to its members.
        types.add_protocol_alias(
            "Codable".into(),
            vec!["Encodable".into(), "Decodable".into()],
        );
        types.record_conformance("Model", vec!["Codable".into()]);
        let mut m = types.all_protocols("Model");
        m.sort();
        assert_eq!(m, vec!["Decodable".to_string(), "Encodable".to_string()]);
    }

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
    fn enum_hint_pop_survives_rhs_error_in_equality() {
        // `lhs == rhs` / `lhs != rhs` pushes the lhs enum type as a contextual
        // hint while evaluating `rhs` (so a leading-dot shorthand on the rhs
        // resolves against that enum). If `rhs` errors/throws, the hint must
        // still be popped — an early `?` before the pop would leak it into
        // whatever implicit-member resolution runs next.
        let src = concat!(
            "enum Color { case red }\n",
            "enum MyError: Error { case boom }\n",
            "func rhsThrows() throws -> Color { throw MyError.boom }\n",
            "let c: Color = .red\n",
            "_ = c == (try rhsThrows())\n",
        );
        let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        let mut buf: Vec<u8> = Vec::new();
        let mut interp = Interpreter::new(&mut buf);
        crate::install_test_print(&mut interp);
        let result = interp.run(analysis);
        assert!(
            result.is_err(),
            "the uncaught throw from rhs should propagate as an error"
        );
        assert!(
            interp.type_hint.is_empty(),
            "the `==` enum contextual hint must be popped even when \
             evaluating the rhs errors, not only on the success path \
             (found: {:?})",
            interp.type_hint,
        );
    }

    #[test]
    fn builtin_collection_and_conversion_ctors_still_resolve() {
        // The table-ized value-only builtins keep working through the single
        // consult point in `eval_call`.
        let src = concat!(
            "let a: [Int] = Array(repeating: 7, count: 3)\n",
            "print(a)\n",
            "print(Int(\"42\")!)\n",
            "let d = Dictionary(uniqueKeysWithValues: [(\"x\", 1)])\n",
            "print(d[\"x\"]!)\n",
        );
        assert_eq!(run(src).unwrap(), "[7, 7, 7]\n42\n1\n");
    }

    #[test]
    fn table_entries_cover_sequence_and_grouping_initializers() {
        // Every table entry reachable through the old conversion ladder still
        // resolves: sequence-conversion ctors, CollectionOfOne, and the
        // `Dictionary(grouping:by:)` form.
        let src = concat!(
            "print(Array(1...3))\n",
            "print(Array(Set([1, 1, 2])))\n",
            "print(ContiguousArray([4, 5]))\n",
            "print(Array(CollectionOfOne(42)))\n",
            "let g = Dictionary(grouping: [1, 2, 3, 4], by: { $0 % 2 })\n",
            "print(g[0]!, g[1]!)\n",
            "print(Double(\"2.5\")!, Bool(\"true\")!)\n",
        );
        assert_eq!(
            run(src).unwrap(),
            "[1, 2, 3]\n[1, 2]\n[4, 5]\n[42]\n[2, 4] [1, 3]\n2.5 true\n",
        );
    }

    #[test]
    fn user_types_shadow_collection_and_scalar_builtins() {
        // A user type named after a table builtin must win over the builtin
        // constructor — collection names (`Array`, `Dictionary`) and scalar
        // conversion names (`Int`) alike.
        let src = concat!(
            "struct Array { let tag = \"S\" }\n",
            "class Dictionary { let tag = \"C\" }\n",
            "struct Int { let tag = \"I\" }\n",
            "print(Array().tag)\n",
            "print(Dictionary().tag)\n",
            "print(Int().tag)\n",
        );
        assert_eq!(run(src).unwrap(), "S\nC\nI\n");
    }

    #[test]
    fn user_struct_shadows_builtin_json_encoder() {
        // A user `struct JSONEncoder` must win over the builtin marker. Before
        // table-ization the marker was matched before user-type dispatch with no
        // shadow guard; now it is consulted after, gated by `is_unshadowed`.
        let src = concat!(
            "struct JSONEncoder { let tag = \"user\" }\n",
            "print(JSONEncoder().tag)\n",
        );
        assert_eq!(run(src).unwrap(), "user\n");
    }

    #[test]
    fn user_function_shadows_builtin_int_conversion() {
        // A user `func Int(_:)` must win over the builtin conversion initializer.
        let src = concat!(
            "func Int(_ s: String) -> String { \"user:\\(s)\" }\n",
            "print(Int(\"42\"))\n",
        );
        assert_eq!(run(src).unwrap(), "user:42\n");
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
    fn typed_builtin_free_fn_pushes_contextual_type_for_member_arg() {
        // Two token namespaces share the leading-dot name `.center`. A builtin
        // free function declared to take an `Align` resolves the ambiguous
        // member against its parameter type (uniqueness alone would fail).
        fn stack(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> crate::StdResult {
            let token = args
                .iter()
                .find(|a| a.label.as_deref() == Some("alignment"))
                .and_then(|a| match &a.value {
                    SwiftValue::Struct(o) => o
                        .fields
                        .iter()
                        .find(|(k, _)| k == "token")
                        .map(|(_, v)| v.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            Ok(SwiftValue::Str(token))
        }
        let src = concat!(
            "struct Align { let token: String\n",
            "  static let leading = Align(token: \"a-leading\")\n",
            "  static let center = Align(token: \"a-center\") }\n",
            "struct TextAlign { let token: String\n",
            "  static let center = TextAlign(token: \"t-center\") }\n",
            "print(stack(alignment: .center))\n",
        );
        let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut interp = Interpreter::new(&mut buf);
            crate::install_test_print(&mut interp);
            interp.register_free_fn_typed(
                "stack",
                stack,
                vec![BuiltinParam::labeled("alignment", "Align")],
            );
            interp.run(analysis).expect("run");
        }
        assert_eq!(String::from_utf8(buf).unwrap(), "a-center\n");
    }

    #[test]
    fn typed_builtin_struct_method_pushes_contextual_type_for_member_arg() {
        // The modifier seam: a builtin struct method declared to take an `Edge`
        // resolves `.horizontal` against `Edge` even though `Axis` also declares
        // it (collision that bare uniqueness cannot disambiguate).
        fn pad(_ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> crate::StdResult {
            let _ = recv;
            let token = args
                .first()
                .and_then(|a| match &a.value {
                    SwiftValue::Struct(o) => o
                        .fields
                        .iter()
                        .find(|(k, _)| k == "token")
                        .map(|(_, v)| v.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            Ok(SwiftValue::Str(token))
        }
        let src = concat!(
            "struct Edge { let token: String\n",
            "  static let horizontal = Edge(token: \"e-horizontal\") }\n",
            "struct Axis { let token: String\n",
            "  static let horizontal = Axis(token: \"x-horizontal\") }\n",
            "struct View {}\n",
            "print(View().pad(.horizontal))\n",
        );
        let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut interp = Interpreter::new(&mut buf);
            crate::install_test_print(&mut interp);
            interp.register_struct_method_typed("pad", pad, vec![BuiltinParam::positional("Edge")]);
            interp.run(analysis).expect("run");
        }
        assert_eq!(String::from_utf8(buf).unwrap(), "e-horizontal\n");
    }

    #[test]
    fn contextual_float_constant_beats_unrelated_unique_static() {
        // A user type declaring `static let infinity` must not steal a
        // contextually-floating `.infinity` (regression: the float constant is
        // resolved before the unique-static fallback).
        fn frame(_ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> crate::StdResult {
            let _ = recv;
            let v = args
                .first()
                .map(|a| a.value.clone())
                .unwrap_or(SwiftValue::Nil);
            Ok(SwiftValue::Bool(
                matches!(v, SwiftValue::Double(d) if d.is_infinite()),
            ))
        }
        let src = concat!(
            "struct Sentinel { let token: String\n",
            "  static let infinity = Sentinel(token: \"not-a-number\") }\n",
            "struct View {}\n",
            "print(View().frame(.infinity))\n",
        );
        let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut interp = Interpreter::new(&mut buf);
            crate::install_test_print(&mut interp);
            interp.register_struct_method_typed(
                "frame",
                frame,
                vec![BuiltinParam::positional("CGFloat")],
            );
            interp.run(analysis).expect("run");
        }
        assert_eq!(String::from_utf8(buf).unwrap(), "true\n");
    }

    #[test]
    fn builtin_free_fn_hints_do_not_leak_to_shadowing_user_type() {
        // A user `struct` shadowing a typed builtin free fn dispatches to its
        // own initializer; the builtin's parameter hints must not be pushed
        // while evaluating that initializer's arguments. Here `.warm` is a
        // genuinely ambiguous member (both `Tone` and `Mood` declare it), so the
        // correct outcome is an unresolved-member error — *not* the builtin
        // `Mood` hint silently resolving it to `mood-warm` (regression).
        fn builtin(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> crate::StdResult {
            Ok(SwiftValue::Str("from-builtin".into()))
        }
        let src = concat!(
            "struct Tone { let token: String\n",
            "  static let warm = Tone(token: \"tone-warm\") }\n",
            "struct Mood { let token: String\n",
            "  static let warm = Mood(token: \"mood-warm\") }\n",
            "struct Card { let label: String\n",
            "  init(_ t: Tone) { label = t.token } }\n",
            "print(Card(.warm).label)\n",
        );
        let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        let mut buf: Vec<u8> = Vec::new();
        let result = {
            let mut interp = Interpreter::new(&mut buf);
            crate::install_test_print(&mut interp);
            interp.register_free_fn_typed("Card", builtin, vec![BuiltinParam::positional("Mood")]);
            interp.run(analysis)
        };
        assert!(
            result.is_err(),
            "ambiguous `.warm` must not silently resolve"
        );
        assert!(
            !String::from_utf8(buf).unwrap().contains("mood-warm"),
            "builtin `Mood` hint must not leak into the shadowing user type"
        );
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
    fn repeated_interpolation_analyzes_fragment_once() {
        // A loop printing the same "\(i)" fragment N times must cache one
        // analysis, not leak one per render (ADR-0007).
        let mut buf: Vec<u8> = Vec::new();
        let mut interp = Interpreter::new(&mut buf);
        for _ in 0..50 {
            interp.eval_interpolation("1 + 2").expect("eval");
        }
        assert_eq!(interp.fragment_cache.len(), 1);
    }

    #[test]
    fn distinct_interpolations_grow_cache_by_distinct_count() {
        let mut buf: Vec<u8> = Vec::new();
        let mut interp = Interpreter::new(&mut buf);
        // Three distinct fragments, each evaluated twice → cache holds three.
        for _ in 0..2 {
            interp.eval_interpolation("1 + 2").expect("eval");
            interp.eval_interpolation("3 * 4").expect("eval");
            interp.eval_interpolation("5 - 1").expect("eval");
        }
        assert_eq!(interp.fragment_cache.len(), 3);
    }

    #[test]
    fn dropping_interpreter_reclaims_fragment_cache() {
        // Scoped block: the cache (and its boxed analyses) drop with the
        // interpreter, so nothing leaks across sessions.
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut interp = Interpreter::new(&mut buf);
            interp.eval_interpolation("42").expect("eval");
            assert_eq!(interp.fragment_cache.len(), 1);
        }
        // A fresh interpreter starts with an empty cache.
        let interp = Interpreter::new(&mut buf);
        assert_eq!(interp.fragment_cache.len(), 0);
    }

    #[test]
    fn nominal_type_info_reports_struct_attributes() {
        use crate::stdlib::StdContext;
        let analysis =
            Analysis::analyze("@Model struct Movie { var title: String }\n", "test.swift")
                .expect("analyze");
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        let mut buf: Vec<u8> = Vec::new();
        let mut interp = Interpreter::new(&mut buf);
        interp.run(analysis).expect("run");
        let info = StdContext::nominal_type_info(&interp, "Movie").expect("info");
        // Struct declaration attributes are reported (not fabricated-empty).
        assert!(info.attributes.iter().any(|a| a == "Model"));
        assert_eq!(info.stored.len(), 1);
        assert_eq!(info.stored[0].name, "title");
    }

    #[test]
    fn registered_finalizers_run_on_drop_and_teardown_is_idempotent() {
        use crate::stdlib::StdContext;
        use std::cell::Cell;
        use std::rc::Rc as R;
        let ran = R::new(Cell::new(0));
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut interp = Interpreter::new(&mut buf);
            let r = R::clone(&ran);
            interp.register_finalizer(Box::new(move |_ctx| r.set(r.get() + 1)));
            assert_eq!(ran.get(), 0, "finalizer must not run at registration");
            // Explicit teardown runs it once; drop must not run it again.
            interp.teardown();
            assert_eq!(ran.get(), 1);
        }
        assert_eq!(ran.get(), 1, "finalizer runs exactly once");
    }

    #[test]
    fn view_scope_hooks_run_in_registration_and_reverse_order() {
        use crate::stdlib::StdContext;
        // Two hooks record their enter/exit tags into a thread-local log; the
        // renderer seam must call enters in registration order and exits in
        // reverse (LIFO), so nested subtree state unwinds correctly.
        thread_local! {
            static LOG: std::cell::RefCell<Vec<&'static str>> =
                const { std::cell::RefCell::new(Vec::new()) };
        }
        fn enter_a(_c: &mut dyn StdContext, _v: &SwiftValue) {
            LOG.with(|l| l.borrow_mut().push("enterA"));
        }
        fn exit_a(_c: &mut dyn StdContext, _v: &SwiftValue) {
            LOG.with(|l| l.borrow_mut().push("exitA"));
        }
        fn enter_b(_c: &mut dyn StdContext, _v: &SwiftValue) {
            LOG.with(|l| l.borrow_mut().push("enterB"));
        }
        fn exit_b(_c: &mut dyn StdContext, _v: &SwiftValue) {
            LOG.with(|l| l.borrow_mut().push("exitB"));
        }
        let mut buf: Vec<u8> = Vec::new();
        let mut interp = Interpreter::new(&mut buf);
        interp.register_view_scope(enter_a, exit_a);
        interp.register_view_scope(enter_b, exit_b);
        let view = SwiftValue::Void;
        StdContext::view_scope_enter(&mut interp, &view);
        StdContext::view_scope_exit(&mut interp, &view);
        LOG.with(|l| {
            assert_eq!(*l.borrow(), vec!["enterA", "enterB", "exitB", "exitA"]);
        });
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
    fn deinit_fires_when_last_strong_reference_is_reassigned() {
        let out = run(concat!(
            "class R { deinit { print(\"freed\") } }\n",
            "var o: R? = R()\n",
            "o = nil\n",
            "print(\"after\")\n",
        ))
        .unwrap();
        assert_eq!(out, "freed\nafter\n");
    }

    #[test]
    fn weak_capture_zeroes_and_does_not_retain() {
        let out = run(concat!(
            "class O {\n",
            "  var name = \"o\"\n",
            "  var h: (() -> String)? = nil\n",
            "  func setup() {\n",
            "    h = { [weak self] in\n",
            "      guard let self = self else { return \"gone\" }\n",
            "      return self.name\n",
            "    }\n",
            "  }\n",
            "  deinit { print(\"deinit\") }\n",
            "}\n",
            "var o: O? = O()\n",
            "o!.setup()\n",
            "let h = o!.h!\n",
            "print(h())\n",
            "o = nil\n",
            "print(h())\n",
        ))
        .unwrap();
        assert_eq!(out, "o\ndeinit\ngone\n");
    }

    #[test]
    fn global_computed_var_and_observers_run_accessors() {
        let out = run(concat!(
            "var stored = 10\n",
            "var doubled: Int { stored * 2 }\n",
            "print(doubled)\n",
            "var score = 0 {\n",
            "  willSet { print(\"will \\(newValue)\") }\n",
            "  didSet { print(\"did \\(oldValue)\") }\n",
            "}\n",
            "score = 5\n",
            "print(score)\n",
        ))
        .unwrap();
        assert_eq!(out, "20\nwill 5\ndid 0\n5\n");
    }

    #[test]
    fn weak_alias_capture_does_not_retain_its_source() {
        let out = run(concat!(
            "class O {\n",
            "  var v = 3\n",
            "  var h: (() -> Int)? = nil\n",
            "  func setup() {\n",
            "    h = { [weak owner = self] in owner?.v ?? -1 }\n",
            "  }\n",
            "  deinit { print(\"deinit\") }\n",
            "}\n",
            "var o: O? = O()\n",
            "o!.setup()\n",
            "let h = o!.h!\n",
            "print(h())\n",
            "o = nil\n",
            "print(h())\n",
        ))
        .unwrap();
        assert_eq!(out, "3\ndeinit\n-1\n");
    }

    #[test]
    fn unowned_capture_traps_after_referent_deallocates() {
        let err = run(concat!(
            "class O { var v = 1 }\n",
            "var o: O? = O()\n",
            "let peek = { [unowned obj = o!] in obj.v }\n",
            "print(peek())\n",
            "o = nil\n",
            "print(peek())\n",
        ))
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unowned"),
            "expected an unowned-dangling trap, got: {msg}"
        );
    }

    #[test]
    fn integer_generic_parameter_is_not_assignable() {
        let err = run(concat!(
            "struct Buf<let N: Int> {\n",
            "  var used = 0\n",
            "  mutating func corrupt() { N = 99 }\n",
            "}\n",
            "var b = Buf<4>()\n",
            "b.corrupt()\n",
        ))
        .unwrap_err();
        assert!(
            matches!(err, EvalError::Immutable(_)),
            "expected an immutability error, got: {err}"
        );
    }

    #[test]
    fn recursive_init_delegation_traps_instead_of_overflowing() {
        // Run on a generous stack so the depth guard fires before any native
        // overflow (matching `deep_recursion_traps_not_crashes`).
        let handle = std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                run(concat!(
                    "class Loop {\n",
                    "  var x: Int\n",
                    "  init(x: Int) { self.x = x }\n",
                    "  convenience init(loop n: Int) { self.init(loop: n) }\n",
                    "}\n",
                    "let _ = Loop(loop: 1)\n",
                ))
            })
            .unwrap();
        let err = handle.join().unwrap().unwrap_err();
        assert!(
            err.to_string().contains("delegation too deep"),
            "expected a clean delegation trap, got: {err}"
        );
    }

    #[test]
    fn self_init_outside_an_initializer_is_rejected() {
        let err = run(concat!(
            "class C {\n",
            "  var x = 1\n",
            "  func reset() { self.init() }\n",
            "}\n",
            "C().reset()\n",
        ))
        .unwrap_err();
        assert!(
            err.to_string().contains("inside an initializer"),
            "expected an initializer-context error, got: {err}"
        );
    }

    #[test]
    fn nil_callee_calls_and_nil_base_subscripts_nil_propagate() {
        // The parser drops the optional-chain `?` (as it does for `?.`), so
        // the runtime nil-propagates in call/subscript position whenever the
        // callee/base is nil — including the unchained spelling, which real
        // Swift rejects at compile time. Documented permissiveness: this
        // runtime has no chain marker to distinguish the two.
        let out = run(concat!(
            "var f: ((Int) -> Void)? = nil\n",
            "f?(1)\n",
            "let a: [Int]? = nil\n",
            "print(a?[0] ?? -1)\n",
            "let s: String? = nil\n",
            "print(s?.hasPrefix(\"x\") ?? false)\n",
        ))
        .unwrap();
        assert_eq!(out, "-1\nfalse\n");
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
    fn mutating_method_preserves_receiver_uniqueness() {
        // Regression guard: a `mutating` method must receive its `self` by move,
        // not by an extra clone, or `isKnownUniquelyReferenced` on a stored
        // class reference sees a phantom retain and copy-on-write copies when it
        // should mutate in place. Exercises the struct-method dispatch path in
        // the fast core signal (the CLI golden fixture is not in that signal).
        let src = concat!(
            "final class Box { var n: Int\n",
            "  init(_ n: Int) { self.n = n } }\n",
            "struct W {\n",
            "  var box: Box\n",
            "  init(_ n: Int) { box = Box(n) }\n",
            "  mutating func touch() {\n",
            "    if isKnownUniquelyReferenced(&box) { print(\"unique\") }\n",
            "    else { print(\"shared\") }\n",
            "  }\n",
            "}\n",
            "var a = W(1)\n",
            "a.touch()\n", // receiver moved in -> box uniquely referenced
            "var b = a\n",
            "b.touch()\n", // struct copy shares the box -> not unique
        );
        let out = run(src).unwrap();
        assert_eq!(out, "unique\nshared\n");
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
        assert_eq!(out, "[4, 27]\n[\"area=4\", \"area=27\"]\n");
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
        // encode returns Data; round-trip through decode to verify end-to-end.
        let out = run(
            "import Foundation\nstruct User: Codable { let name: String; let age: Int }\n@main struct App {\n  static func main() throws {\n    let u = User(name: \"Sam\", age: 30)\n    let data = try JSONEncoder().encode(u)\n    let back = try JSONDecoder().decode(User.self, from: data)\n    print(back.name, back.age)\n  }\n}\n",
        )
        .unwrap();
        assert_eq!(out, "Sam 30\n");
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

    // -----------------------------------------------------------------------
    // StdContext HTTP forwarding (M2)
    // -----------------------------------------------------------------------

    /// Build an interpreter with a single-route mock transport installed.
    fn interp_with_mock<'a>(buf: &'a mut Vec<u8>, url: &str, body: &[u8]) -> Interpreter<'a> {
        use crate::http::{HttpResponse, MockHttpTransport, MockRoute};
        let mut interp = Interpreter::new(buf);
        crate::install_test_print(&mut interp);
        interp.set_http_transport(Box::new(MockHttpTransport::new(vec![MockRoute {
            method: "GET".into(),
            url: url.into(),
            outcome: Ok(HttpResponse {
                status: 200,
                headers: vec![("Content-Type".into(), "text/plain".into())],
                body: body.to_vec(),
            }),
        }])));
        interp
    }

    #[test]
    fn std_context_http_start_next_event_cancel_forward_to_transport() {
        use crate::http::{HttpEvent, HttpRequest};
        use crate::stdlib::StdContext;
        let mut buf = Vec::new();
        let url = "https://example.com/test";
        let mut interp = interp_with_mock(&mut buf, url, b"hello");
        let req = HttpRequest {
            url: url.into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: 60.0,
        };
        // http_start should return a valid handle (no Unavailable)
        let h = interp.http_start(&req).expect("http_start");
        // next_event: first event is Response
        let e0 = interp.http_next_event(h);
        assert!(matches!(e0, HttpEvent::Response { status: 200, .. }));
        // next_event: body chunk
        let e1 = interp.http_next_event(h);
        assert_eq!(e1, HttpEvent::Chunk(b"hello".to_vec()));
        // next_event: terminal Done
        let e2 = interp.http_next_event(h);
        assert_eq!(e2, HttpEvent::Done);
    }

    #[test]
    fn std_context_http_start_returns_unavailable_without_transport() {
        use crate::http::HttpError;
        use crate::http::HttpRequest;
        use crate::stdlib::StdContext;
        let mut buf = Vec::new();
        let mut interp = Interpreter::new(&mut buf);
        crate::install_test_print(&mut interp);
        // No transport installed — http_start must return Unavailable
        let req = HttpRequest {
            url: "https://example.com/".into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: 60.0,
        };
        let err = interp.http_start(&req).unwrap_err();
        assert_eq!(err, HttpError::Unavailable);
    }

    #[test]
    fn std_context_http_cancel_yields_cancelled_then_sentinel() {
        use crate::http::{HttpEvent, HttpRequest};
        use crate::stdlib::StdContext;
        let mut buf = Vec::new();
        let url = "https://example.com/cancel";
        let mut interp = interp_with_mock(&mut buf, url, b"data");
        let req = HttpRequest {
            url: url.into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: 60.0,
        };
        let h = interp.http_start(&req).expect("http_start");
        interp.http_cancel(h);
        // First post-cancel poll must return Failed{cancelled} per cancel contract
        let e = interp.http_next_event(h);
        assert!(
            matches!(e, HttpEvent::Failed { ref code, .. } if code == "cancelled"),
            "expected Failed{{cancelled}}, got {e:?}"
        );
        // After the terminal is consumed the handle is dead — sentinel
        let sentinel = interp.http_next_event(h);
        assert!(
            matches!(sentinel, HttpEvent::Failed { ref code, .. } if code == "badServerResponse"),
            "expected badServerResponse sentinel, got {sentinel:?}"
        );
    }

    #[test]
    fn std_context_current_task_cancelled_is_false_at_top_level() {
        let mut buf = Vec::new();
        let interp = Interpreter::new(&mut buf);
        // At top level (no running task) cancellation flag is false.
        // current_task_cancelled is both a pub(super) inherent method on
        // Interpreter and a StdContext trait method; both return false here.
        assert!(!interp.current_task_cancelled());
    }

    // -----------------------------------------------------------------------
    // IMPORTANT fix: ResponseDispositionCapture token staleness (M4 review)
    // -----------------------------------------------------------------------

    /// Verify that calling a *stale* `ResponseDispositionCapture` closure
    /// (one allocated for a previous request) does NOT poison the current
    /// request's disposition.
    ///
    /// Scenario:
    ///   1. Request A allocates closure C_A, fires with `.cancel` → disposition=false.
    ///   2. `take_response_disposition` reads false → resets to None.
    ///   3. Request B allocates closure C_B (token advances), resets disposition=None.
    ///   4. Script late-calls C_A with `.cancel` → stale token, ignored.
    ///   5. `take_response_disposition` returns the default (true = allow).
    #[test]
    fn stale_response_disposition_closure_does_not_poison_next_request() {
        use crate::stdlib::StdContext;

        let mut buf = Vec::new();
        let mut interp = Interpreter::new(&mut buf);

        // Helper: build a `.cancel` enum value.
        fn cancel_enum() -> SwiftValue {
            SwiftValue::Enum(Rc::new(EnumObj {
                type_name: "URLSession.ResponseDisposition".into(),
                case: "cancel".into(),
                payload: vec![],
            }))
        }

        // ---- Request A ----
        let id_a = interp.allocate_response_disposition_closure();
        // Simulate delegate calling completionHandler(.cancel).
        interp
            .call_closure(id_a, vec![cancel_enum()])
            .expect("call C_A");
        let disp_a = interp.take_response_disposition();
        assert!(!disp_a, "request A disposition should be false (cancel)");

        // ---- Request B ----
        // allocate advances the token and clears disposition.
        let _id_b = interp.allocate_response_disposition_closure();
        // Request B's delegate has NOT called the handler yet (synchronous
        // delegate called its handler immediately in the test above, but here
        // we simulate the late-call scenario first).

        // ---- Late call from A ----
        // The stale closure C_A fires with .cancel.  Token mismatch → ignored.
        interp
            .call_closure(id_a, vec![cancel_enum()])
            .expect("late call C_A is a no-op, not an error");

        // Request B's take must return the default (true = allow) because the
        // late call from A was silently discarded.
        let disp_b = interp.take_response_disposition();
        assert!(
            disp_b,
            "request B disposition must be true (allow, default) — not poisoned by stale A handler"
        );
    }

    // -----------------------------------------------------------------------
    // Host-native function bridge (Epic #246) — end-to-end through the
    // interpreter's call dispatch.
    // -----------------------------------------------------------------------

    /// A configurable mock host handler: replies with a canned JSON string, or
    /// echoes the received `args_json` so a test can assert on the encoding.
    struct MockHostHandler {
        reply: std::sync::Mutex<Box<dyn Fn(&str, &str) -> Result<String, String> + Send>>,
    }

    impl MockHostHandler {
        fn new(f: impl Fn(&str, &str) -> Result<String, String> + Send + 'static) -> Self {
            MockHostHandler {
                reply: std::sync::Mutex::new(Box::new(f)),
            }
        }
    }

    impl crate::host_bridge::HostCallHandler for MockHostHandler {
        fn call(&self, name: &str, args_json: &str) -> Result<String, String> {
            (self.reply.lock().unwrap())(name, args_json)
        }
    }

    /// Run `src` against an interpreter with the given host functions installed.
    fn run_with_host(
        src: &str,
        register: impl FnOnce(&mut Interpreter),
    ) -> Result<String, EvalError> {
        let analysis = Analysis::analyze(src, "test.swift").expect("analyze");
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut interp = Interpreter::new(&mut buf);
            crate::install_test_print(&mut interp);
            register(&mut interp);
            interp.run(analysis)?;
        }
        Ok(String::from_utf8(buf).unwrap())
    }

    #[test]
    fn host_fn_success_round_trips_through_interpreter() {
        let out = run_with_host("print(sum(2, 3))\n", |interp| {
            let handler = std::sync::Arc::new(MockHostHandler::new(|_name, args| {
                let crate::json::Json::Array(items) = crate::json::parse(args).unwrap() else {
                    return Err("expected array".into());
                };
                let (crate::json::Json::Int(a), crate::json::Json::Int(b)) = (&items[0], &items[1])
                else {
                    return Err("expected ints".into());
                };
                Ok(format!("{}", a + b))
            }));
            interp
                .register_host_fn(
                    r#"{"name":"sum","params":[{"type":"Int"},{"type":"Int"}],"returns":"Int"}"#,
                    Some(handler),
                )
                .unwrap();
        })
        .unwrap();
        assert_eq!(out, "5\n");
    }

    #[test]
    fn host_fn_encodes_labels_and_string_return() {
        let out = run_with_host("print(greet(name: \"Sam\"))\n", |interp| {
            let handler = std::sync::Arc::new(MockHostHandler::new(|_name, args| {
                let crate::json::Json::Array(items) = crate::json::parse(args).unwrap() else {
                    return Err("expected array".into());
                };
                let crate::json::Json::Str(who) = &items[0] else {
                    return Err("expected string".into());
                };
                Ok(format!("\"Hello, {who}\""))
            }));
            interp
                .register_host_fn(
                    r#"{"name":"greet","params":[{"label":"name","type":"String"}],"returns":"String"}"#,
                    Some(handler),
                )
                .unwrap();
        })
        .unwrap();
        assert_eq!(out, "Hello, Sam\n");
    }

    #[test]
    fn host_fn_wrong_result_type_is_runtime_type_error() {
        let err = run_with_host("print(count())\n", |interp| {
            let handler =
                std::sync::Arc::new(MockHostHandler::new(|_n, _a| Ok(r#""nope""#.into())));
            interp
                .register_host_fn(r#"{"name":"count","returns":"Int"}"#, Some(handler))
                .unwrap();
        })
        .unwrap_err();
        assert!(
            matches!(err, EvalError::Type(ref m) if m.contains("count") && m.contains("bad result")),
            "got {err:?}"
        );
    }

    #[test]
    fn host_fn_handler_error_is_runtime_error_naming_fn() {
        let err = run_with_host("ping()\n", |interp| {
            let handler = std::sync::Arc::new(MockHostHandler::new(|_n, _a| Err("kaboom".into())));
            interp
                .register_host_fn(r#"{"name":"ping","returns":"Void"}"#, Some(handler))
                .unwrap();
        })
        .unwrap_err();
        assert!(
            matches!(err, EvalError::Type(ref m) if m.contains("ping") && m.contains("kaboom")),
            "got {err:?}"
        );
    }

    #[test]
    fn host_fn_thrown_payload_is_catchable_swift_error() {
        // The `{"$thrown": …}` payload becomes a catchable Swift error, so a
        // do/catch around the call recovers and reads its message.
        let src = concat!(
            "struct HostError: Error { let message: String }\n",
            "func go() {\n",
            "  do { try risky() }\n",
            "  catch let e as HostError { print(\"caught \\(e.message)\") }\n",
            "  catch { print(\"other\") }\n",
            "}\n",
            "go()\n",
        );
        let out = run_with_host(src, |interp| {
            let handler = std::sync::Arc::new(MockHostHandler::new(|_n, _a| {
                Ok(r#"{"$thrown":"disk full"}"#.into())
            }));
            interp
                .register_host_fn(
                    r#"{"name":"risky","returns":"Int","throws":true}"#,
                    Some(handler),
                )
                .unwrap();
        })
        .unwrap();
        assert_eq!(out, "caught disk full\n");
    }

    #[test]
    fn host_fn_arg_count_mismatch_is_runtime_error() {
        // `sum` declared with two params, called with one.
        let err = run_with_host("print(sum(1))\n", |interp| {
            let handler = std::sync::Arc::new(MockHostHandler::new(|_n, _a| Ok("0".into())));
            interp
                .register_host_fn(
                    r#"{"name":"sum","params":[{"type":"Int"},{"type":"Int"}],"returns":"Int"}"#,
                    Some(handler),
                )
                .unwrap();
        })
        .unwrap_err();
        assert!(
            matches!(err, EvalError::Type(ref m) if m.contains("sum") && m.contains("expects 2")),
            "got {err:?}"
        );
    }

    #[test]
    fn user_type_shadows_host_fn() {
        // A same-named user struct must win over the registered host fn, so
        // the host handler is never consulted (correct Swift shadowing).
        let src = concat!("struct sum { let tag = \"S\" }\n", "print(sum().tag)\n",);
        let out = run_with_host(src, |interp| {
            let handler = std::sync::Arc::new(MockHostHandler::new(|_n, _a| {
                Err("should not be called".into())
            }));
            interp
                .register_host_fn(r#"{"name":"sum","returns":"Void"}"#, Some(handler))
                .unwrap();
        })
        .unwrap();
        assert_eq!(out, "S\n");
    }
}
