//! `Bool` method and property intrinsics.

use tswift_core::{
    Arg, BuiltinReceiver, EvalError, Interpreter, MethodEntry, Outcome, StdContext, StdError,
    StdResult, SwiftValue,
};

/// Register the `Bool` intrinsics of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_intrinsic(
        BuiltinReceiver::Bool,
        "toggle",
        MethodEntry {
            mutating: true,
            func: toggle,
        },
    );
    interp.register_property(BuiltinReceiver::Bool, "description", description);
    interp.register_property(BuiltinReceiver::Bool, "hashValue", hash_value);
    interp.register_static(BuiltinReceiver::Bool, "random", random);
}

/// `Bool.random()` — a uniformly random boolean from the builtin RNG. The
/// no-argument form is the only one supported; `random(using:)` is rejected
/// rather than silently treated as `random()`.
fn random(c: &mut dyn StdContext, a: Vec<Arg>) -> StdResult {
    if !a.is_empty() {
        return Err(StdError::Error(EvalError::Type(
            "Bool.random() takes no arguments".into(),
        )));
    }
    Ok(SwiftValue::Bool(c.random_u64() & 1 == 1))
}

type Outcomes = Result<Outcome, StdError>;

/// `Bool.toggle()` — flip the value in place. Returns `Void` (`()`), matching
/// Swift's `mutating func toggle()`; it takes no arguments.
fn toggle(_c: &mut dyn StdContext, recv: SwiftValue, a: Vec<SwiftValue>) -> Outcomes {
    if !a.is_empty() {
        return Err(StdError::Error(EvalError::Type(
            "Bool.toggle() takes no arguments".into(),
        )));
    }
    let b = as_bool(&recv)?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Bool(!b),
    })
}

/// `Bool.description` — `"true"` or `"false"`.
fn description(recv: SwiftValue) -> StdResult {
    let b = as_bool(&recv)?;
    Ok(SwiftValue::Str(if b { "true" } else { "false" }.into()))
}

/// `Bool.hashValue` — a deterministic per-run hash (`1` for `true`, `0` for
/// `false`). Swift seeds its hasher per process, so only equality of hashes for
/// equal values is observable; this models that with a stable witness.
fn hash_value(recv: SwiftValue) -> StdResult {
    let b = as_bool(&recv)?;
    Ok(SwiftValue::int(if b { 1 } else { 0 }))
}

fn as_bool(v: &SwiftValue) -> Result<bool, StdError> {
    match v {
        SwiftValue::Bool(b) => Ok(*b),
        other => Err(StdError::Error(EvalError::Type(format!(
            "expected a boolean, got {}",
            other.type_name()
        )))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockCtx {
        rng: u64,
    }
    impl StdContext for MockCtx {
        fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> StdResult {
            Ok(SwiftValue::Nil)
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            unreachable!("bool intrinsics never write output")
        }
        fn random_u64(&mut self) -> u64 {
            self.rng = self.rng.wrapping_add(1);
            self.rng
        }
    }

    fn ctx() -> MockCtx {
        MockCtx { rng: 0 }
    }

    #[test]
    fn random_draws_both_values() {
        let mut c = ctx();
        // The mock RNG yields 1, 2, 3, …; low bit alternates true/false.
        assert_eq!(random(&mut c, vec![]).unwrap(), SwiftValue::Bool(true));
        assert_eq!(random(&mut c, vec![]).unwrap(), SwiftValue::Bool(false));
        assert_eq!(random(&mut c, vec![]).unwrap(), SwiftValue::Bool(true));
    }

    #[test]
    fn random_rejects_arguments() {
        let mut c = ctx();
        assert!(random(&mut c, vec![Arg::positional(SwiftValue::int(1))]).is_err());
    }

    #[test]
    fn toggle_flips_value() {
        let mut c = ctx();
        let out = toggle(&mut c, SwiftValue::Bool(true), vec![]).unwrap();
        assert_eq!(out.receiver, SwiftValue::Bool(false));
        assert_eq!(out.result, SwiftValue::Void);
        let out = toggle(&mut c, SwiftValue::Bool(false), vec![]).unwrap();
        assert_eq!(out.receiver, SwiftValue::Bool(true));
    }

    #[test]
    fn toggle_rejects_arguments() {
        let mut c = ctx();
        assert!(toggle(&mut c, SwiftValue::Bool(true), vec![SwiftValue::int(1)]).is_err());
    }

    #[test]
    fn description_renders_literal() {
        assert_eq!(
            description(SwiftValue::Bool(true)).unwrap(),
            SwiftValue::Str("true".into())
        );
        assert_eq!(
            description(SwiftValue::Bool(false)).unwrap(),
            SwiftValue::Str("false".into())
        );
    }

    #[test]
    fn hash_value_is_stable() {
        assert_eq!(
            hash_value(SwiftValue::Bool(true)).unwrap(),
            hash_value(SwiftValue::Bool(true)).unwrap()
        );
        assert_ne!(
            hash_value(SwiftValue::Bool(true)).unwrap(),
            hash_value(SwiftValue::Bool(false)).unwrap()
        );
    }

    #[test]
    fn non_bool_receiver_errors() {
        let mut c = ctx();
        assert!(toggle(&mut c, SwiftValue::int(1), vec![]).is_err());
        assert!(description(SwiftValue::int(1)).is_err());
    }
}
