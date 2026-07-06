//! `Optional` method intrinsics (`map`, `flatMap`).
//!
//! tswift models `Optional` with a flattened value: an absent optional is
//! [`SwiftValue::Nil`]; a present one *is* its wrapped value. So `Optional.map`
//! is dispatched on the wrapped value's receiver kind. The scalar kinds
//! (`Int`/`Double`/`Bool`/`String`) have no other `map`, so registering it there
//! is unambiguous; `nil` itself dispatches as the `Optional` receiver.
//!
//! **Declared-type-aware members** — `take()` and `debugDescription` belong to
//! `Optional` itself, not the wrapped type, so they must NOT be reached by
//! wrapped-kind registration (that would let `var x = 1; x.take()` corrupt a
//! non-optional `Int`). Instead they are registered on the `Optional` receiver
//! and the interpreter routes to them only when the *static* type of the
//! receiver expression is optional (via the per-binding declared-type map added
//! in #241). Dispatch on a present optional stored flat as its wrapped value is
//! decided at the member-access site in `interp` (`eval_method_call` /
//! `eval_member`), which consults `static_type_of` before the wrapped-type path.
//!
//! Known gap: a present `Optional<[T]>` is an `Array` receiver, where `map`
//! means `Sequence.map`; the two are indistinguishable in this value model, so
//! the sequence meaning wins (same root cause).

use tswift_core::{
    describe_with_type, BuiltinReceiver, Interpreter, MethodEntry, Outcome, StdContext, StdError,
    StdResult, SwiftValue,
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
    // `take()` and `debugDescription` are registered on the `Optional` receiver
    // ONLY. The interpreter routes to them from the member-access site when the
    // receiver's static type is optional (see the module-level doc comment).
    interp.register_intrinsic(
        BuiltinReceiver::Optional,
        "take",
        MethodEntry {
            mutating: true,
            func: take,
        },
    );
    interp.register_property(
        BuiltinReceiver::Optional,
        "debugDescription",
        debug_description,
    );
}

/// `Optional.take()` — `mutating func take() -> Wrapped?`.
///
/// Returns the current value and resets the receiver to `nil`. In the flattened
/// model the present value *is* the result; the write-back sets the receiver to
/// `SwiftValue::Nil`. A `nil` receiver yields `nil` and stays `nil`.
fn take(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    Ok(Outcome {
        result: recv,
        receiver: SwiftValue::Nil,
    })
}

/// `Optional.debugDescription` — `Optional(<wrapped debugDescription>)` for a
/// present value, `"nil"` for an absent one. The `"_?"` spelling drives
/// [`describe_with_type`] to render the wrapped value as a quoted element.
fn debug_description(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Str(describe_with_type(&recv, Some("_?"))))
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
    fn take_present_returns_value_and_resets_receiver() {
        let mut c = Doubler;
        let out = take(&mut c, SwiftValue::int(5), Vec::new()).unwrap();
        assert_eq!(out.result, SwiftValue::int(5));
        assert_eq!(out.receiver, SwiftValue::Nil);
    }

    #[test]
    fn take_nil_stays_nil() {
        let mut c = Doubler;
        let out = take(&mut c, SwiftValue::Nil, Vec::new()).unwrap();
        assert_eq!(out.result, SwiftValue::Nil);
        assert_eq!(out.receiver, SwiftValue::Nil);
    }

    #[test]
    fn debug_description_wraps_present_quotes_strings() {
        assert_eq!(
            debug_description(SwiftValue::Str("x".into())).unwrap(),
            SwiftValue::Str("Optional(\"x\")".into())
        );
        assert_eq!(
            debug_description(SwiftValue::int(5)).unwrap(),
            SwiftValue::Str("Optional(5)".into())
        );
        assert_eq!(
            debug_description(SwiftValue::Nil).unwrap(),
            SwiftValue::Str("nil".into())
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
