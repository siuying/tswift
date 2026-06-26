//! Builtin conformance accessors: `description` (CustomStringConvertible) and
//! `hashValue` (Hashable), exposed as explicit members on builtin values.
//!
//! `description` renders through [`StdContext::display`], the same path `print`
//! and string interpolation use, so a builtin's explicit `.description` matches
//! its printed form (and honours nested `CustomStringConvertible` values).
//!
//! `hashValue` is a deterministic hash of the value's contents. Swift seeds
//! hashing randomly per process, so the exact integer is unspecified; programs
//! (and fixtures) may only rely on its *consistency* within a run.

use std::hash::{Hash, Hasher};

use qswift_core::{BuiltinReceiver, Interpreter, StdContext, StdResult, SwiftValue};

/// Register the builtin conformance accessors of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    // `description` on every CustomStringConvertible builtin.
    for recv in [
        BuiltinReceiver::Int,
        BuiltinReceiver::Double,
        BuiltinReceiver::Bool,
        BuiltinReceiver::String,
        BuiltinReceiver::Array,
        BuiltinReceiver::Dictionary,
        BuiltinReceiver::Set,
    ] {
        interp.register_context_property(recv, "description", description);
    }

    // `debugDescription` on the CustomDebugStringConvertible builtins. Note
    // Swift gives `Int`/`Bool` *no* `debugDescription`, so they are excluded.
    // For `Optional` only the `.none` (nil) case is reachable here — a present
    // optional is unboxed to its wrapped value in this runtime.
    for recv in [
        BuiltinReceiver::Double,
        BuiltinReceiver::String,
        BuiltinReceiver::Array,
        BuiltinReceiver::Dictionary,
        BuiltinReceiver::Set,
        BuiltinReceiver::Optional,
    ] {
        interp.register_context_property(recv, "debugDescription", debug_description);
    }

    // `hashValue` on every Hashable builtin (scalars, String, and — recursively
    // — the collections + the nil Optional).
    for recv in [
        BuiltinReceiver::Int,
        BuiltinReceiver::Double,
        BuiltinReceiver::Bool,
        BuiltinReceiver::String,
        BuiltinReceiver::Array,
        BuiltinReceiver::Dictionary,
        BuiltinReceiver::Set,
        BuiltinReceiver::Optional,
    ] {
        interp.register_property(recv, "hashValue", hash_value);
    }
}

/// `<builtin>.description` — render exactly as `print`/interpolation would.
fn description(c: &mut dyn StdContext, recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Str(c.display(&recv)))
}

/// `<builtin>.debugDescription` — render as `String(reflecting:)` would.
fn debug_description(c: &mut dyn StdContext, recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Str(c.debug_display(&recv)))
}

/// `<builtin>.hashValue` — a deterministic, structurally-recursive hash.
fn hash_value(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(hash_of(&recv) as i64 as i128))
}

/// A deterministic hash over a value's contents. Order-independent for `Set`
/// and `Dictionary` (so equal-but-reordered collections hash equally),
/// order-sensitive for `Array`/`Tuple`.
fn hash_of(v: &SwiftValue) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match v {
        SwiftValue::Int(i) => (0u8, i.raw).hash(&mut h),
        SwiftValue::Double(d) => (1u8, d.to_bits()).hash(&mut h),
        SwiftValue::Bool(b) => (2u8, b).hash(&mut h),
        SwiftValue::Str(s) => (3u8, s).hash(&mut h),
        SwiftValue::Nil => 4u8.hash(&mut h),
        SwiftValue::Array(xs) => {
            5u8.hash(&mut h);
            for x in xs.iter() {
                hash_of(x).hash(&mut h);
            }
        }
        SwiftValue::Tuple(xs) => {
            6u8.hash(&mut h);
            for x in xs.iter() {
                hash_of(x).hash(&mut h);
            }
        }
        SwiftValue::Set(xs) => {
            // XOR is order-independent; Set elements are unique so it is sound.
            let combined = xs.iter().fold(0u64, |acc, x| acc ^ hash_of(x));
            (7u8, combined).hash(&mut h);
        }
        SwiftValue::Dict(pairs) => {
            // Sum per-pair hashes so key/value pairs combine order-independently.
            let combined = pairs.iter().fold(0u64, |acc, (k, val)| {
                acc.wrapping_add(hash_of(k).rotate_left(1) ^ hash_of(val))
            });
            (8u8, combined).hash(&mut h);
        }
        SwiftValue::Enum(e) => {
            (9u8, &e.type_name, &e.case).hash(&mut h);
            for p in e.payload.iter() {
                hash_of(p).hash(&mut h);
            }
        }
        other => {
            // Reference/function-like values fall back to a type-keyed hash.
            (255u8, other.type_name()).hash(&mut h);
        }
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A context whose `display` echoes the plain value rendering, standing in
    /// for the interpreter's `CustomStringConvertible`-aware renderer.
    struct EchoCtx;
    impl StdContext for EchoCtx {
        fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> StdResult {
            Ok(SwiftValue::Nil)
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            unreachable!()
        }
    }

    #[test]
    fn description_renders_through_display() {
        let mut c = EchoCtx;
        assert_eq!(
            description(&mut c, SwiftValue::int(42)).unwrap(),
            SwiftValue::Str("42".into())
        );
        assert_eq!(
            description(&mut c, SwiftValue::Str("hi".into())).unwrap(),
            SwiftValue::Str("hi".into())
        );
    }

    #[test]
    fn hash_value_is_consistent_within_a_run() {
        // The same value hashes equal; the contract programs may rely on.
        assert_eq!(
            hash_value(SwiftValue::int(42)).unwrap(),
            hash_value(SwiftValue::int(42)).unwrap()
        );
        assert_eq!(
            hash_value(SwiftValue::Str("a".into())).unwrap(),
            hash_value(SwiftValue::Str("a".into())).unwrap()
        );
        // Distinct values almost certainly differ.
        assert_ne!(
            hash_value(SwiftValue::int(1)).unwrap(),
            hash_value(SwiftValue::int(2)).unwrap()
        );
    }

    #[test]
    fn hash_value_is_recursive_and_order_independent() {
        use std::rc::Rc;
        // Arrays hash by element order.
        let a = SwiftValue::Array(Rc::new(vec![SwiftValue::int(1), SwiftValue::int(2)]));
        let b = SwiftValue::Array(Rc::new(vec![SwiftValue::int(1), SwiftValue::int(2)]));
        assert_eq!(hash_value(a).unwrap(), hash_value(b).unwrap());
        // Sets hash independent of element order.
        let s1 = SwiftValue::Set(Rc::new(vec![SwiftValue::int(1), SwiftValue::int(2)]));
        let s2 = SwiftValue::Set(Rc::new(vec![SwiftValue::int(2), SwiftValue::int(1)]));
        assert_eq!(hash_value(s1).unwrap(), hash_value(s2).unwrap());
        // nil hashes consistently.
        assert_eq!(
            hash_value(SwiftValue::Nil).unwrap(),
            hash_value(SwiftValue::Nil).unwrap()
        );
    }
}
