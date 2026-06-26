//! Scalar-value method and property intrinsics for `Int` and `Double`.

use std::rc::Rc;

use qswift_core::{
    BuiltinReceiver, EnumObj, EvalError, IntValue, Interpreter, MethodEntry, Outcome, StdContext,
    StdError, StdResult, SwiftValue,
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
    method(
        interp,
        BuiltinReceiver::Int,
        "addingReportingOverflow",
        int_adding_reporting_overflow,
    );
    method(
        interp,
        BuiltinReceiver::Int,
        "subtractingReportingOverflow",
        int_subtracting_reporting_overflow,
    );
    method(
        interp,
        BuiltinReceiver::Int,
        "multipliedReportingOverflow",
        int_multiplied_reporting_overflow,
    );
    // Int properties.
    interp.register_property(BuiltinReceiver::Int, "magnitude", int_magnitude);
    interp.register_property(BuiltinReceiver::Int, "bitWidth", int_bit_width);
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
    interp.register_property(BuiltinReceiver::Int, "nonzeroBitCount", int_nonzero_bit_count);
    interp.register_property(BuiltinReceiver::Int, "byteSwapped", int_byte_swapped);

    // Double static type constants.
    interp.register_static_property(BuiltinReceiver::Double, "pi", || {
        Ok(SwiftValue::Double(std::f64::consts::PI))
    });
    interp.register_static_property(BuiltinReceiver::Double, "infinity", || {
        Ok(SwiftValue::Double(f64::INFINITY))
    });
    interp.register_static_property(BuiltinReceiver::Double, "nan", || {
        Ok(SwiftValue::Double(f64::NAN))
    });
    interp.register_static_property(BuiltinReceiver::Double, "greatestFiniteMagnitude", || {
        Ok(SwiftValue::Double(f64::MAX))
    });
    interp.register_static_property(BuiltinReceiver::Double, "leastNonzeroMagnitude", || {
        // The smallest positive subnormal: the f64 with bit pattern 1.
        Ok(SwiftValue::Double(f64::from_bits(1)))
    });

    // Double methods.
    method(interp, BuiltinReceiver::Double, "rounded", double_rounded);
    method(interp, BuiltinReceiver::Double, "squareRoot", double_sqrt);
    method(
        interp,
        BuiltinReceiver::Double,
        "truncatingRemainder",
        double_truncating_remainder,
    );
    // Double mutating methods.
    mutating_method(interp, BuiltinReceiver::Double, "round", double_round);
    mutating_method(interp, BuiltinReceiver::Double, "negate", double_negate);
    // Double properties.
    interp.register_property(BuiltinReceiver::Double, "magnitude", double_magnitude);
    interp.register_property(BuiltinReceiver::Double, "isNaN", double_is_nan);
    interp.register_property(BuiltinReceiver::Double, "isFinite", double_is_finite);
    interp.register_property(BuiltinReceiver::Double, "isInfinite", double_is_infinite);
    interp.register_property(BuiltinReceiver::Double, "isZero", double_is_zero);
    interp.register_property(BuiltinReceiver::Double, "sign", double_sign);
    interp.register_property(BuiltinReceiver::Double, "nextUp", double_next_up);
    interp.register_property(BuiltinReceiver::Double, "exponent", double_exponent);
    interp.register_property(BuiltinReceiver::Double, "significand", double_significand);
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

/// Register a mutating method intrinsic (the dispatcher writes the returned
/// receiver back to the caller's storage).
fn mutating_method(
    interp: &mut Interpreter<'_>,
    recv: BuiltinReceiver,
    name: &str,
    func: qswift_core::IntrinsicFn,
) {
    interp.register_intrinsic(recv, name, MethodEntry { mutating: true, func });
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

/// `Int.bitWidth` — the number of bits in the value's binary representation.
fn int_bit_width(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(as_int(&recv)?.width.bits() as i128))
}

/// `Int.leadingZeroBitCount` — zero bits above the most-significant set bit,
/// counted within the value's fixed width.
fn int_leading_zero_bit_count(recv: SwiftValue) -> StdResult {
    let i = as_int(&recv)?;
    let bits = i.width.bits();
    let significant = 128 - unsigned_in_width(i).leading_zeros();
    Ok(SwiftValue::int((bits - significant) as i128))
}

/// `Int.trailingZeroBitCount` — zero bits below the least-significant set bit;
/// equals the bit width when the value is zero.
fn int_trailing_zero_bit_count(recv: SwiftValue) -> StdResult {
    let i = as_int(&recv)?;
    let u = unsigned_in_width(i);
    let count = if u == 0 {
        i.width.bits()
    } else {
        u.trailing_zeros()
    };
    Ok(SwiftValue::int(count as i128))
}

/// `Int.nonzeroBitCount` — population count over the fixed-width representation.
fn int_nonzero_bit_count(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(unsigned_in_width(as_int(&recv)?).count_ones() as i128))
}

/// `Int.byteSwapped` — the value with its bytes in reverse order.
fn int_byte_swapped(recv: SwiftValue) -> StdResult {
    let i = as_int(&recv)?;
    let nbytes = i.width.bits() / 8;
    let mut v = unsigned_in_width(i);
    let mut swapped: u128 = 0;
    for _ in 0..nbytes {
        swapped = (swapped << 8) | (v & 0xff);
        v >>= 8;
    }
    Ok(SwiftValue::Int(IntValue::wrapped(i.width, swapped as i128)))
}

/// `Int.addingReportingOverflow(_:)` — `(partialValue, overflow)` where
/// `partialValue` wraps within the width and `overflow` flags truncation.
fn int_adding_reporting_overflow(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    reporting_overflow(recv, args, "addingReportingOverflow", |a, b| a + b)
}

/// `Int.subtractingReportingOverflow(_:)`.
fn int_subtracting_reporting_overflow(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    reporting_overflow(recv, args, "subtractingReportingOverflow", |a, b| a - b)
}

/// `Int.multipliedReportingOverflow(by:)`.
fn int_multiplied_reporting_overflow(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    reporting_overflow(recv, args, "multipliedReportingOverflow", |a, b| a * b)
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

/// `Double.round()` (mutating) — round to nearest, ties away from zero, in place.
fn double_round(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let rounded = SwiftValue::Double(as_double(&recv)?.round());
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: rounded,
    })
}

/// `Double.negate()` (mutating) — replace the value with its additive inverse.
fn double_negate(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let negated = SwiftValue::Double(-as_double(&recv)?);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: negated,
    })
}

fn double_magnitude(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Double(as_double(&recv)?.abs()))
}

fn double_is_zero(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(as_double(&recv)? == 0.0))
}

/// `Double.sign` — `.minus` when the sign bit is set (incl. `-0.0`), else
/// `.plus`. Returns the builtin `FloatingPointSign` enum.
fn double_sign(recv: SwiftValue) -> StdResult {
    let case = if as_double(&recv)?.is_sign_negative() {
        "minus"
    } else {
        "plus"
    };
    Ok(SwiftValue::Enum(Rc::new(EnumObj {
        type_name: "FloatingPointSign".into(),
        case: case.into(),
        payload: vec![],
    })))
}

/// `Double.nextUp` — the smallest representable value greater than `self`.
fn double_next_up(recv: SwiftValue) -> StdResult {
    let d = as_double(&recv)?;
    let next = if d.is_nan() || d == f64::INFINITY {
        d
    } else if d == 0.0 {
        f64::from_bits(1)
    } else if d > 0.0 {
        f64::from_bits(d.to_bits() + 1)
    } else {
        f64::from_bits(d.to_bits() - 1)
    };
    Ok(SwiftValue::Double(next))
}

/// `Double.exponent` — the unbiased binary exponent of the magnitude. Zero maps
/// to `Int.min` and non-finite values to `Int.max`, matching Swift.
fn double_exponent(recv: SwiftValue) -> StdResult {
    let d = as_double(&recv)?;
    let e = if d == 0.0 {
        i64::MIN as i128
    } else if !d.is_finite() {
        i64::MAX as i128
    } else {
        binary_exponent(d.abs()) as i128
    };
    Ok(SwiftValue::int(e))
}

/// `Double.significand` — the magnitude's significand in `[1, 2)` for finite
/// non-zero values; `0`/`inf`/`nan` map to themselves (magnitude).
fn double_significand(recv: SwiftValue) -> StdResult {
    let d = as_double(&recv)?;
    let s = if d.is_nan() {
        d
    } else if d == 0.0 {
        d
    } else if d.is_infinite() {
        f64::INFINITY
    } else {
        const MASK52: u64 = (1u64 << 52) - 1;
        let bits = d.abs().to_bits();
        let raw_exp = (bits >> 52) & 0x7ff;
        let mantissa = if raw_exp == 0 {
            // Subnormal: shift the leading set bit up into the implicit position.
            let m = bits & MASK52;
            let shift = (m.leading_zeros() - 12) + 1;
            (m << shift) & MASK52
        } else {
            bits & MASK52
        };
        f64::from_bits((1023u64 << 52) | mantissa)
    };
    Ok(SwiftValue::Double(s))
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

/// The value's fixed-width two's-complement representation as an unsigned
/// integer (used by the bit-counting properties).
fn unsigned_in_width(i: IntValue) -> u128 {
    let bits = i.width.bits();
    let mask = if bits >= 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    };
    (i.raw as u128) & mask
}

/// The unbiased binary exponent of a positive, finite, normal-or-subnormal f64.
fn binary_exponent(a: f64) -> i64 {
    let bits = a.to_bits();
    let raw_exp = ((bits >> 52) & 0x7ff) as i64;
    if raw_exp == 0 {
        // Subnormal: leading zeros within the 52-bit mantissa set the exponent.
        let mantissa = bits & ((1u64 << 52) - 1);
        let lz52 = (mantissa.leading_zeros() - 12) as i64;
        -1023 - lz52
    } else {
        raw_exp - 1023
    }
}

/// Shared body for the `*ReportingOverflow` methods: apply `op` in wide
/// arithmetic, wrap into the receiver's width, and flag whether it truncated.
fn reporting_overflow(
    recv: SwiftValue,
    args: Vec<SwiftValue>,
    who: &str,
    op: fn(i128, i128) -> i128,
) -> Outcomes {
    let a = as_int(&recv)?;
    let b = as_int(args.first().ok_or_else(|| arg_err(who))?)?;
    let full = op(a.raw, b.raw);
    let overflow = full < a.width.min() || full > a.width.max();
    let partial = IntValue::wrapped(a.width, full);
    let tuple = SwiftValue::Tuple(vec![SwiftValue::Int(partial), SwiftValue::Bool(overflow)]);
    ok(tuple, recv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qswift_core::IntWidth;

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

    fn int_n(raw: i128, name: &str) -> IntValue {
        IntValue::new(raw, IntWidth::from_type_name(name).unwrap())
    }

    #[test]
    fn bit_counts_are_width_aware() {
        // 255 occupies 8 bits inside a 64-bit Int -> 56 leading zeros.
        assert_eq!(
            int_leading_zero_bit_count(SwiftValue::int(255)).unwrap(),
            SwiftValue::int(56)
        );
        assert_eq!(
            int_leading_zero_bit_count(SwiftValue::int(0)).unwrap(),
            SwiftValue::int(64)
        );
        // -1 is all ones -> no leading zeros, full popcount, no trailing zeros.
        assert_eq!(
            int_leading_zero_bit_count(SwiftValue::int(-1)).unwrap(),
            SwiftValue::int(0)
        );
        assert_eq!(
            int_nonzero_bit_count(SwiftValue::int(-1)).unwrap(),
            SwiftValue::int(64)
        );
        assert_eq!(
            int_trailing_zero_bit_count(SwiftValue::int(8)).unwrap(),
            SwiftValue::int(3)
        );
        // Zero has trailing-zero count equal to the bit width.
        assert_eq!(
            int_trailing_zero_bit_count(SwiftValue::int(0)).unwrap(),
            SwiftValue::int(64)
        );
        assert_eq!(
            int_nonzero_bit_count(SwiftValue::int(255)).unwrap(),
            SwiftValue::int(8)
        );
        assert_eq!(
            int_bit_width(SwiftValue::Int(int_n(0, "Int8"))).unwrap(),
            SwiftValue::int(8)
        );
    }

    #[test]
    fn byte_swapped_reverses_bytes() {
        // 1 as a 64-bit value byte-swaps to 1 << 56.
        assert_eq!(
            int_byte_swapped(SwiftValue::int(1)).unwrap(),
            SwiftValue::int(72057594037927936)
        );
        // Single-byte width is its own swap.
        assert_eq!(
            int_byte_swapped(SwiftValue::Int(int_n(7, "Int8"))).unwrap(),
            SwiftValue::Int(int_n(7, "Int8"))
        );
    }

    #[test]
    fn reporting_overflow_wraps_and_flags() {
        let mut c = MockCtx;
        // Int8 100 + 50 = 150 -> wraps to -106, overflow.
        let r = int_adding_reporting_overflow(
            &mut c,
            SwiftValue::Int(int_n(100, "Int8")),
            vec![SwiftValue::Int(int_n(50, "Int8"))],
        )
        .unwrap()
        .result;
        assert_eq!(
            r,
            SwiftValue::Tuple(vec![SwiftValue::Int(int_n(-106, "Int8")), SwiftValue::Bool(true)])
        );
        // 10 * 3 = 30, no overflow.
        let r = int_multiplied_reporting_overflow(&mut c, SwiftValue::int(10), vec![SwiftValue::int(3)])
            .unwrap()
            .result;
        assert_eq!(
            r,
            SwiftValue::Tuple(vec![SwiftValue::int(30), SwiftValue::Bool(false)])
        );
        // Int.max + 1 overflows to Int.min.
        let r = int_adding_reporting_overflow(
            &mut c,
            SwiftValue::int(i64::MAX as i128),
            vec![SwiftValue::int(1)],
        )
        .unwrap()
        .result;
        assert_eq!(
            r,
            SwiftValue::Tuple(vec![SwiftValue::int(i64::MIN as i128), SwiftValue::Bool(true)])
        );
    }

    #[test]
    fn double_round_and_negate_mutate_receiver() {
        let mut c = MockCtx;
        let out = double_round(&mut c, SwiftValue::Double(2.6), vec![]).unwrap();
        assert_eq!(out.result, SwiftValue::Void);
        assert_eq!(out.receiver, SwiftValue::Double(3.0));
        let out = double_negate(&mut c, SwiftValue::Double(3.5), vec![]).unwrap();
        assert_eq!(out.receiver, SwiftValue::Double(-3.5));
    }

    #[test]
    fn double_exponent_significand_and_edges() {
        assert_eq!(
            double_exponent(SwiftValue::Double(8.0)).unwrap(),
            SwiftValue::int(3)
        );
        assert_eq!(
            double_exponent(SwiftValue::Double(0.5)).unwrap(),
            SwiftValue::int(-1)
        );
        // Magnitude-based: sign does not change the exponent.
        assert_eq!(
            double_exponent(SwiftValue::Double(-8.0)).unwrap(),
            SwiftValue::int(3)
        );
        assert_eq!(
            double_exponent(SwiftValue::Double(0.0)).unwrap(),
            SwiftValue::int(i64::MIN as i128)
        );
        assert_eq!(
            double_exponent(SwiftValue::Double(f64::INFINITY)).unwrap(),
            SwiftValue::int(i64::MAX as i128)
        );
        assert_eq!(
            double_significand(SwiftValue::Double(0.75)).unwrap(),
            SwiftValue::Double(1.5)
        );
        assert_eq!(
            double_significand(SwiftValue::Double(16.0)).unwrap(),
            SwiftValue::Double(1.0)
        );
        // Smallest subnormal normalizes to significand 1.0.
        assert_eq!(
            double_significand(SwiftValue::Double(f64::from_bits(1))).unwrap(),
            SwiftValue::Double(1.0)
        );
    }

    #[test]
    fn sign_returns_floating_point_sign_case() {
        match double_sign(SwiftValue::Double(-3.5)).unwrap() {
            SwiftValue::Enum(e) => {
                assert_eq!(e.type_name, "FloatingPointSign");
                assert_eq!(e.case, "minus");
            }
            other => panic!("expected enum, got {other:?}"),
        }
        // -0.0 has the sign bit set -> .minus; +0.0 -> .plus.
        match double_sign(SwiftValue::Double(-0.0)).unwrap() {
            SwiftValue::Enum(e) => assert_eq!(e.case, "minus"),
            other => panic!("expected enum, got {other:?}"),
        }
        match double_sign(SwiftValue::Double(0.0)).unwrap() {
            SwiftValue::Enum(e) => assert_eq!(e.case, "plus"),
            other => panic!("expected enum, got {other:?}"),
        }
    }

    #[test]
    fn next_up_and_is_zero() {
        // 1.0.nextUp is the adjacent f64 above 1.0 (bit pattern + 1).
        assert_eq!(
            double_next_up(SwiftValue::Double(1.0)).unwrap(),
            SwiftValue::Double(f64::from_bits(1.0_f64.to_bits() + 1))
        );
        assert_eq!(
            double_is_zero(SwiftValue::Double(0.0)).unwrap(),
            SwiftValue::Bool(true)
        );
        assert_eq!(
            double_is_zero(SwiftValue::Double(1.0)).unwrap(),
            SwiftValue::Bool(false)
        );
    }
}
