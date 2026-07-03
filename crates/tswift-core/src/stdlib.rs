//! The standard-library dispatch seam.
//!
//! `tswift-std` plugs native Swift builtins into the interpreter through the
//! types here. Two layers cooperate (see `docs/plan/stdlib-support.md` §4.1):
//!
//! 1. an **intrinsic registry** keyed by `(BuiltinReceiver, method-name)` for
//!    type-specific members (`Array.append`, `String.uppercased`, …);
//! 2. an **algorithm layer** for `Sequence`/`Collection` methods written once
//!    against an iterator adapter and shared by every builtin sequence.
//!
//! Intrinsics never see the whole [`crate::Interpreter`]. They receive a narrow
//! capability handle, [`StdContext`], that exposes only what a builtin needs:
//! calling a closure, throwing, and writing output. This keeps `tswift-std`
//! decoupled and unit-testable against a mock context.

use std::io::Write;

use crate::value::SwiftValue;
use crate::EvalError;

/// A failure raised by a standard-library intrinsic.
///
/// Mirrors the interpreter's control-flow channel, but exposes only the two
/// outcomes a builtin can produce: a thrown Swift error or a genuine
/// interpreter error. Loop/`return` control flow never crosses the seam.
#[derive(Debug, Clone)]
pub enum StdError {
    /// A thrown Swift error value (Swift's `throw`).
    Throw(SwiftValue),
    /// A genuine interpreter error (trap, type mismatch, …).
    Error(EvalError),
}

impl From<EvalError> for StdError {
    fn from(e: EvalError) -> Self {
        StdError::Error(e)
    }
}

/// The result of an intrinsic: a produced value or a [`StdError`].
pub type StdResult = Result<SwiftValue, StdError>;

/// The narrow capability handle handed to every standard-library intrinsic.
///
/// Defined in core and implemented for [`crate::Interpreter`]; widen only as a
/// concrete slice needs it.
pub trait StdContext {
    /// Call the closure with table id `id`, returning its result.
    fn call_closure(&mut self, id: usize, args: Vec<SwiftValue>) -> StdResult;

    /// Evaluate the body of closure `id` as a result-builder block, returning an
    /// array of each top-level statement's value. This is the SwiftUI
    /// `@ViewBuilder` shim: `VStack { Text(…); Text(…) }` yields both children.
    /// The default treats the closure as producing a single value (its result).
    fn eval_block_values(&mut self, id: usize) -> StdResult {
        let value = self.call_closure(id, Vec::new())?;
        Ok(SwiftValue::Array(std::rc::Rc::new(vec![value])))
    }
    /// Like [`StdContext::eval_block_values`] but binds `args` to the closure's
    /// parameters first — the `@ViewBuilder` shim for a content closure that
    /// takes an argument yet may emit several sibling views (`ForEach`'s
    /// per-element body). The default applies the closure to `args` and wraps
    /// the single result.
    fn eval_block_values_with_args(&mut self, id: usize, args: Vec<SwiftValue>) -> StdResult {
        let value = self.call_closure(id, args)?;
        Ok(SwiftValue::Array(std::rc::Rc::new(vec![value])))
    }
    /// Read `name` from a struct `value` — a stored field or a computed getter
    /// (e.g. a `View`'s `body`). Lets an intrinsic drive member evaluation; the
    /// SwiftUI render host uses it to expand a composed sub-`View` into its
    /// `body`. The default reports an unknown member; the interpreter overrides.
    fn get_member(&mut self, _value: &SwiftValue, name: &str) -> StdResult {
        Err(StdError::Error(EvalError::Type(format!(
            "unknown member `{name}`"
        ))))
    }
    /// Inject environment `objects` into `view`'s property-wrapper fields of
    /// type `wrapper_type` (SwiftUI's `@EnvironmentObject` injection): each such
    /// field whose declared type matches an object's type (or the sole object)
    /// has the wrapper's stored slot set to that object. Returns the updated
    /// view. The default leaves the view unchanged; the interpreter overrides.
    fn inject_environment_objects(
        &mut self,
        view: &SwiftValue,
        _wrapper_type: &str,
        _objects: &[SwiftValue],
    ) -> StdResult {
        Ok(view.clone())
    }

    /// The program output sink (`print` and friends write here).
    fn out(&mut self) -> &mut dyn Write;
    /// Build a thrown-error outcome from a Swift error value.
    fn throw(&self, error: SwiftValue) -> StdError {
        StdError::Throw(error)
    }

    /// Render a value as `print`/string-interpolation would, honouring a
    /// user-defined `CustomStringConvertible.description` when present. The
    /// default ignores `description` and uses the plain value rendering; the
    /// interpreter overrides it.
    fn display(&mut self, value: &SwiftValue) -> String {
        value.to_string()
    }

    /// Whether `a < b` under `Comparable`. The default handles only the scalar
    /// values; the interpreter overrides it to also consult a type's static
    /// `<` operator. `None` means the values are not comparable.
    fn value_less_than(&mut self, a: &SwiftValue, b: &SwiftValue) -> Option<bool> {
        scalar_less_than(a, b)
    }

    /// Draw the next 64-bit value from the builtin RNG (`Bool.random()`, …).
    /// The default is a fixed value; the interpreter overrides it with a
    /// seeded, advancing generator.
    fn random_u64(&mut self) -> u64 {
        0
    }

    /// Seconds since the Unix epoch (1970-01-01 UTC). The default is fixed so
    /// direct builtin tests stay deterministic; the interpreter overrides it
    /// with wall-clock time on native targets.
    fn now_unix_seconds(&mut self) -> f64 {
        0.0
    }

    /// Perform an HTTP request through the embedding's configured transport
    /// (the seam behind `URLSession`). The default reports that no transport
    /// is available; the interpreter overrides it to delegate to the transport
    /// installed with [`crate::Interpreter::set_http_transport`].
    fn perform_http(
        &mut self,
        _req: &crate::http::HttpRequest,
    ) -> Result<crate::http::HttpResponse, crate::http::HttpError> {
        Err(crate::http::HttpError::Unavailable)
    }
}

/// The natural `<` over comparable scalar values (the default `Comparable`).
pub fn scalar_less_than(a: &SwiftValue, b: &SwiftValue) -> Option<bool> {
    use std::cmp::Ordering;
    let ord = match (a, b) {
        (SwiftValue::Int(x), SwiftValue::Int(y)) => x.raw.cmp(&y.raw),
        (SwiftValue::Double(x), SwiftValue::Double(y)) => x.partial_cmp(y)?,
        (SwiftValue::Int(x), SwiftValue::Double(y)) => (x.raw as f64).partial_cmp(y)?,
        (SwiftValue::Double(x), SwiftValue::Int(y)) => x.partial_cmp(&(y.raw as f64))?,
        (SwiftValue::Str(x), SwiftValue::Str(y)) => x.cmp(y),
        (SwiftValue::Bool(x), SwiftValue::Bool(y)) => x.cmp(y),
        _ => return None,
    };
    Some(ord == Ordering::Less)
}

/// The builtin receiver an intrinsic is registered against.
///
/// Grown one slice at a time; new container/scalar kinds are added as their
/// stdlib slice lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinReceiver {
    Array,
    Dictionary,
    Set,
    String,
    Character,
    Int,
    Double,
    Bool,
    Optional,
    Range,
    Data,
    UUID,
    IndexPath,
    IndexSet,
    URL,
    URLComponents,
    URLQueryItem,
    URLRequest,
    URLResponse,
    HTTPURLResponse,
    URLError,
    URLSession,
    URLSessionConfiguration,
    Date,
    DateComponents,
    Calendar,
    DateFormatter,
    ISO8601DateFormatter,
    Decimal,
    NumberFormatter,
    Measurement,
    /// A `Substring` value (represented identically to `String` at runtime;
    /// registered separately so `Substring.*` keys appear in the registry).
    Substring,
    /// A `ArraySlice` view: a window `[start, end)` into a base `Array`.
    /// Indices are base-relative, matching Swift semantics.
    ArraySlice,
    /// `ContiguousArray` — semantically an `Array` in this runtime;
    /// registered separately so `ContiguousArray.*` keys appear in the registry.
    ContiguousArray,
    /// `ClosedRange` — inclusive `a...b` range; represented as
    /// `SwiftValue::Range { inclusive: true }` at runtime.
    ClosedRange,
    /// `ReversedCollection` — lazy reversed view over a base collection;
    /// represented as a `Struct { type_name: "ReversedCollection" }`.
    ReversedCollection,
    /// `CollectionOfOne` — a single-element collection;
    /// represented as a `Struct { type_name: "CollectionOfOne" }`.
    CollectionOfOne,
    /// `EmptyCollection` — an always-empty typed collection;
    /// represented as a `Struct { type_name: "EmptyCollection" }`.
    EmptyCollection,
    /// `Date.FormatStyle` — a format-style builder produced by `.dateTime`;
    /// represented as a `Struct { type_name: "Date.FormatStyle" }`.
    DateFormatStyle,
}

impl BuiltinReceiver {
    /// The Swift type name this receiver corresponds to.
    pub fn type_name(self) -> &'static str {
        match self {
            BuiltinReceiver::Array => "Array",
            BuiltinReceiver::Dictionary => "Dictionary",
            BuiltinReceiver::Set => "Set",
            BuiltinReceiver::String => "String",
            BuiltinReceiver::Character => "Character",
            BuiltinReceiver::Int => "Int",
            BuiltinReceiver::Double => "Double",
            BuiltinReceiver::Bool => "Bool",
            BuiltinReceiver::Optional => "Optional",
            BuiltinReceiver::Range => "Range",
            BuiltinReceiver::Data => "Data",
            BuiltinReceiver::UUID => "UUID",
            BuiltinReceiver::IndexPath => "IndexPath",
            BuiltinReceiver::IndexSet => "IndexSet",
            BuiltinReceiver::URL => "URL",
            BuiltinReceiver::URLComponents => "URLComponents",
            BuiltinReceiver::URLQueryItem => "URLQueryItem",
            BuiltinReceiver::URLRequest => "URLRequest",
            BuiltinReceiver::URLResponse => "URLResponse",
            BuiltinReceiver::HTTPURLResponse => "HTTPURLResponse",
            BuiltinReceiver::URLError => "URLError",
            BuiltinReceiver::URLSession => "URLSession",
            BuiltinReceiver::URLSessionConfiguration => "URLSessionConfiguration",
            BuiltinReceiver::Date => "Date",
            BuiltinReceiver::DateComponents => "DateComponents",
            BuiltinReceiver::Calendar => "Calendar",
            BuiltinReceiver::DateFormatter => "DateFormatter",
            BuiltinReceiver::ISO8601DateFormatter => "ISO8601DateFormatter",
            BuiltinReceiver::Decimal => "Decimal",
            BuiltinReceiver::NumberFormatter => "NumberFormatter",
            BuiltinReceiver::Measurement => "Measurement",
            BuiltinReceiver::Substring => "Substring",
            BuiltinReceiver::ArraySlice => "ArraySlice",
            BuiltinReceiver::ContiguousArray => "ContiguousArray",
            BuiltinReceiver::ClosedRange => "ClosedRange",
            BuiltinReceiver::ReversedCollection => "ReversedCollection",
            BuiltinReceiver::CollectionOfOne => "CollectionOfOne",
            BuiltinReceiver::EmptyCollection => "EmptyCollection",
            BuiltinReceiver::DateFormatStyle => "Date.FormatStyle",
        }
    }

    /// The receiver for a builtin Swift type name (`"Bool"` → `Bool`, …).
    pub fn from_type_name(name: &str) -> Option<BuiltinReceiver> {
        Some(match name {
            "Array" => BuiltinReceiver::Array,
            "Dictionary" => BuiltinReceiver::Dictionary,
            "Set" => BuiltinReceiver::Set,
            "String" => BuiltinReceiver::String,
            "Character" => BuiltinReceiver::Character,
            "Int" => BuiltinReceiver::Int,
            "Double" => BuiltinReceiver::Double,
            "Bool" => BuiltinReceiver::Bool,
            "Optional" => BuiltinReceiver::Optional,
            "Range" => BuiltinReceiver::Range,
            "Data" => BuiltinReceiver::Data,
            "UUID" => BuiltinReceiver::UUID,
            "IndexPath" => BuiltinReceiver::IndexPath,
            "IndexSet" => BuiltinReceiver::IndexSet,
            "URL" => BuiltinReceiver::URL,
            "URLComponents" => BuiltinReceiver::URLComponents,
            "URLQueryItem" => BuiltinReceiver::URLQueryItem,
            "URLRequest" => BuiltinReceiver::URLRequest,
            "URLResponse" => BuiltinReceiver::URLResponse,
            "HTTPURLResponse" => BuiltinReceiver::HTTPURLResponse,
            "URLError" => BuiltinReceiver::URLError,
            "URLSession" => BuiltinReceiver::URLSession,
            "URLSessionConfiguration" => BuiltinReceiver::URLSessionConfiguration,
            "Date" => BuiltinReceiver::Date,
            "DateComponents" => BuiltinReceiver::DateComponents,
            "Calendar" => BuiltinReceiver::Calendar,
            "DateFormatter" => BuiltinReceiver::DateFormatter,
            "ISO8601DateFormatter" => BuiltinReceiver::ISO8601DateFormatter,
            "Decimal" => BuiltinReceiver::Decimal,
            "NumberFormatter" => BuiltinReceiver::NumberFormatter,
            "Measurement" => BuiltinReceiver::Measurement,
            "Substring" => BuiltinReceiver::Substring,
            "ArraySlice" => BuiltinReceiver::ArraySlice,
            "ContiguousArray" => BuiltinReceiver::ContiguousArray,
            "ClosedRange" => BuiltinReceiver::ClosedRange,
            "ReversedCollection" => BuiltinReceiver::ReversedCollection,
            "CollectionOfOne" => BuiltinReceiver::CollectionOfOne,
            "EmptyCollection" => BuiltinReceiver::EmptyCollection,
            "Date.FormatStyle" => BuiltinReceiver::DateFormatStyle,
            _ => return None,
        })
    }

    /// The receiver classification for a runtime value, if it is a builtin one.
    pub fn of(value: &SwiftValue) -> Option<BuiltinReceiver> {
        Some(match value {
            SwiftValue::Array(_) => BuiltinReceiver::Array,
            SwiftValue::Dict(_) => BuiltinReceiver::Dictionary,
            SwiftValue::Set(_) => BuiltinReceiver::Set,
            SwiftValue::Str(_) => BuiltinReceiver::String,
            SwiftValue::Substring { .. } => BuiltinReceiver::Substring,
            SwiftValue::ArraySlice { .. } => BuiltinReceiver::ArraySlice,
            SwiftValue::Int(_) => BuiltinReceiver::Int,
            SwiftValue::Double(_) => BuiltinReceiver::Double,
            SwiftValue::Bool(_) => BuiltinReceiver::Bool,
            SwiftValue::Range {
                inclusive: true, ..
            } => BuiltinReceiver::ClosedRange,
            SwiftValue::Range { .. } => BuiltinReceiver::Range,
            SwiftValue::Nil => BuiltinReceiver::Optional,
            SwiftValue::Struct(obj) if obj.type_name == "Data" => BuiltinReceiver::Data,
            SwiftValue::Struct(obj) if obj.type_name == "UUID" => BuiltinReceiver::UUID,
            SwiftValue::Struct(obj) if obj.type_name == "IndexPath" => BuiltinReceiver::IndexPath,
            SwiftValue::Struct(obj) if obj.type_name == "IndexSet" => BuiltinReceiver::IndexSet,
            SwiftValue::Struct(obj) if obj.type_name == "URL" => BuiltinReceiver::URL,
            SwiftValue::Struct(obj) if obj.type_name == "URLComponents" => {
                BuiltinReceiver::URLComponents
            }
            SwiftValue::Struct(obj) if obj.type_name == "URLQueryItem" => {
                BuiltinReceiver::URLQueryItem
            }
            SwiftValue::Struct(obj) if obj.type_name == "URLRequest" => BuiltinReceiver::URLRequest,
            SwiftValue::Struct(obj) if obj.type_name == "URLResponse" => {
                BuiltinReceiver::URLResponse
            }
            SwiftValue::Struct(obj) if obj.type_name == "HTTPURLResponse" => {
                BuiltinReceiver::HTTPURLResponse
            }
            SwiftValue::Struct(obj) if obj.type_name == "URLError" => BuiltinReceiver::URLError,
            SwiftValue::Struct(obj) if obj.type_name == "URLSession" => BuiltinReceiver::URLSession,
            SwiftValue::Struct(obj) if obj.type_name == "URLSessionConfiguration" => {
                BuiltinReceiver::URLSessionConfiguration
            }
            SwiftValue::Struct(obj) if obj.type_name == "Date" => BuiltinReceiver::Date,
            SwiftValue::Struct(obj) if obj.type_name == "DateComponents" => {
                BuiltinReceiver::DateComponents
            }
            SwiftValue::Struct(obj) if obj.type_name == "Calendar" => BuiltinReceiver::Calendar,
            SwiftValue::Struct(obj) if obj.type_name == "DateFormatter" => {
                BuiltinReceiver::DateFormatter
            }
            SwiftValue::Struct(obj) if obj.type_name == "ISO8601DateFormatter" => {
                BuiltinReceiver::ISO8601DateFormatter
            }
            SwiftValue::Struct(obj) if obj.type_name == "Decimal" => BuiltinReceiver::Decimal,
            SwiftValue::Struct(obj) if obj.type_name == "NumberFormatter" => {
                BuiltinReceiver::NumberFormatter
            }
            SwiftValue::Struct(obj) if obj.type_name == "Measurement" => {
                BuiltinReceiver::Measurement
            }
            SwiftValue::Struct(obj) if obj.type_name == "ReversedCollection" => {
                BuiltinReceiver::ReversedCollection
            }
            SwiftValue::Struct(obj) if obj.type_name == "CollectionOfOne" => {
                BuiltinReceiver::CollectionOfOne
            }
            SwiftValue::Struct(obj) if obj.type_name == "EmptyCollection" => {
                BuiltinReceiver::EmptyCollection
            }
            SwiftValue::Struct(obj) if obj.type_name == "Date.FormatStyle" => {
                BuiltinReceiver::DateFormatStyle
            }
            _ => return None,
        })
    }
}

/// The outcome of an intrinsic call: the call's result value plus the receiver
/// after the call.
///
/// Mutating intrinsics take the receiver by value and return the updated
/// receiver in `receiver`; the dispatcher writes it back to the caller's
/// storage. Non-mutating intrinsics echo the receiver back unchanged and the
/// dispatcher ignores it. By-value (rather than `&mut`) avoids aliasing the
/// `&mut Interpreter` that backs [`StdContext`]; copy-on-write comes for free
/// via `Rc::make_mut`.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub result: SwiftValue,
    pub receiver: SwiftValue,
}

/// A method intrinsic: receives the context, the receiver by value, and the
/// already-evaluated positional argument values; returns its [`Outcome`].
pub type IntrinsicFn =
    fn(&mut dyn StdContext, SwiftValue, Vec<SwiftValue>) -> Result<Outcome, StdError>;

/// One evaluated free-function or label-aware method argument: its (optional)
/// label and value.
///
/// Free functions and selected overloaded methods see labels because some APIs
/// are label-overloaded (`stride(from:to:by:)`, `Array.append(contentsOf:)`).
#[derive(Debug, Clone)]
pub struct Arg {
    pub label: Option<String>,
    pub value: SwiftValue,
}

impl Arg {
    /// A positional (unlabeled) argument.
    pub fn positional(value: SwiftValue) -> Arg {
        Arg { label: None, value }
    }
}

/// A free-function intrinsic (`print`, `min`, `max`, …).
pub type FreeFn = fn(&mut dyn StdContext, Vec<Arg>) -> StdResult;

/// A label-aware method intrinsic for builtin receivers whose overloads cannot
/// be selected from value shape alone. Returning `None` lets the normal
/// positional intrinsic registry handle the call.
pub type LabeledIntrinsicFn =
    fn(&mut dyn StdContext, SwiftValue, Vec<Arg>) -> Result<Option<Outcome>, StdError>;

/// A generic method intrinsic dispatched on *any* `SwiftValue::Struct` receiver
/// by method name, tried as a fallback after user-declared methods and builtin
/// receivers fail to match. Receives the context, the receiver by value, and
/// the evaluated, *labeled* call arguments; returns the call's result.
///
/// This is the seam SwiftUI view modifiers (`.font`, `.frame`, …) register
/// through: they take a view value, append to its `_modifiers` field
/// copy-on-write, and return the new view — all without `tswift-core` knowing
/// anything SwiftUI-specific.
pub type StructMethodFn = fn(&mut dyn StdContext, SwiftValue, Vec<Arg>) -> StdResult;

/// A static (type-level) method intrinsic on a builtin type (`Bool.random()`,
/// …). Receives the context and the evaluated, labeled call arguments.
pub type StaticFn = fn(&mut dyn StdContext, Vec<Arg>) -> StdResult;

/// A computed-property intrinsic on a builtin receiver (`Double.isNaN`,
/// `Int.magnitude`, …). Pure: no closures, no mutation, no output.
pub type PropertyFn = fn(SwiftValue) -> StdResult;

/// A built-in computed-property **setter** for a registered builtin type.
///
/// Arguments:
/// - `recv`: the current struct value (URLComponents, etc.).
/// - `new_value`: the right-hand side being assigned.
///
/// Returns the mutated struct on success, or a [`StdError`] (including a
/// `Trap` for invalid input such as an illegally percent-encoded string).
pub type PropertySetterFn =
    fn(recv: SwiftValue, new_value: SwiftValue) -> Result<SwiftValue, StdError>;

/// A computed-property intrinsic that needs the standard-library context
/// (`Date.timeIntervalSinceNow`, for example, needs the current clock).
pub type ContextualPropertyFn = fn(&mut dyn StdContext, SwiftValue) -> StdResult;

/// A `Sequence`/`Collection` algorithm written once against the materialized
/// elements of any builtin sequence receiver (`map`, `filter`, `sorted`, …).
///
/// The dispatcher expands the receiver into its elements via the sequence
/// adapter and passes the (labeled) call arguments through, so closure-taking
/// algorithms can call back into [`StdContext`].
pub type AlgoFn = fn(&mut dyn StdContext, Vec<SwiftValue>, Vec<Arg>) -> StdResult;

/// One registered method intrinsic plus whether it mutates its receiver.
#[derive(Clone, Copy)]
pub struct MethodEntry {
    /// When set, the dispatcher resolves the receiver's lvalue and writes the
    /// returned receiver back after the call.
    pub mutating: bool,
    pub func: IntrinsicFn,
}

/// A registered label-aware method intrinsic plus whether it mutates its
/// receiver. This keeps overload policy in `tswift-std` without forcing every
/// simple intrinsic to traffic in labels.
#[derive(Clone, Copy)]
pub struct LabeledMethodEntry {
    /// When set, the dispatcher resolves the receiver's lvalue and writes the
    /// returned receiver back after the call.
    pub mutating: bool,
    pub func: LabeledIntrinsicFn,
}

/// Resolve an integer collection range into validated `start..end` bounds.
///
/// Both open and closed two-sided ranges reject negative lower bounds, inverted
/// raw bounds (`3...2`), and upper bounds past `len`. One-sided ranges should be
/// normalized into a concrete [`SwiftValue::Range`] before calling this helper.
/// Eagerly materialize a builtin sequence value into its element values.
///
/// This is shared by the interpreter's generic algorithm dispatcher and stdlib
/// overloads that need to accept `Sequence`-shaped arguments such as
/// `contentsOf:`.
pub fn materialize_builtin_sequence(value: &SwiftValue) -> Option<Vec<SwiftValue>> {
    match value {
        SwiftValue::Array(items) => Some(items.as_ref().clone()),
        SwiftValue::ArraySlice { base, start, end } => Some(base[*start..*end].to_vec()),
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
        SwiftValue::Substring { base, start, end } => Some(
            crate::graphemes(base)[*start..*end]
                .iter()
                .map(|g| SwiftValue::Str(g.to_string()))
                .collect(),
        ),
        SwiftValue::Dict(pairs) => Some(
            pairs
                .iter()
                .map(|(k, v)| {
                    SwiftValue::tuple_labeled(
                        vec![k.clone(), v.clone()],
                        vec![Some("key".to_string()), Some("value".to_string())],
                    )
                })
                .collect(),
        ),
        SwiftValue::Set(items) => Some(items.as_ref().clone()),
        // `IndexPath` — iterate as Int elements (stored in `_indexes`).
        SwiftValue::Struct(obj) if obj.type_name == "IndexPath" => {
            if let Some(SwiftValue::Array(items)) = obj.get("_indexes") {
                Some(items.as_ref().clone())
            } else {
                None
            }
        }
        // `IndexSet` — iterate as ascending Int elements (stored in `_values`).
        SwiftValue::Struct(obj) if obj.type_name == "IndexSet" => {
            if let Some(SwiftValue::Array(items)) = obj.get("_values") {
                Some(items.as_ref().clone())
            } else {
                None
            }
        }
        // `Data` — iterate as UInt8 elements (stored in `_bytes`).
        SwiftValue::Struct(obj) if obj.type_name == "Data" => {
            if let Some(SwiftValue::Array(items)) = obj.get("_bytes") {
                Some(items.as_ref().clone())
            } else {
                None
            }
        }
        // ReversedCollection — iterate the base in reverse order.
        SwiftValue::Struct(obj) if obj.type_name == "ReversedCollection" => {
            if let Some(SwiftValue::Array(items)) = obj.get("_base") {
                let mut v = items.as_ref().clone();
                v.reverse();
                Some(v)
            } else {
                None
            }
        }
        // CollectionOfOne — single element.
        SwiftValue::Struct(obj) if obj.type_name == "CollectionOfOne" => {
            obj.get("_element").map(|e| vec![e.clone()])
        }
        // EmptyCollection — no elements.
        SwiftValue::Struct(obj) if obj.type_name == "EmptyCollection" => Some(vec![]),
        _ => None,
    }
}

pub fn collection_range_bounds(
    range: &SwiftValue,
    len: usize,
    who: &str,
) -> Result<(usize, usize), EvalError> {
    let SwiftValue::Range { lo, hi, inclusive } = range else {
        return Err(EvalError::Type(format!("{who} expects a range")));
    };
    if *lo < 0 || *hi < *lo {
        return Err(EvalError::Trap(format!(
            "{who} invalid range {lo}..{}{hi} for collection of length {len}",
            if *inclusive { "=" } else { "<" }
        )));
    }
    let end = if *inclusive {
        hi.checked_add(1)
            .ok_or_else(|| EvalError::Trap(format!("{who} range upperBound overflow")))?
    } else {
        *hi
    };
    if end > len as i128 {
        return Err(EvalError::Trap(format!(
            "{who} range {lo}..{}{hi} out of bounds for collection of length {len}",
            if *inclusive { "=" } else { "<" }
        )));
    }
    Ok((*lo as usize, end as usize))
}
