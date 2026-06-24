//! The runtime value model.
//!
//! Carries the scalar Swift values the evaluator manipulates. Integers track
//! their *width* (`Int8`..`UInt64`) so overflow-trapping (`+`/`-`/`*`) and
//! wrapping (`&+`/`&-`/`&*`) operators match Swift's semantics exactly.

use std::fmt;
use std::rc::Rc;

/// The bit width and signedness of an integer value, mirroring Swift's fixed
/// width integer family. `Int`/`UInt` map to the 64-bit arms on the platforms
/// quick-swift targets.
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
#[derive(Debug, Clone, PartialEq)]
pub enum SwiftValue {
    /// The empty tuple `()` — the result of a statement with no value.
    Void,
    Bool(bool),
    Int(IntValue),
    Double(f64),
    Str(String),
    /// A tuple `(a, b, ...)`.
    Tuple(Vec<SwiftValue>),
    /// An array `[a, b, ...]` (used today for variadic parameter packs).
    Array(Rc<Vec<SwiftValue>>),
    /// An integer range `lo..<hi` (exclusive) or `lo...hi` (inclusive).
    Range {
        lo: i128,
        hi: i128,
        inclusive: bool,
    },
    /// A first-class function value: an index into the interpreter's function
    /// table paired with its captured scope chain (opaque to this crate).
    Function(usize),
}

impl SwiftValue {
    /// Construct a platform `Int`.
    pub fn int(raw: i128) -> SwiftValue {
        SwiftValue::Int(IntValue::int(raw))
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
            SwiftValue::Tuple(_) => "tuple".into(),
            SwiftValue::Array(_) => "Array".into(),
            SwiftValue::Range { .. } => "Range".into(),
            SwiftValue::Function(_) => "function".into(),
        }
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
            SwiftValue::Tuple(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, ")")
            }
            SwiftValue::Array(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
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
        }
    }
}

/// Format a `Double` the way Swift's `print` does: integral values keep a
/// trailing `.0`, everything else uses the shortest round-tripping form.
pub fn format_double(d: f64) -> String {
    if d.is_infinite() {
        return if d > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if d.is_nan() {
        return "nan".into();
    }
    if d == d.trunc() && d.abs() < 1e16 {
        format!("{d:.1}")
    } else {
        format!("{d}")
    }
}
