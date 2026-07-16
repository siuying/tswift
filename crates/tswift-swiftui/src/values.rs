//! Shared value/id helpers for view values: constructing view structs, reading
//! prelude tokens, numeric coercions, and the stable identity-key scheme
//! (`key_of`/`child_id`) that the serializer, diff, and session walkers share.

use std::rc::Rc;

use tswift_core::{EvalError, StdError, StructObj, SwiftValue};

use crate::{CHILDREN_FIELD, KEY_FIELD, MODIFIERS_FIELD};

/// The token string carried by a prelude token struct (`Color`/`Font`/
/// `FontWeight`), if `value` is one.
pub fn token_of(value: &SwiftValue) -> Option<(&str, &str)> {
    let SwiftValue::Struct(obj) = value else {
        return None;
    };
    if !matches!(
        obj.type_name.as_str(),
        "Color"
            | "Font"
            | "FontWeight"
            | "TextAlignment"
            | "TextCase"
            | "Axis"
            | "_ControlStyle"
            | "Alignment"
            | "HorizontalAlignment"
            | "VerticalAlignment"
            | "Edge"
            | "ContentMode"
            | "Visibility"
            | "BlendMode"
            | "ControlSize"
            | "SymbolRenderingMode"
            | "RedactionReasons"
            | "TruncationMode"
    ) {
        return None;
    }
    match obj.get("token") {
        Some(SwiftValue::Str(s)) => Some((obj.type_name.as_str(), s.as_str())),
        _ => None,
    }
}

/// Build a view value: a struct carrying `type_name` plus any constructor
/// fields, an empty ordered `_modifiers` list, and (for containers) `_children`.
///
/// Public so sibling render-host frameworks (e.g. Charts) can reuse the same
/// view-value shape without duplicating the `_modifiers` contract.
pub fn view_value(type_name: &str, mut fields: Vec<(String, SwiftValue)>) -> SwiftValue {
    fields.push((
        MODIFIERS_FIELD.into(),
        SwiftValue::Array(Rc::new(Vec::new())),
    ));
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: type_name.into(),
        fields,
    }))
}

/// Build a container view value with an ordered `_children` list.
///
/// Public so sibling render-host frameworks can build container nodes that
/// serialize through the shared UIIR path.
pub fn container_value(type_name: &str, children: Vec<SwiftValue>) -> SwiftValue {
    view_value(
        type_name,
        vec![(CHILDREN_FIELD.into(), SwiftValue::Array(Rc::new(children)))],
    )
}

/// Set (or append) a field on a mutable struct object.
pub(crate) fn set_or_push_field(obj: &mut StructObj, name: &str, value: SwiftValue) {
    if let Some(slot) = obj.fields.iter_mut().find(|(k, _)| k == name) {
        slot.1 = value;
    } else {
        obj.fields.push((name.into(), value));
    }
}

/// Read a Swift numeric value as `f64` (int widened, double as-is).
pub(crate) fn number_f64(value: &SwiftValue) -> Option<f64> {
    match value {
        SwiftValue::Int(i) => Some(i.raw as f64),
        SwiftValue::Double(d) => Some(*d),
        _ => None,
    }
}

/// Resolve `(lower, upper)` bounds from an `in:` range argument, falling back to
/// the given defaults when absent or not a range. v1 limitation: the runtime
/// represents only integer ranges, so a `Slider` range is written as `0...1`
/// (not `0.0...1.0`); the integer endpoints are widened to `f64` here.
pub(crate) fn range_bounds(range: Option<&SwiftValue>, def_lo: f64, def_hi: f64) -> (f64, f64) {
    match range {
        Some(SwiftValue::Range { lo, hi, .. }) => (*lo as f64, *hi as f64),
        _ => (def_lo, def_hi),
    }
}

/// Materialize a ForEach data argument into an ordered element list. Supports
/// arrays and integer ranges (the two common `ForEach` sources).
pub(crate) fn sequence_items(data: &SwiftValue) -> Option<Vec<SwiftValue>> {
    match data {
        SwiftValue::Array(items) => Some(items.iter().cloned().collect()),
        SwiftValue::Range { lo, hi, inclusive } => {
            let end = if *inclusive { *hi + 1 } else { *hi };
            Some((*lo..end).map(SwiftValue::int).collect())
        }
        _ => None,
    }
}

/// Stringify an identity value into a stable, id-safe key: an *injective* escape
/// so distinct identities never collapse to the same key (which would let the
/// keyed diff preserve the wrong row's state). ASCII alphanumerics and `-` pass
/// through; every other byte (including `_` and `.`) becomes `_<hex>`, so the
/// key is a reversible, `.`-free path segment.
pub(crate) fn key_string(value: &SwiftValue) -> String {
    let raw = match value {
        SwiftValue::Str(s) => s.clone(),
        other => other.to_string(),
    };
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' {
            out.push(b as char);
        } else {
            out.push('_');
            out.push_str(&format!("{b:02x}"));
        }
    }
    out
}

/// Attach a stable identity [`KEY_FIELD`] to a view value (copy-on-write).
pub(crate) fn with_key(view: SwiftValue, key: String) -> SwiftValue {
    let SwiftValue::Struct(obj) = view else {
        return view;
    };
    let mut obj = (*obj).clone();
    obj.fields.retain(|(k, _)| k != KEY_FIELD);
    obj.fields.push((KEY_FIELD.into(), SwiftValue::Str(key)));
    SwiftValue::Struct(Rc::new(obj))
}

pub(crate) fn type_error(message: impl Into<String>) -> StdError {
    StdError::Error(EvalError::Type(message.into()))
}

/// Returns the SwiftUI type name of a view value, if it is one.
pub fn view_type_name(value: &SwiftValue) -> Option<&str> {
    match value {
        SwiftValue::Struct(obj) if obj.fields.iter().any(|(k, _)| k == MODIFIERS_FIELD) => {
            Some(obj.type_name.as_str())
        }
        _ => None,
    }
}

/// A node's stable identity key ([`KEY_FIELD`]), set on `ForEach`-generated
/// children so the diff and serializer agree on a position-independent id.
pub fn key_of(value: &SwiftValue) -> Option<&str> {
    match value {
        SwiftValue::Struct(obj) => match obj.get(KEY_FIELD) {
            Some(SwiftValue::Str(s)) => Some(s.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// The UIIR id of the `index`-th child of node `parent_id`. A keyed child
/// (`ForEach` row) uses its stable key so reorders preserve identity; every
/// other child uses its structural position.
pub fn child_id(parent_id: &str, index: usize, child: &SwiftValue) -> String {
    match key_of(child) {
        Some(key) => format!("{parent_id}.{key}"),
        None => format!("{parent_id}.{index}"),
    }
}
