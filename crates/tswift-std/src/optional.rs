//! `Optional` method intrinsics (`map`, `flatMap`).
//!
//! tswift models `Optional` with a flattened value: an absent optional is
//! [`SwiftValue::Nil`]; a present one *is* its wrapped value. So `Optional.map`
//! is dispatched on the wrapped value's receiver kind. The scalar kinds
//! (`Int`/`Double`/`Bool`/`String`) have no other `map`, so registering it there
//! is unambiguous; `nil` itself dispatches as the `Optional` receiver.
//!
//! **Known dispatch limitation** — `take()` and `debugDescription` are NOT
//! registered because both would require knowing the *declared* type of the
//! receiver variable at call time:
//!
//! * A present `Optional<Int>` is stored as `SwiftValue::Int(n)` at runtime,
//!   indistinguishable from a plain `Int`.  Routing `take()` to an Optional
//!   implementation via wrapped-kind registration would allow `var x = 1;
//!   x.take()` (non-optional) to silently corrupt `x` to `nil`.
//! * The `Binding` struct in `env.rs` stores only the current value and
//!   mutability flag — no declared type.  Declared-type-aware dispatch would
//!   require storing the type annotation in every binding and threading it
//!   through ~20 `env.declare()` call sites — deferred to a future slice.
//!
//! Known gap: a present `Optional<[T]>` is an `Array` receiver, where `map`
//! means `Sequence.map`; the two are indistinguishable in this value model, so
//! the sequence meaning wins (same root cause).

use tswift_core::{
    BuiltinReceiver, Interpreter, MethodEntry, Outcome, StdContext, StdError, StdResult, SwiftValue,
};

/// Receiver kinds a present optional's wrapped value can take (excluding Array).
const WRAPPED_KINDS: [BuiltinReceiver; 5] = [
    BuiltinReceiver::Int,
    BuiltinReceiver::Double,
    BuiltinReceiver::Bool,
    BuiltinReceiver::String,
    BuiltinReceiver::Optional,
];

/// Register `Optional.map`/`flatMap` across the wrapped-value receiver kinds.
pub fn install(interp: &mut Interpreter<'_>) {
    for kind in WRAPPED_KINDS {
        interp.register_intrinsic(kind, "map", entry());
        interp.register_intrinsic(kind, "flatMap", entry());
        // `unsafelyUnwrapped` yields the wrapped value. (`nil.unsafelyUnwrapped`
        // follows the interpreter's optional-member semantics on `Nil`.)
        interp.register_property(kind, "unsafelyUnwrapped", unsafely_unwrapped);
    }
    // NOTE: `take()` and `debugDescription` are intentionally NOT registered.
    // See module-level doc comment for the full rationale (declared-type-aware
    // dispatch is required but not yet implemented).
}

/// `Optional.unsafelyUnwrapped` — the wrapped value of a present optional.
fn unsafely_unwrapped(recv: SwiftValue) -> StdResult {
    Ok(recv)
}

fn entry() -> MethodEntry {
    MethodEntry {
        mutating: false,
        func: map_or_flat_map,
    }
}

/// `Optional.map(_:)` / `Optional.flatMap(_:)`.
///
/// `nil` short-circuits to `nil`; otherwise the closure is applied to the
/// wrapped value and its result becomes the new optional. In the flattened
/// model both behave identically (the result *is* the produced optional).
fn map_or_flat_map(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if matches!(recv, SwiftValue::Nil) {
        return Ok(Outcome {
            result: SwiftValue::Nil,
            receiver: recv,
        });
    }
    let id = args.iter().find_map(|a| match a {
        SwiftValue::Closure(id) => Some(*id),
        _ => None,
    });
    let Some(id) = id else {
        return Err(StdError::Error(tswift_core::EvalError::Type(
            "map/flatMap expects a closure".into(),
        )));
    };
    let result = ctx.call_closure(id, vec![recv.clone()])?;
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tswift_core::StdResult;

    /// A mock that applies a fixed transform (double an Int) as "the closure".
    struct Doubler;
    impl StdContext for Doubler {
        fn call_closure(&mut self, _id: usize, args: Vec<SwiftValue>) -> StdResult {
            match args.first() {
                Some(SwiftValue::Int(i)) => Ok(SwiftValue::int(i.raw * 2)),
                _ => Ok(SwiftValue::Nil),
            }
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            unreachable!()
        }
    }

    #[test]
    fn map_on_present_applies_closure() {
        let mut c = Doubler;
        let out = map_or_flat_map(&mut c, SwiftValue::int(5), vec![SwiftValue::Closure(0)])
            .unwrap()
            .result;
        assert_eq!(out, SwiftValue::int(10));
    }

    #[test]
    fn unsafely_unwrapped_returns_wrapped_value() {
        assert_eq!(
            unsafely_unwrapped(SwiftValue::int(5)).unwrap(),
            SwiftValue::int(5)
        );
        assert_eq!(
            unsafely_unwrapped(SwiftValue::Str("hi".into())).unwrap(),
            SwiftValue::Str("hi".into())
        );
    }

    #[test]
    fn map_on_nil_short_circuits() {
        let mut c = Doubler;
        let out = map_or_flat_map(&mut c, SwiftValue::Nil, vec![SwiftValue::Closure(0)])
            .unwrap()
            .result;
        assert_eq!(out, SwiftValue::Nil);
    }
}
