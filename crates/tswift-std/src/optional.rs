//! `Optional` method intrinsics (`map`, `flatMap`).
//!
//! quick-swift models `Optional` with a flattened value: an absent optional is
//! [`SwiftValue::Nil`]; a present one *is* its wrapped value. So `Optional.map`
//! is dispatched on the wrapped value's receiver kind. The scalar kinds
//! (`Int`/`Double`/`Bool`/`String`) have no other `map`, so registering it there
//! is unambiguous; `nil` itself dispatches as the `Optional` receiver.
//!
//! Known gap: a present `Optional<[T]>` is an `Array` receiver, where `map`
//! means `Sequence.map`; the two are indistinguishable in this value model, so
//! the sequence meaning wins (documented limitation).

use tswift_core::{
    BuiltinReceiver, Interpreter, MethodEntry, Outcome, StdContext, StdError, SwiftValue,
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
    }
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
    fn map_on_nil_short_circuits() {
        let mut c = Doubler;
        let out = map_or_flat_map(&mut c, SwiftValue::Nil, vec![SwiftValue::Closure(0)])
            .unwrap()
            .result;
        assert_eq!(out, SwiftValue::Nil);
    }
}
