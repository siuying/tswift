//! Scalar-value method and property intrinsics for `Int` and `Double`.

use qswift_core::{
    BuiltinReceiver, EvalError, IntValue, Interpreter, MethodEntry, Outcome, StdContext, StdError,
    StdResult, SwiftValue,
};

/// Register the `Int`/`Double` intrinsics of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    // Int methods.
    method(interp, BuiltinReceiver::Int, "signum", int_signum);
    method(interp, BuiltinReceiver::Int, "isMultiple", int_is_multiple);
    method(
        interp,
        BuiltinReceiver::Int,
        "quotientAndRemainder",
        int_quotient_and_remainder,
    );
    // Int property.
    interp.register_property(BuiltinReceiver::Int, "magnitude", int_magnitude);

    // Double methods.
    method(interp, BuiltinReceiver::Double, "rounded", double_rounded);
    method(interp, BuiltinReceiver::Double, "squareRoot", double_sqrt);
    method(
        interp,
        BuiltinReceiver::Double,
        "truncatingRemainder",
        double_truncating_remainder,
    );
    // Double properties.
    interp.register_property(BuiltinReceiver::Double, "magnitude", double_magnitude);
    interp.register_property(BuiltinReceiver::Double, "isNaN", double_is_nan);
    interp.register_property(BuiltinReceiver::Double, "isFinite", double_is_finite);
    interp.register_property(BuiltinReceiver::Double, "isInfinite", double_is_infinite);
}

/// Register a non-mutating method intrinsic.
fn method(
    interp: &mut Interpreter<'_>,
    recv: BuiltinReceiver,
    name: &str,
    func: qswift_core::IntrinsicFn,
) {
    interp.register_intrinsic(
        recv,
        name,
        MethodEntry {
            mutating: false,
            func,
        },
    );
}

// ---- Int -------------------------------------------------------------------

/// `Int.signum()` — `-1`, `0`, or `1`.
fn int_signum(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let i = as_int(&recv)?;
    let s = i.raw.signum();
    ok(SwiftValue::Int(IntValue::new(s, i.width)), recv)
}

/// `Int.isMultiple(of:)` — whether `self` is an exact multiple of the argument.
fn int_is_multiple(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let i = as_int(&recv)?;
    let d = as_int(args.first().ok_or_else(|| arg_err("isMultiple(of:)"))?)?;
    let result = if d.raw == 0 {
        i.raw == 0
    } else {
        i.raw % d.raw == 0
    };
    ok(SwiftValue::Bool(result), recv)
}

/// `Int.quotientAndRemainder(dividingBy:)` — the `(quotient, remainder)` pair.
fn int_quotient_and_remainder(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    let i = as_int(&recv)?;
    let d = as_int(args.first().ok_or_else(|| arg_err("quotientAndRemainder(dividingBy:)"))?)?;
    if d.raw == 0 {
        return Err(StdError::Error(EvalError::Trap("division by zero".into())));
    }
    let tuple = SwiftValue::Tuple(vec![
        SwiftValue::Int(IntValue::new(i.raw / d.raw, i.width)),
        SwiftValue::Int(IntValue::new(i.raw % d.raw, i.width)),
    ]);
    ok(tuple, recv)
}

/// `Int.magnitude` — absolute value (typed as the unsigned counterpart in Swift;
/// modelled here as the same width).
fn int_magnitude(recv: SwiftValue) -> StdResult {
    let i = as_int(&recv)?;
    Ok(SwiftValue::Int(IntValue::new(i.raw.abs(), i.width)))
}

// ---- Double ----------------------------------------------------------------

/// `Double.rounded()` — round to the nearest integer, ties away from zero.
fn double_rounded(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let d = as_double(&recv)?;
    ok(SwiftValue::Double(d.round()), recv)
}

/// `Double.squareRoot()` — the non-negative square root.
fn double_sqrt(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let d = as_double(&recv)?;
    ok(SwiftValue::Double(d.sqrt()), recv)
}

/// `Double.truncatingRemainder(dividingBy:)` — IEEE truncating remainder.
fn double_truncating_remainder(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    let d = as_double(&recv)?;
    let by = as_double(args.first().ok_or_else(|| arg_err("truncatingRemainder(dividingBy:)"))?)?;
    ok(SwiftValue::Double(d % by), recv)
}

fn double_magnitude(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Double(as_double(&recv)?.abs()))
}

fn double_is_nan(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(as_double(&recv)?.is_nan()))
}

fn double_is_finite(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(as_double(&recv)?.is_finite()))
}

fn double_is_infinite(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(as_double(&recv)?.is_infinite()))
}

// ---- helpers ---------------------------------------------------------------

type Outcomes = Result<Outcome, StdError>;

fn ok(result: SwiftValue, receiver: SwiftValue) -> Outcomes {
    Ok(Outcome { result, receiver })
}

fn as_int(v: &SwiftValue) -> Result<IntValue, StdError> {
    match v {
        SwiftValue::Int(i) => Ok(*i),
        other => Err(StdError::Error(EvalError::Type(format!(
            "expected an integer, got {}",
            other.type_name()
        )))),
    }
}

fn as_double(v: &SwiftValue) -> Result<f64, StdError> {
    match v {
        SwiftValue::Double(d) => Ok(*d),
        SwiftValue::Int(i) => Ok(i.raw as f64),
        other => Err(StdError::Error(EvalError::Type(format!(
            "expected a floating-point value, got {}",
            other.type_name()
        )))),
    }
}

fn arg_err(who: &str) -> StdError {
    StdError::Error(EvalError::Type(format!("{who} expects one argument")))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockCtx;
    impl StdContext for MockCtx {
        fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> StdResult {
            Ok(SwiftValue::Nil)
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            unreachable!("scalar intrinsics never write output")
        }
    }

    #[test]
    fn signum_and_magnitude() {
        let mut c = MockCtx;
        assert_eq!(
            int_signum(&mut c, SwiftValue::int(-9), vec![]).unwrap().result,
            SwiftValue::int(-1)
        );
        assert_eq!(int_magnitude(SwiftValue::int(-9)).unwrap(), SwiftValue::int(9));
    }

    #[test]
    fn is_multiple_and_quotient_remainder() {
        let mut c = MockCtx;
        assert_eq!(
            int_is_multiple(&mut c, SwiftValue::int(12), vec![SwiftValue::int(4)])
                .unwrap()
                .result,
            SwiftValue::Bool(true)
        );
        let qr = int_quotient_and_remainder(&mut c, SwiftValue::int(17), vec![SwiftValue::int(5)])
            .unwrap()
            .result;
        assert_eq!(
            qr,
            SwiftValue::Tuple(vec![SwiftValue::int(3), SwiftValue::int(2)])
        );
    }

    #[test]
    fn quotient_remainder_traps_on_zero() {
        let mut c = MockCtx;
        assert!(
            int_quotient_and_remainder(&mut c, SwiftValue::int(1), vec![SwiftValue::int(0)])
                .is_err()
        );
    }

    #[test]
    fn double_math() {
        let mut c = MockCtx;
        assert_eq!(
            double_sqrt(&mut c, SwiftValue::Double(9.0), vec![]).unwrap().result,
            SwiftValue::Double(3.0)
        );
        assert_eq!(
            double_rounded(&mut c, SwiftValue::Double(2.6), vec![]).unwrap().result,
            SwiftValue::Double(3.0)
        );
        assert_eq!(
            double_is_nan(SwiftValue::Double(f64::NAN)).unwrap(),
            SwiftValue::Bool(true)
        );
    }
}
