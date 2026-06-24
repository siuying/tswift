//! The runtime value model.
//!
//! This is the *spine* of `SwiftValue` — the full plan grows it with `Rc`-backed
//! structs/enums/classes (ARC), arrays/dicts (CoW), closures, and width-tracked
//! integers. The walking skeleton carries only the scalar cases needed to
//! evaluate an integer-literal call to `print`.

use std::fmt;

/// A Swift runtime value.
#[derive(Debug, Clone, PartialEq)]
pub enum SwiftValue {
    /// The empty tuple `()` — the result of a statement with no value.
    Void,
    Bool(bool),
    /// A platform `Int`. Width tracking (`I8..U64`) and overflow/wrap semantics
    /// arrive with the numeric milestone; the skeleton stores a bare `i64`.
    Int(i64),
    Double(f64),
    Str(String),
}

impl fmt::Display for SwiftValue {
    /// Renders a value the way Swift's `print` would for these scalar cases.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SwiftValue::Void => write!(f, "()"),
            SwiftValue::Bool(b) => write!(f, "{b}"),
            SwiftValue::Int(i) => write!(f, "{i}"),
            SwiftValue::Double(d) => write!(f, "{d}"),
            SwiftValue::Str(s) => write!(f, "{s}"),
        }
    }
}
