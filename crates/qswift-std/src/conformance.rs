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

use qswift_core::{
    BuiltinReceiver, EvalError, Interpreter, StdContext, StdError, StdResult, SwiftValue,
};

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

    // `hashValue` on the scalar + string Hashable builtins.
    for recv in [
        BuiltinReceiver::Int,
        BuiltinReceiver::Double,
        BuiltinReceiver::Bool,
        BuiltinReceiver::String,
    ] {
        interp.register_property(recv, "hashValue", hash_value);
    }
}

/// `<builtin>.description` — render exactly as `print`/interpolation would.
fn description(c: &mut dyn StdContext, recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Str(c.display(&recv)))
}

/// `<scalar>.hashValue` — a deterministic hash of the value's contents.
fn hash_value(recv: SwiftValue) -> StdResult {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match &recv {
        SwiftValue::Int(i) => i.raw.hash(&mut h),
        SwiftValue::Double(d) => d.to_bits().hash(&mut h),
        SwiftValue::Bool(b) => b.hash(&mut h),
        SwiftValue::Str(s) => s.hash(&mut h),
        other => {
            return Err(StdError::Error(EvalError::Type(format!(
                "hashValue is not available on {}",
                other.type_name()
            ))))
        }
    }
    Ok(SwiftValue::int(h.finish() as i64 as i128))
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
    fn hash_value_rejects_unhashable_builtins() {
        assert!(hash_value(SwiftValue::Void).is_err());
    }
}
