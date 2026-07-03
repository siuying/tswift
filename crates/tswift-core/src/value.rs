//! The runtime value model.
//!
//! Carries the scalar Swift values the evaluator manipulates. Integers track
//! their *width* (`Int8`..`UInt64`) so overflow-trapping (`+`/`-`/`*`) and
//! wrapping (`&+`/`&-`/`&*`) operators match Swift's semantics exactly.

use std::cell::RefCell;
use std::fmt;
use std::rc::{Rc, Weak};

use crate::regex::Regex;

/// The bit width and signedness of an integer value, mirroring Swift's fixed
/// width integer family. `Int`/`UInt` map to the 64-bit arms on the platforms
/// tswift targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntWidth {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
}

impl IntWidth {
    /// `true` for the signed arms (`Int8`..`Int`).
    pub fn is_signed(self) -> bool {
        matches!(
            self,
            IntWidth::I8 | IntWidth::I16 | IntWidth::I32 | IntWidth::I64
        )
    }

    /// Number of bits in this width.
    pub fn bits(self) -> u32 {
        match self {
            IntWidth::I8 | IntWidth::U8 => 8,
            IntWidth::I16 | IntWidth::U16 => 16,
            IntWidth::I32 | IntWidth::U32 => 32,
            IntWidth::I64 | IntWidth::U64 => 64,
        }
    }

    /// Inclusive minimum representable value (as `i128`).
    pub fn min(self) -> i128 {
        if self.is_signed() {
            -(1i128 << (self.bits() - 1))
        } else {
            0
        }
    }

    /// Inclusive maximum representable value (as `i128`).
    pub fn max(self) -> i128 {
        if self.is_signed() {
            (1i128 << (self.bits() - 1)) - 1
        } else {
            (1i128 << self.bits()) - 1
        }
    }

    /// Swift's spelling of this width (e.g. `Int`, `UInt8`).
    pub fn type_name(self) -> &'static str {
        match self {
            IntWidth::I8 => "Int8",
            IntWidth::I16 => "Int16",
            IntWidth::I32 => "Int32",
            IntWidth::I64 => "Int",
            IntWidth::U8 => "UInt8",
            IntWidth::U16 => "UInt16",
            IntWidth::U32 => "UInt32",
            IntWidth::U64 => "UInt",
        }
    }

    /// Resolve a Swift type name to a width, if it names an integer type.
    pub fn from_type_name(name: &str) -> Option<IntWidth> {
        Some(match name {
            "Int" | "Int64" => IntWidth::I64,
            "Int8" => IntWidth::I8,
            "Int16" => IntWidth::I16,
            "Int32" => IntWidth::I32,
            "UInt" | "UInt64" => IntWidth::U64,
            "UInt8" => IntWidth::U8,
            "UInt16" => IntWidth::U16,
            "UInt32" => IntWidth::U32,
            _ => return None,
        })
    }
}

/// A width-tracked integer value. `raw` always lies within `width`'s range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntValue {
    pub raw: i128,
    pub width: IntWidth,
}

impl IntValue {
    /// A platform `Int` (signed 64-bit).
    pub fn int(raw: i128) -> IntValue {
        IntValue {
            raw,
            width: IntWidth::I64,
        }
    }

    pub fn new(raw: i128, width: IntWidth) -> IntValue {
        IntValue { raw, width }
    }

    /// `true` if `raw` fits within `width`.
    pub fn in_range(&self) -> bool {
        self.raw >= self.width.min() && self.raw <= self.width.max()
    }

    /// Reduce `raw` into `width` with two's-complement wraparound.
    pub fn wrapped(width: IntWidth, raw: i128) -> IntValue {
        let bits = width.bits();
        let modulo = 1i128 << bits;
        let mut m = raw % modulo;
        if m < 0 {
            m += modulo;
        }
        // `m` is now in [0, 2^bits). Re-interpret as signed if needed.
        let raw = if width.is_signed() && m > width.max() {
            m - modulo
        } else {
            m
        };
        IntValue { raw, width }
    }
}

/// A Swift runtime value.
#[derive(Debug, Clone)]
pub enum SwiftValue {
    /// The empty tuple `()` — the result of a statement with no value.
    Void,
    Bool(bool),
    Int(IntValue),
    Double(f64),
    Str(String),
    /// A Substring view: an immutable grapheme-cluster window `[start, end)` into
    /// a base `String`.  Indices on a Substring are **base-relative** — the same
    /// coordinate space as the parent String — so `s[i..<j].startIndex == i`.
    Substring {
        base: Rc<String>,
        start: usize,
        end: usize,
    },
    /// A tuple `(a, b, ...)`. The second vector holds an optional label per
    /// element (`(min: 1, max: 9)` → `[Some("min"), Some("max")]`); labels are
    /// type-level metadata that does not participate in equality.
    Tuple(Vec<SwiftValue>, Vec<Option<String>>),
    /// An array `[a, b, ...]` (used today for variadic parameter packs).
    Array(Rc<Vec<SwiftValue>>),
    /// An ArraySlice view: an element window `[start, end)` into a base `Array`.
    /// Indices are **base-relative** — the same coordinate space as the parent
    /// array — so `a[i..<j].startIndex == i`.
    ArraySlice {
        base: Rc<Vec<SwiftValue>>,
        start: usize,
        end: usize,
    },
    /// A dictionary `[k: v, ...]`. Stored as insertion-ordered key/value pairs
    /// (linear lookup) under an `Rc` for copy-on-write value semantics. Swift
    /// dictionaries are unordered, so callers must not rely on iteration order.
    Dict(Rc<Vec<(SwiftValue, SwiftValue)>>),
    /// A set. Stored as insertion-ordered unique elements under an `Rc` for
    /// copy-on-write value semantics; iteration order is not meaningful.
    Set(Rc<Vec<SwiftValue>>),
    /// An integer range `lo..<hi` (exclusive) or `lo...hi` (inclusive).
    Range {
        lo: i128,
        hi: i128,
        inclusive: bool,
    },
    /// A first-class function value: an index into the interpreter's function
    /// table paired with its captured scope chain (opaque to this crate).
    Function(usize),
    /// A value-semantics struct instance. The `Rc` enables copy-on-write: an
    /// assignment shares the `Rc`; a mutation calls [`Rc::make_mut`] to clone
    /// only when the instance is aliased.
    Struct(Rc<StructObj>),
    /// The absent optional, `nil`. tswift models `Optional` with this
    /// sentinel: a present optional is simply its wrapped value.
    Nil,
    /// An enum case value, with any associated values.
    Enum(Rc<EnumObj>),
    /// A reference-semantics class instance. ARC is the `Rc` strong count;
    /// shared mutation goes through the `RefCell`.
    Object(Rc<RefCell<ClassObj>>),
    /// A `weak` reference to a class instance (zeroes to `nil` on dealloc).
    Weak(Weak<RefCell<ClassObj>>),
    /// An `unowned` reference to a class instance: non-retaining like `weak`,
    /// but reading it after the referent deallocated is a runtime trap rather
    /// than `nil`.
    Unowned(Weak<RefCell<ClassObj>>),
    /// A compiled regular expression, produced by a `/.../`/`#/.../#` literal or
    /// `Regex(_:)`. Shared under an `Rc` (the compiled program is immutable).
    Regex(Rc<Regex>),
    /// A closure value: an index into the interpreter's closure table.
    Closure(usize),
    /// A structured-concurrency task handle: an index into the interpreter's
    /// task table. Produced by `async let`, `Task { }`, and `group.addTask`;
    /// `await`-ing it drives the task to completion and yields its result.
    Task(usize),
    /// A `withTaskGroup` child-task group: an index into the interpreter's
    /// group table. `addTask` appends children; `for await` drains their
    /// results.
    TaskGroup(usize),
    /// A `withCheckedContinuation`/`withUnsafeContinuation` continuation: an
    /// index into the interpreter's continuation table. `resume(...)` fills the
    /// slot; the enclosing `with*Continuation` reads it back as its result.
    Continuation(usize),
    /// An `AsyncStream.Continuation`: an index into the interpreter's stream
    /// table. `yield(_:)` appends to the stream's buffer; `finish()` closes it.
    StreamContinuation(usize),
    /// The reader half of an `AsyncStream` produced by `makeStream(of:)`: an
    /// index into the stream table. Iterating it (`for await`) drains the buffer
    /// its paired `StreamContinuation` filled.
    AsyncStreamHandle(usize),
    /// A metatype value, e.g. `Int.self` or `type(of: x)`. Carries the spelled
    /// type name; printing it renders the bare type name like Swift.
    Metatype(String),
    /// A global/local variable with accessor bodies (computed `get`/`set` or
    /// `willSet`/`didSet` observers): an index into the interpreter's
    /// accessor-variable table. The environment binding holds this marker;
    /// reads and writes route through the accessors and never expose it.
    AccessorVar(usize),
}

/// The storage of a class instance.
#[derive(Debug)]
pub struct ClassObj {
    pub class_name: String,
    pub fields: Vec<(String, SwiftValue)>,
}

impl ClassObj {
    pub fn get(&self, name: &str) -> Option<&SwiftValue> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    pub fn set(&mut self, name: &str, value: SwiftValue) {
        if let Some(slot) = self.fields.iter_mut().find(|(n, _)| n == name) {
            slot.1 = value;
        } else {
            self.fields.push((name.to_string(), value));
        }
    }
}

/// The storage of an enum case value.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumObj {
    pub type_name: String,
    pub case: String,
    /// Associated values (empty for plain cases), with optional labels.
    pub payload: Vec<SwiftValue>,
}

/// The storage of a struct instance: its type name and ordered fields.
#[derive(Debug, Clone, PartialEq)]
pub struct StructObj {
    pub type_name: String,
    pub fields: Vec<(String, SwiftValue)>,
}

impl StructObj {
    /// Read a stored field by name.
    pub fn get(&self, name: &str) -> Option<&SwiftValue> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// Set a stored field, inserting it if absent.
    pub fn set(&mut self, name: &str, value: SwiftValue) {
        if let Some(slot) = self.fields.iter_mut().find(|(n, _)| n == name) {
            slot.1 = value;
        } else {
            self.fields.push((name.to_string(), value));
        }
    }
}

impl SwiftValue {
    /// Construct a platform `Int`.
    pub fn int(raw: i128) -> SwiftValue {
        SwiftValue::Int(IntValue::int(raw))
    }

    /// Construct an unlabeled tuple.
    pub fn tuple(items: Vec<SwiftValue>) -> SwiftValue {
        let labels = vec![None; items.len()];
        SwiftValue::Tuple(items, labels)
    }

    /// Construct a tuple with an explicit label per element.
    pub fn tuple_labeled(items: Vec<SwiftValue>, labels: Vec<Option<String>>) -> SwiftValue {
        debug_assert_eq!(items.len(), labels.len());
        SwiftValue::Tuple(items, labels)
    }

    /// The index of a tuple element by label, if the tuple carries that label.
    pub fn tuple_label_index(labels: &[Option<String>], name: &str) -> Option<usize> {
        labels.iter().position(|l| l.as_deref() == Some(name))
    }

    /// Interpret the value as a boolean (only `Bool` qualifies).
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SwiftValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The Swift type name used in error messages.
    pub fn type_name(&self) -> String {
        match self {
            SwiftValue::Void => "()".into(),
            SwiftValue::Bool(_) => "Bool".into(),
            SwiftValue::Int(i) => i.width.type_name().into(),
            SwiftValue::Double(_) => "Double".into(),
            SwiftValue::Str(_) => "String".into(),
            SwiftValue::Substring { .. } => "Substring".into(),
            SwiftValue::Tuple(..) => "tuple".into(),
            SwiftValue::Array(_) => "Array".into(),
            SwiftValue::ArraySlice { .. } => "ArraySlice".into(),
            SwiftValue::Dict(_) => "Dictionary".into(),
            SwiftValue::Set(_) => "Set".into(),
            SwiftValue::Range { .. } => "Range".into(),
            SwiftValue::Function(_) => "function".into(),
            SwiftValue::Regex(_) => "Regex".into(),
            SwiftValue::Struct(s) => s.type_name.clone(),
            SwiftValue::Nil => "Optional".into(),
            SwiftValue::Enum(e) => e.type_name.clone(),
            SwiftValue::Object(o) => o.borrow().class_name.clone(),
            SwiftValue::Weak(_) => "Optional".into(),
            SwiftValue::Unowned(w) => w
                .upgrade()
                .map(|o| o.borrow().class_name.clone())
                .unwrap_or_else(|| "unowned".into()),
            SwiftValue::Closure(_) => "closure".into(),
            SwiftValue::Task(_) => "Task".into(),
            SwiftValue::TaskGroup(_) => "TaskGroup".into(),
            SwiftValue::Continuation(_) => "Continuation".into(),
            SwiftValue::StreamContinuation(_) => "AsyncStream.Continuation".into(),
            SwiftValue::AsyncStreamHandle(_) => "AsyncStream".into(),
            SwiftValue::Metatype(name) => format!("{name}.Type"),
            SwiftValue::AccessorVar(_) => "variable".into(),
        }
    }
}

impl PartialEq for SwiftValue {
    fn eq(&self, other: &Self) -> bool {
        use SwiftValue::*;
        match (self, other) {
            (Void, Void) | (Nil, Nil) => true,
            (Bool(a), Bool(b)) => a == b,
            (Int(a), Int(b)) => a == b,
            (Double(a), Double(b)) => a == b,
            (Str(a), Str(b)) => a == b,
            (
                Substring {
                    base: b1,
                    start: s1,
                    end: e1,
                },
                Substring {
                    base: b2,
                    start: s2,
                    end: e2,
                },
            ) => {
                let g1 = crate::graphemes(b1);
                let g2 = crate::graphemes(b2);
                g1[*s1..*e1].concat() == g2[*s2..*e2].concat()
            }
            (Str(a), Substring { base, start, end }) => {
                *a == crate::graphemes(base)[*start..*end].concat()
            }
            (Substring { base, start, end }, Str(a)) => {
                *a == crate::graphemes(base)[*start..*end].concat()
            }
            // Tuple labels are type metadata, not value: compare elements only.
            (Tuple(a, _), Tuple(b, _)) => a == b,
            (Array(a), Array(b)) => a == b,
            // ArraySlice equality: compare elements in the slice window.
            (
                ArraySlice {
                    base: b1,
                    start: s1,
                    end: e1,
                },
                ArraySlice {
                    base: b2,
                    start: s2,
                    end: e2,
                },
            ) => b1[*s1..*e1] == b2[*s2..*e2],
            // NOTE: ArraySlice vs Array is NOT equal (distinct types, strict separation).
            // Dictionaries are equal as unordered key/value sets.
            (Dict(a), Dict(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| b.iter().any(|(k2, v2)| k == k2 && v == v2))
            }
            // Sets are equal as unordered element collections.
            (Set(a), Set(b)) => a.len() == b.len() && a.iter().all(|x| b.contains(x)),
            (
                Range {
                    lo: l1,
                    hi: h1,
                    inclusive: i1,
                },
                Range {
                    lo: l2,
                    hi: h2,
                    inclusive: i2,
                },
            ) => l1 == l2 && h1 == h2 && i1 == i2,
            (Function(a), Function(b)) => a == b,
            (Regex(a), Regex(b)) => a == b,
            (Closure(a), Closure(b)) => a == b,
            (Struct(a), Struct(b)) => a == b,
            (Enum(a), Enum(b)) => a == b,
            // Class instances compare by identity (`===`).
            (Object(a), Object(b)) => Rc::ptr_eq(a, b),
            (Weak(a), Weak(b)) => a.ptr_eq(b),
            (Unowned(a), Unowned(b)) => a.ptr_eq(b),
            (Metatype(a), Metatype(b)) => a == b,
            _ => false,
        }
    }
}

/// Escape a string for display inside a Swift collection.
///
/// Rules (matches Swift's `debugDescription` / collection-element semantics):
/// - `\` → `\\`
/// - `"` → `\"`
/// - NUL  (U+00) → `\0`
/// - TAB  (U+09) → `\t`
/// - LF   (U+0A) → `\n`
/// - CR   (U+0D) → `\r`
/// - Other C0 control chars (U+01-U+08, U+0B-U+0C, U+0E-U+1F) and DEL (U+7F)
///   → `\u{XX}` with lowercase two-digit minimum hex.
/// All other chars are written as-is (including multibyte UTF-8).
fn escape_string_for_collection(s: &str, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    for ch in s.chars() {
        match ch {
            '\\' => write!(f, "\\\\")?,
            '"' => write!(f, "\\\"")?,
            '\0' => write!(f, "\\0")?,
            '\t' => write!(f, "\\t")?,
            '\n' => write!(f, "\\n")?,
            '\r' => write!(f, "\\r")?,
            c if (c as u32) <= 0x08
                || c as u32 == 0x0B
                || c as u32 == 0x0C
                || (0x0E..=0x1F).contains(&(c as u32))
                || c as u32 == 0x7F =>
            {
                write!(f, "\\u{{{:02X}}}", c as u32)?
            }
            c => write!(f, "{c}")?,
        }
    }
    Ok(())
}

/// Render a value as a *collection element* — strings/substrings get quoted
/// and their contents escaped (Swift `debugDescription` semantics); other
/// types use their normal `Display`.
fn fmt_element(v: &SwiftValue, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match v {
        SwiftValue::Str(s) => {
            write!(f, "\"")?;
            escape_string_for_collection(s, f)?;
            write!(f, "\"")
        }
        SwiftValue::Substring { base, start, end } => {
            let gs = crate::graphemes(base);
            let text = gs[*start..*end].concat();
            write!(f, "\"")?;
            escape_string_for_collection(&text, f)?;
            write!(f, "\"")
        }
        other => write!(f, "{other}"),
    }
}

impl fmt::Display for SwiftValue {
    /// Renders a value the way Swift's `print` would for these scalar cases.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SwiftValue::Void => write!(f, "()"),
            SwiftValue::Bool(b) => write!(f, "{b}"),
            SwiftValue::Int(i) => write!(f, "{}", i.raw),
            SwiftValue::Double(d) => write!(f, "{}", format_double(*d)),
            SwiftValue::Str(s) => write!(f, "{s}"),
            SwiftValue::Substring { base, start, end } => {
                let gs = crate::graphemes(base);
                write!(f, "{}", gs[*start..*end].concat())
            }
            SwiftValue::Tuple(items, labels) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    if let Some(Some(label)) = labels.get(i) {
                        write!(f, "{label}: ")?;
                    }
                    fmt_element(item, f)?;
                }
                write!(f, ")")
            }
            SwiftValue::Array(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    fmt_element(item, f)?;
                }
                write!(f, "]")
            }
            SwiftValue::ArraySlice { base, start, end } => {
                write!(f, "[")?;
                for (i, item) in base[*start..*end].iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    fmt_element(item, f)?;
                }
                write!(f, "]")
            }
            SwiftValue::Dict(pairs) => {
                if pairs.is_empty() {
                    return write!(f, "[:]");
                }
                write!(f, "[")?;
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    fmt_element(k, f)?;
                    write!(f, ": ")?;
                    fmt_element(v, f)?;
                }
                write!(f, "]")
            }
            SwiftValue::Set(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    fmt_element(item, f)?;
                }
                write!(f, "]")
            }
            SwiftValue::Range { lo, hi, inclusive } => {
                if *inclusive {
                    write!(f, "{lo}...{hi}")
                } else {
                    write!(f, "{lo}..<{hi}")
                }
            }
            SwiftValue::Function(_) => write!(f, "(Function)"),
            SwiftValue::Regex(r) => write!(f, "{}", r.pattern()),
            SwiftValue::Struct(s) => {
                write!(f, "{}(", s.type_name)?;
                for (i, (name, value)) in s.fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{name}: {value}")?;
                }
                write!(f, ")")
            }
            SwiftValue::Nil => write!(f, "nil"),
            SwiftValue::Object(o) => {
                let obj = o.borrow();
                write!(f, "{}(", obj.class_name)?;
                for (i, (name, value)) in obj.fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{name}: {value}")?;
                }
                write!(f, ")")
            }
            SwiftValue::Weak(w) => match w.upgrade() {
                Some(o) => write!(f, "{}", SwiftValue::Object(o)),
                None => write!(f, "nil"),
            },
            SwiftValue::Unowned(w) => match w.upgrade() {
                Some(o) => write!(f, "{}", SwiftValue::Object(o)),
                None => write!(f, "<deallocated>"),
            },
            SwiftValue::Closure(_) => write!(f, "(Function)"),
            SwiftValue::Task(_) => write!(f, "Task"),
            SwiftValue::TaskGroup(_) => write!(f, "TaskGroup"),
            SwiftValue::Continuation(_) => write!(f, "Continuation"),
            SwiftValue::StreamContinuation(_) => write!(f, "AsyncStream.Continuation"),
            SwiftValue::AsyncStreamHandle(_) => write!(f, "AsyncStream"),
            SwiftValue::Metatype(name) => write!(f, "{name}"),
            SwiftValue::AccessorVar(_) => write!(f, "(Variable)"),
            SwiftValue::Enum(e) => {
                write!(f, "{}", e.case)?;
                if !e.payload.is_empty() {
                    write!(f, "(")?;
                    for (i, v) in e.payload.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{v}")?;
                    }
                    write!(f, ")")?;
                }
                Ok(())
            }
        }
    }
}

/// Format a `Double` the way Swift's `print` does.
///
/// Rules (derived empirically from Swift 6.x):
/// 1. NaN → `"nan"`, ±∞ → `"inf"`/`"-inf"`.
/// 2. Exact integers whose magnitude fits in 2^53 use `"N.0"` decimal form.
/// 3. Large integers (magnitude > 2^53, always IEEE integers) use Swift-style
///    scientific notation (`9.9e+15`).
/// 4. Non-integers use shortest-round-trip form:
///    - decimal when the base-10 exponent E is in `[-4, 15]`
///    - scientific (`1e-05`, `1.5e+300`) otherwise.
pub fn format_double(d: f64) -> String {
    fmt_double_impl(d, true)
}

/// Format a `Double` for JSON output: same as `format_double` but integers
/// are written without the `.0` suffix (`0` not `0.0`, `42` not `42.0`),
/// and `-0.0` is written as `-0`.
pub fn format_double_json(d: f64) -> String {
    fmt_double_impl(d, false)
}

/// Maximum f64 that can represent every integer exactly (2^53).
const MAX_EXACT_INT_F64: f64 = 9_007_199_254_740_992.0;

fn fmt_double_impl(d: f64, dot_zero: bool) -> String {
    if d.is_nan() {
        return "nan".into();
    }
    if d.is_infinite() {
        return if d > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if d == 0.0 {
        return if d.is_sign_negative() {
            if dot_zero {
                "-0.0".into()
            } else {
                "-0".into()
            }
        } else {
            if dot_zero {
                "0.0".into()
            } else {
                "0".into()
            }
        };
    }
    // Exact integers in [-2^53, 2^53]: decimal form, optionally with ".0".
    if d.fract() == 0.0 && d.abs() <= MAX_EXACT_INT_F64 {
        return if dot_zero {
            format!("{:.0}.0", d)
        } else {
            format!("{:.0}", d)
        };
    }
    // Get shortest round-trip scientific form from Rust's formatter.
    // Rust {:e} produces e.g. "9.9e15", "-1.5e-10" (no + on positive exp).
    let sci = format!("{:e}", d);
    let e_pos = sci.find('e').unwrap();
    let mantissa_full = &sci[..e_pos]; // may start with '-'
    let exp: i32 = sci[e_pos + 1..].parse().unwrap();
    let neg = d.is_sign_negative();
    let mantissa_abs: &str = if neg {
        &mantissa_full[1..]
    } else {
        mantissa_full
    };

    if d.fract() == 0.0 {
        // Large integer (|d| > 2^53): Swift always uses scientific.
        fmt_sci(neg, mantissa_abs, exp)
    } else if exp >= -4 && exp <= 15 {
        // Non-integer in the "decimal" exponent window.
        let digits: String = mantissa_abs
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        fmt_decimal(neg, &digits, exp)
    } else {
        // Non-integer outside the window: scientific.
        fmt_sci(neg, mantissa_abs, exp)
    }
}

/// Build Swift-style scientific notation: `[−]M.Ne+EE`.
fn fmt_sci(neg: bool, mantissa_abs: &str, exp: i32) -> String {
    let sign = if neg { "-" } else { "" };
    let exp_sign = if exp >= 0 { "+" } else { "-" };
    let exp_mag = exp.unsigned_abs();
    // Swift uses at least two exponent digits.
    let exp_str = if exp_mag < 10 {
        format!("0{exp_mag}")
    } else {
        format!("{exp_mag}")
    };
    format!("{sign}{mantissa_abs}e{exp_sign}{exp_str}")
}

/// Build decimal form from significant `digits` and base-10 exponent `exp`.
///
/// `digits` contains the raw digit string (no sign, no dot), and
/// `exp` is the exponent so that the number equals `0.digits... × 10^(exp+1)`
/// (i.e., exp is the position of the most-significant digit relative to units).
fn fmt_decimal(neg: bool, digits: &str, exp: i32) -> String {
    let sign = if neg { "-" } else { "" };
    // Number of digits that appear before the decimal point.
    let int_len = (exp + 1) as usize; // exp in [-4,15], so int_len in [-3,16]; clamp ≥0 below

    if exp < 0 {
        // Value is between 0 and 1: "0.000...{digits}"
        let leading = (-exp - 1) as usize; // zeros after the '.'
        format!("{}0.{}{}", sign, "0".repeat(leading), digits)
    } else {
        // int_len digits before the decimal point; the rest after.
        let n = digits.len();
        if int_len >= n {
            // All digits are in the integer part; pad with zeros if needed.
            let trailing = int_len - n;
            format!("{}{}{}", sign, digits, "0".repeat(trailing))
            // Note: caller (non-integer path) always has a fractional component
            // so this branch in practice only fires when rounding makes the
            // mantissa exact.  We do NOT append ".0" here (that's the integer
            // branch above).
        } else {
            let int_part = &digits[..int_len];
            let frac_part = &digits[int_len..];
            format!("{}{}.{}", sign, int_part, frac_part)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Copy-on-write: assigning shares the `Rc`; mutating one side clones it,
    /// leaving the other's storage uniquely owned and unchanged.
    #[test]
    fn struct_cow_uniqueness() {
        let a = SwiftValue::Struct(Rc::new(StructObj {
            type_name: "P".into(),
            fields: vec![("x".into(), SwiftValue::int(1))],
        }));
        let mut b = a.clone(); // assignment shares the Rc

        if let SwiftValue::Struct(ra) = &a {
            assert_eq!(Rc::strong_count(ra), 2, "assignment should share storage");
        }

        if let SwiftValue::Struct(rb) = &mut b {
            Rc::make_mut(rb).set("x", SwiftValue::int(99)); // CoW clone
        }

        match (&a, &b) {
            (SwiftValue::Struct(ra), SwiftValue::Struct(rb)) => {
                assert_eq!(
                    Rc::strong_count(ra),
                    1,
                    "original is uniquely owned after CoW"
                );
                assert_eq!(ra.get("x"), Some(&SwiftValue::int(1)));
                assert_eq!(rb.get("x"), Some(&SwiftValue::int(99)));
            }
            _ => unreachable!(),
        }
    }

    /// Helper: wrap a single value in an Array and Display it.
    fn element_display(v: &SwiftValue) -> String {
        format!("{}", SwiftValue::Array(std::rc::Rc::new(vec![v.clone()])))
    }

    #[test]
    fn string_escaping_in_collections() {
        // Double-quote inside string
        assert_eq!(
            element_display(&SwiftValue::Str(r#"a"b"#.into())),
            r#"["a\"b"]"#
        );
        // Backslash inside string
        assert_eq!(
            element_display(&SwiftValue::Str("a\\b".into())),
            r#"["a\\b"]"#
        );
        // Newline
        assert_eq!(
            element_display(&SwiftValue::Str("a\nb".into())),
            r#"["a\nb"]"#
        );
        // Tab
        assert_eq!(
            element_display(&SwiftValue::Str("a\tb".into())),
            r#"["a\tb"]"#
        );
        // Carriage return
        assert_eq!(
            element_display(&SwiftValue::Str("a\rb".into())),
            r#"["a\rb"]"#
        );
        // NUL
        assert_eq!(
            element_display(&SwiftValue::Str("a\0b".into())),
            r#"["a\0b"]"#
        );
        // Control char U+01
        assert_eq!(
            element_display(&SwiftValue::Str("a\x01b".into())),
            "[\"a\\u{01}b\"]"
        );
        // Control char U+0B (vertical tab) — uppercase hex
        assert_eq!(
            element_display(&SwiftValue::Str("a\x0Bb".into())),
            "[\"a\\u{0B}b\"]"
        );
        // DEL U+7F — uppercase hex
        assert_eq!(
            element_display(&SwiftValue::Str("a\x7Fb".into())),
            "[\"a\\u{7F}b\"]"
        );
        // Normal ASCII — no escaping
        assert_eq!(
            element_display(&SwiftValue::Str("hello".into())),
            r#"["hello"]"#
        );
        // Unicode (U+00E9 = 'é') — non-ASCII printable, no escaping needed
        assert_eq!(
            element_display(&SwiftValue::Str("caf\u{00E9}".into())),
            "[\"caf\u{00e9}\"]"
        );
        // Empty string
        assert_eq!(element_display(&SwiftValue::Str(String::new())), r#"[""]"#);
    }
}
