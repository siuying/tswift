//! Scalar-value method and property intrinsics for `Int` and `Double`.

use tswift_core::{
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
    // Int properties.
    interp.register_property(BuiltinReceiver::Int, "magnitude", int_magnitude);
    interp.register_property(BuiltinReceiver::Int, "bitWidth", int_bit_width);
    interp.register_property(
        BuiltinReceiver::Int,
        "nonzeroBitCount",
        int_nonzero_bit_count,
    );
    interp.register_property(
        BuiltinReceiver::Int,
        "leadingZeroBitCount",
        int_leading_zero_bit_count,
    );
    interp.register_property(
        BuiltinReceiver::Int,
        "trailingZeroBitCount",
        int_trailing_zero_bit_count,
    );
    interp.register_property(BuiltinReceiver::Int, "byteSwapped", int_byte_swapped);

    // Double methods.
    method(interp, BuiltinReceiver::Double, "rounded", double_rounded);
    method(interp, BuiltinReceiver::Double, "squareRoot", double_sqrt);
    method(
        interp,
        BuiltinReceiver::Double,
        "truncatingRemainder",
        double_truncating_remainder,
    );
    interp.register_intrinsic(
        BuiltinReceiver::Double,
        "negate",
        MethodEntry {
            mutating: true,
            func: double_negate,
        },
    );
    // Double properties.
    interp.register_property(BuiltinReceiver::Double, "magnitude", double_magnitude);
    interp.register_property(BuiltinReceiver::Double, "isNaN", double_is_nan);
    interp.register_property(BuiltinReceiver::Double, "isFinite", double_is_finite);
    interp.register_property(BuiltinReceiver::Double, "isInfinite", double_is_infinite);
    interp.register_property(BuiltinReceiver::Double, "isZero", double_is_zero);
    interp.register_property(BuiltinReceiver::Double, "isNormal", double_is_normal);
    interp.register_property(BuiltinReceiver::Double, "isSubnormal", double_is_subnormal);
}

/// Width-masked unsigned bit pattern of an integer (two's complement within
/// `width`). Used by the bit-counting properties.
fn bit_pattern(i: IntValue) -> (u128, u32) {
    let bits = i.width.bits();
    let mask: u128 = if bits >= 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    };
    ((i.raw as u128) & mask, bits)
}

/// Register a non-mutating method intrinsic.
fn method(
    interp: &mut Interpreter<'_>,
    recv: BuiltinReceiver,
    name: &str,
    func: tswift_core::IntrinsicFn,
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
    let d = as_int(
        args.first()
            .ok_or_else(|| arg_err("quotientAndRemainder(dividingBy:)"))?,
    )?;
    if d.raw == 0 {
        return Err(StdError::Error(EvalError::Trap("division by zero".into())));
    }
    let tuple = SwiftValue::tuple_labeled(
        vec![
            SwiftValue::Int(IntValue::new(i.raw / d.raw, i.width)),
            SwiftValue::Int(IntValue::new(i.raw % d.raw, i.width)),
        ],
        vec![Some("quotient".to_string()), Some("remainder".to_string())],
    );
    ok(tuple, recv)
}

/// `Int.magnitude` — absolute value (typed as the unsigned counterpart in Swift;
/// modelled here as the same width).
fn int_magnitude(recv: SwiftValue) -> StdResult {
    let i = as_int(&recv)?;
    Ok(SwiftValue::Int(IntValue::new(i.raw.abs(), i.width)))
}

/// `Int.bitWidth` — the number of bits in this integer's representation.
fn int_bit_width(recv: SwiftValue) -> StdResult {
    let i = as_int(&recv)?;
    Ok(SwiftValue::int(i128::from(i.width.bits())))
}

/// `Int.nonzeroBitCount` — population count of the two's-complement pattern.
fn int_nonzero_bit_count(recv: SwiftValue) -> StdResult {
    let (v, _) = bit_pattern(as_int(&recv)?);
    Ok(SwiftValue::int(i128::from(v.count_ones())))
}

/// `Int.leadingZeroBitCount` — zero bits above the most-significant set bit.
fn int_leading_zero_bit_count(recv: SwiftValue) -> StdResult {
    let (v, bits) = bit_pattern(as_int(&recv)?);
    let leading = v.leading_zeros() - (128 - bits);
    Ok(SwiftValue::int(i128::from(leading)))
}

/// `Int.trailingZeroBitCount` — zero bits below the least-significant set bit
/// (the full width when the value is zero).
fn int_trailing_zero_bit_count(recv: SwiftValue) -> StdResult {
    let (v, bits) = bit_pattern(as_int(&recv)?);
    let trailing = if v == 0 { bits } else { v.trailing_zeros() };
    Ok(SwiftValue::int(i128::from(trailing)))
}

/// `Int.byteSwapped` — the value with its bytes in reverse order.
fn int_byte_swapped(recv: SwiftValue) -> StdResult {
    let i = as_int(&recv)?;
    let (v, bits) = bit_pattern(i);
    let nbytes = (bits / 8) as usize;
    let mut swapped: u128 = 0;
    for b in 0..nbytes {
        let byte = (v >> (8 * b)) & 0xff;
        swapped |= byte << (8 * (nbytes - 1 - b));
    }
    Ok(SwiftValue::Int(IntValue::wrapped(i.width, swapped as i128)))
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
    let by = as_double(
        args.first()
            .ok_or_else(|| arg_err("truncatingRemainder(dividingBy:)"))?,
    )?;
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

fn double_is_zero(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(as_double(&recv)? == 0.0))
}

fn double_is_normal(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(as_double(&recv)?.is_normal()))
}

fn double_is_subnormal(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(as_double(&recv)?.is_subnormal()))
}

/// `Double.negate()` — flip the sign in place (`mutating func negate()`).
fn double_negate(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let d = as_double(&recv)?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Double(-d),
    })
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
            int_signum(&mut c, SwiftValue::int(-9), vec![])
                .unwrap()
                .result,
            SwiftValue::int(-1)
        );
        assert_eq!(
            int_magnitude(SwiftValue::int(-9)).unwrap(),
            SwiftValue::int(9)
        );
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
            SwiftValue::tuple(vec![SwiftValue::int(3), SwiftValue::int(2)])
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
    fn int_bit_properties() {
        assert_eq!(
            int_bit_width(SwiftValue::int(42)).unwrap(),
            SwiftValue::int(64)
        );
        assert_eq!(
            int_nonzero_bit_count(SwiftValue::int(42)).unwrap(),
            SwiftValue::int(3)
        );
        assert_eq!(
            int_leading_zero_bit_count(SwiftValue::int(42)).unwrap(),
            SwiftValue::int(58)
        );
        assert_eq!(
            int_trailing_zero_bit_count(SwiftValue::int(42)).unwrap(),
            SwiftValue::int(1)
        );
        // Zero has no set bits: full-width leading and trailing zero counts.
        assert_eq!(
            int_leading_zero_bit_count(SwiftValue::int(0)).unwrap(),
            SwiftValue::int(64)
        );
        assert_eq!(
            int_trailing_zero_bit_count(SwiftValue::int(0)).unwrap(),
            SwiftValue::int(64)
        );
        assert_eq!(
            int_byte_swapped(SwiftValue::int(1)).unwrap(),
            SwiftValue::int(72057594037927936)
        );
    }

    #[test]
    fn double_classification_and_negate() {
        let mut c = MockCtx;
        assert_eq!(
            double_negate(&mut c, SwiftValue::Double(3.5), vec![])
                .unwrap()
                .receiver,
            SwiftValue::Double(-3.5)
        );
        assert_eq!(
            double_is_zero(SwiftValue::Double(0.0)).unwrap(),
            SwiftValue::Bool(true)
        );
        assert_eq!(
            double_is_normal(SwiftValue::Double(1.0)).unwrap(),
            SwiftValue::Bool(true)
        );
        assert_eq!(
            double_is_normal(SwiftValue::Double(0.0)).unwrap(),
            SwiftValue::Bool(false)
        );
        assert_eq!(
            double_is_subnormal(SwiftValue::Double(1.0)).unwrap(),
            SwiftValue::Bool(false)
        );
    }

    #[test]
    fn double_math() {
        let mut c = MockCtx;
        assert_eq!(
            double_sqrt(&mut c, SwiftValue::Double(9.0), vec![])
                .unwrap()
                .result,
            SwiftValue::Double(3.0)
        );
        assert_eq!(
            double_rounded(&mut c, SwiftValue::Double(2.6), vec![])
                .unwrap()
                .result,
            SwiftValue::Double(3.0)
        );
        assert_eq!(
            double_is_nan(SwiftValue::Double(f64::NAN)).unwrap(),
            SwiftValue::Bool(true)
        );
    }
}
