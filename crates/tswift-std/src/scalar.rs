//! Scalar-value method and property intrinsics for `Int` and `Double`.

use std::rc::Rc;
use tswift_core::{
    format_double, BuiltinReceiver, EnumObj, EvalError, IntValue, IntWidth, Interpreter,
    MethodEntry, Outcome, StdContext, StdError, StdResult, SwiftValue,
};

/// Register the `Int`/`Double` intrinsics of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    // `Double.sign` results (stdlib type, so registered under Swift, not
    // Foundation, for strict import-gating).
    interp.register_builtin_enum("FloatingPointSign", &["plus", "minus"]);
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
    method(
        interp,
        BuiltinReceiver::Int,
        "dividedReportingOverflow",
        int_divided_reporting_overflow,
    );
    method(
        interp,
        BuiltinReceiver::Int,
        "remainderReportingOverflow",
        int_remainder_reporting_overflow,
    );
    method(interp, BuiltinReceiver::Int, "distance", int_distance);
    method(interp, BuiltinReceiver::Int, "advanced", int_advanced);
    method(
        interp,
        BuiltinReceiver::Int,
        "multipliedFullWidth",
        int_multiplied_full_width,
    );
    method(
        interp,
        BuiltinReceiver::Int,
        "dividingFullWidth",
        int_dividing_full_width,
    );
    // Int properties.
    interp.register_property(BuiltinReceiver::Int, "magnitude", int_magnitude);
    interp.register_property(BuiltinReceiver::Int, "description", int_description);
    interp.register_property(BuiltinReceiver::Int, "hashValue", int_hash_value);
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
    interp.register_property(BuiltinReceiver::Int, "words", int_words);

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
    method(
        interp,
        BuiltinReceiver::Double,
        "remainder",
        double_remainder,
    );
    method(interp, BuiltinReceiver::Double, "isEqual", double_is_equal);
    method(interp, BuiltinReceiver::Double, "isLess", double_is_less);
    method(
        interp,
        BuiltinReceiver::Double,
        "isLessThanOrEqualTo",
        double_is_less_or_equal,
    );
    method(interp, BuiltinReceiver::Double, "advanced", double_advanced);
    method(interp, BuiltinReceiver::Double, "distance", double_distance);
    mutating(
        interp,
        BuiltinReceiver::Double,
        "formSquareRoot",
        double_form_square_root,
    );
    mutating(
        interp,
        BuiltinReceiver::Double,
        "formTruncatingRemainder",
        double_form_truncating_remainder,
    );
    mutating(
        interp,
        BuiltinReceiver::Double,
        "formRemainder",
        double_form_remainder,
    );
    mutating(interp, BuiltinReceiver::Double, "round", double_round);
    mutating(
        interp,
        BuiltinReceiver::Double,
        "addProduct",
        double_add_product,
    );
    // Double properties.
    interp.register_property(BuiltinReceiver::Double, "magnitude", double_magnitude);
    interp.register_property(BuiltinReceiver::Double, "isNaN", double_is_nan);
    interp.register_property(BuiltinReceiver::Double, "isFinite", double_is_finite);
    interp.register_property(BuiltinReceiver::Double, "isInfinite", double_is_infinite);
    interp.register_property(BuiltinReceiver::Double, "isZero", double_is_zero);
    interp.register_property(BuiltinReceiver::Double, "isNormal", double_is_normal);
    interp.register_property(BuiltinReceiver::Double, "isSubnormal", double_is_subnormal);
    interp.register_property(BuiltinReceiver::Double, "isSignMinus", double_is_sign_minus);
    interp.register_property(BuiltinReceiver::Double, "description", double_description);
    interp.register_property(
        BuiltinReceiver::Double,
        "debugDescription",
        double_description,
    );
    interp.register_property(BuiltinReceiver::Double, "nextUp", double_next_up);
    interp.register_property(BuiltinReceiver::Double, "ulp", double_ulp);
    interp.register_property(BuiltinReceiver::Double, "bitPattern", double_bit_pattern);
    interp.register_property(BuiltinReceiver::Double, "exponent", double_exponent);
    interp.register_property(BuiltinReceiver::Double, "significand", double_significand);
    interp.register_property(BuiltinReceiver::Double, "binade", double_binade);
    interp.register_property(
        BuiltinReceiver::Double,
        "exponentBitPattern",
        double_exponent_bit_pattern,
    );
    interp.register_property(
        BuiltinReceiver::Double,
        "significandBitPattern",
        double_significand_bit_pattern,
    );
    interp.register_property(BuiltinReceiver::Double, "sign", double_sign);
    interp.register_property(
        BuiltinReceiver::Double,
        "significandWidth",
        double_significand_width,
    );
    interp.register_property(BuiltinReceiver::Double, "isCanonical", double_is_canonical);
    interp.register_property(
        BuiltinReceiver::Double,
        "isSignalingNaN",
        double_is_signaling_nan,
    );
    interp.register_property(BuiltinReceiver::Double, "hashValue", double_hash_value);
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

/// Register a mutating method intrinsic (writes a new receiver in place).
fn mutating(
    interp: &mut Interpreter<'_>,
    recv: BuiltinReceiver,
    name: &str,
    func: tswift_core::IntrinsicFn,
) {
    interp.register_intrinsic(
        recv,
        name,
        MethodEntry {
            mutating: true,
            func,
        },
    );
}

/// IEEE 754 remainder: `x - round_ties_even(x / y) * y`. NaN when `y` is zero
/// or `x` is non-finite, matching `Double.remainder(dividingBy:)`.
fn ieee_remainder(x: f64, y: f64) -> f64 {
    if y == 0.0 || !x.is_finite() {
        return f64::NAN;
    }
    x - (x / y).round_ties_even() * y
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

/// `Int.dividedReportingOverflow(by:)` — `(partialValue, overflow)` for `self / other`.
/// Dividing by zero or overflowing (`min / -1`) reports `overflow == true`.
fn int_divided_reporting_overflow(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    let a = as_int(&recv)?;
    let b = as_int(
        args.first()
            .ok_or_else(|| arg_err("dividedReportingOverflow(by:)"))?,
    )?;
    let (partial, overflow) = if b.raw == 0 {
        (a.raw, true)
    } else if a.raw == a.width.min() && b.raw == -1 {
        (a.width.min(), true)
    } else {
        (a.raw / b.raw, false)
    };
    ok(overflow_pair(a.width, partial, overflow), recv)
}

/// `Int.remainderReportingOverflow(dividingBy:)` — `(partialValue, overflow)` for `self % other`.
fn int_remainder_reporting_overflow(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    let a = as_int(&recv)?;
    let b = as_int(
        args.first()
            .ok_or_else(|| arg_err("remainderReportingOverflow(dividingBy:)"))?,
    )?;
    let (partial, overflow) = if b.raw == 0 {
        (a.raw, true)
    } else if a.raw == a.width.min() && b.raw == -1 {
        (0, true)
    } else {
        (a.raw % b.raw, false)
    };
    ok(overflow_pair(a.width, partial, overflow), recv)
}

/// Build the labelled `(partialValue, overflow)` tuple shared by the
/// reporting-overflow division methods.
fn overflow_pair(width: tswift_core::IntWidth, partial: i128, overflow: bool) -> SwiftValue {
    SwiftValue::tuple_labeled(
        vec![
            SwiftValue::Int(IntValue::wrapped(width, partial)),
            SwiftValue::Bool(overflow),
        ],
        vec![
            Some("partialValue".to_string()),
            Some("overflow".to_string()),
        ],
    )
}

/// `Int.distance(to:)` — `other - self` (the `Strideable` conformance).
fn int_distance(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let a = as_int(&recv)?;
    let other = as_int(args.first().ok_or_else(|| arg_err("distance(to:)"))?)?;
    ok(
        SwiftValue::Int(IntValue::new(other.raw - a.raw, a.width)),
        recv,
    )
}

/// `Int.advanced(by:)` — `self + amount` (the `Strideable` conformance).
fn int_advanced(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let a = as_int(&recv)?;
    let by = as_int(args.first().ok_or_else(|| arg_err("advanced(by:)"))?)?;
    ok(
        SwiftValue::Int(IntValue::new(a.raw + by.raw, a.width)),
        recv,
    )
}

/// The unsigned (`Magnitude`) counterpart of an integer width.
fn magnitude_width(w: IntWidth) -> IntWidth {
    match w {
        IntWidth::I8 | IntWidth::U8 => IntWidth::U8,
        IntWidth::I16 | IntWidth::U16 => IntWidth::U16,
        IntWidth::I32 | IntWidth::U32 => IntWidth::U32,
        IntWidth::I64 | IntWidth::U64 => IntWidth::U64,
    }
}

/// `Int.multipliedFullWidth(by:)` — the exact double-width product split into
/// `(high: Self, low: Magnitude)`. The product of two `w`-bit integers fits in
/// `2w ≤ 128` bits, so an `i128` holds it without loss.
fn int_multiplied_full_width(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    let a = as_int(&recv)?;
    let b = as_int(
        args.first()
            .ok_or_else(|| arg_err("multipliedFullWidth(by:)"))?,
    )?;
    let bits = a.width.bits();
    let full = a.raw * b.raw;
    let high = full >> bits;
    let low = full & ((1i128 << bits) - 1);
    let tuple = SwiftValue::tuple_labeled(
        vec![
            SwiftValue::Int(IntValue::new(high, a.width)),
            SwiftValue::Int(IntValue::new(low, magnitude_width(a.width))),
        ],
        vec![Some("high".to_string()), Some("low".to_string())],
    );
    ok(tuple, recv)
}

/// `Int.dividingFullWidth(_:)` — divide the double-width dividend
/// `(high: Self, low: Magnitude)` by `self`, returning
/// `(quotient: Self, remainder: Self)`. Traps on a zero divisor or a quotient
/// that does not fit in `Self`, matching Swift.
fn int_dividing_full_width(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    let a = as_int(&recv)?;
    let parts = match args.first() {
        Some(SwiftValue::Tuple(t, _)) if t.len() == 2 => t,
        _ => {
            return Err(arg_err("dividingFullWidth(_:)"));
        }
    };
    let high = as_int(&parts[0])?;
    let low = as_int(&parts[1])?;
    if a.raw == 0 {
        return Err(StdError::Error(EvalError::Trap("division by zero".into())));
    }
    let bits = a.width.bits();
    let dividend = (high.raw << bits) + (low.raw & ((1i128 << bits) - 1));
    let quotient = dividend / a.raw;
    let remainder = dividend % a.raw;
    let q = IntValue::new(quotient, a.width);
    if !q.in_range() {
        return Err(StdError::Error(EvalError::Trap(
            "quotient overflows in dividingFullWidth".into(),
        )));
    }
    let tuple = SwiftValue::tuple_labeled(
        vec![
            SwiftValue::Int(q),
            SwiftValue::Int(IntValue::new(remainder, a.width)),
        ],
        vec![Some("quotient".to_string()), Some("remainder".to_string())],
    );
    ok(tuple, recv)
}

/// `Int.words` — the value's words as `UInt`, low-order first. The modelled
/// integers are at most one machine word, so this yields a single element.
fn int_words(recv: SwiftValue) -> StdResult {
    let a = as_int(&recv)?;
    let word = a.raw as u64;
    Ok(SwiftValue::Array(std::rc::Rc::new(vec![SwiftValue::Int(
        IntValue::new(i128::from(word), IntWidth::U64),
    )])))
}

/// `Int.description` — the base-10 textual form.
fn int_description(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Str(as_int(&recv)?.raw.to_string()))
}

/// `Int.hashValue` — a deterministic per-run hash (FNV-1a over the raw bytes).
fn int_hash_value(recv: SwiftValue) -> StdResult {
    let raw = as_int(&recv)?.raw;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in (raw as u64).to_le_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Ok(SwiftValue::int(i128::from(h as i64)))
}

/// Shared body for the `*ReportingOverflow(by:)` methods: apply `op` in wide
/// arithmetic, wrap the result into `self`'s width, and report whether the
/// mathematical result fell outside that width.
fn int_reporting_overflow(
    who: &str,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
    op: impl Fn(i128, i128) -> i128,
) -> Outcomes {
    let a = as_int(&recv)?;
    let b = as_int(args.first().ok_or_else(|| arg_err(who))?)?;
    let wide = op(a.raw, b.raw);
    let partial = IntValue::wrapped(a.width, wide);
    let overflow = wide < a.width.min() || wide > a.width.max();
    let tuple = SwiftValue::tuple_labeled(
        vec![SwiftValue::Int(partial), SwiftValue::Bool(overflow)],
        vec![
            Some("partialValue".to_string()),
            Some("overflow".to_string()),
        ],
    );
    ok(tuple, recv)
}

/// `Int.addingReportingOverflow(_:)` — `(partialValue, overflow)` for `self + other`.
fn int_adding_reporting_overflow(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    int_reporting_overflow("addingReportingOverflow(_:)", recv, args, |a, b| a + b)
}

/// `Int.subtractingReportingOverflow(_:)` — `(partialValue, overflow)` for `self - other`.
fn int_subtracting_reporting_overflow(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    int_reporting_overflow("subtractingReportingOverflow(_:)", recv, args, |a, b| a - b)
}

/// `Int.multipliedReportingOverflow(by:)` — `(partialValue, overflow)` for `self * other`.
fn int_multiplied_reporting_overflow(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    int_reporting_overflow("multipliedReportingOverflow(by:)", recv, args, |a, b| a * b)
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

/// `Double.isSignMinus` — whether the sign bit is set (true for `-0.0`).
fn double_is_sign_minus(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(as_double(&recv)?.is_sign_negative()))
}

/// `Double.description` / `debugDescription` — the textual form `print` uses.
fn double_description(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Str(format_double(as_double(&recv)?)))
}

/// `Double.nextUp` — the least representable value greater than `self`.
fn double_next_up(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Double(as_double(&recv)?.next_up()))
}

/// `Double.ulp` — the unit in the last place: the distance to `nextUp`.
fn double_ulp(recv: SwiftValue) -> StdResult {
    let d = as_double(&recv)?;
    Ok(SwiftValue::Double(d.next_up() - d))
}

/// Decompose a finite, non-zero double into its unbiased `exponent` and the
/// significand magnitude in `[1, 2)`. Scaling by two is exact in binary
/// floating point, so the loop introduces no rounding error.
fn decode_double(x: f64) -> Option<(i128, f64)> {
    if !x.is_finite() || x == 0.0 {
        return None;
    }
    let mut m = x.abs();
    let mut e: i128 = 0;
    while m >= 2.0 {
        m /= 2.0;
        e += 1;
    }
    while m < 1.0 {
        m *= 2.0;
        e -= 1;
    }
    Some((e, m))
}

/// `Double.bitPattern` — the IEEE-754 storage as a `UInt64`.
fn double_bit_pattern(recv: SwiftValue) -> StdResult {
    let bits = as_double(&recv)?.to_bits();
    Ok(SwiftValue::Int(IntValue::new(
        i128::from(bits),
        IntWidth::U64,
    )))
}

/// `Double.exponent` — the unbiased base-2 exponent (`Int.min` for zero,
/// `Int.max` for non-finite values), matching `FloatingPoint.exponent`.
fn double_exponent(recv: SwiftValue) -> StdResult {
    let x = as_double(&recv)?;
    let e = if x == 0.0 {
        i128::from(i64::MIN)
    } else if !x.is_finite() {
        i128::from(i64::MAX)
    } else {
        decode_double(x).unwrap().0
    };
    Ok(SwiftValue::int(e))
}

/// `Double.significand` — the significand magnitude in `[1, 2)` for finite
/// non-zero values; `0`, `inf`, `nan` map to themselves.
fn double_significand(recv: SwiftValue) -> StdResult {
    let x = as_double(&recv)?;
    let s = if x == 0.0 || !x.is_finite() {
        x.abs()
    } else {
        decode_double(x).unwrap().1
    };
    Ok(SwiftValue::Double(s))
}

/// `Double.binade` — the value `sign * 2 ** exponent` (the start of the binade
/// containing `self`).
fn double_binade(recv: SwiftValue) -> StdResult {
    let x = as_double(&recv)?;
    let b = if x == 0.0 || !x.is_finite() {
        x
    } else {
        let (e, _) = decode_double(x).unwrap();
        2f64.powi(e as i32).copysign(x)
    };
    Ok(SwiftValue::Double(b))
}

/// `Double.exponentBitPattern` — the raw biased exponent field (`UInt`).
fn double_exponent_bit_pattern(recv: SwiftValue) -> StdResult {
    let bits = as_double(&recv)?.to_bits();
    let raw = (bits >> 52) & 0x7ff;
    Ok(SwiftValue::Int(IntValue::new(
        i128::from(raw),
        IntWidth::U64,
    )))
}

/// `Double.significandBitPattern` — the raw 52-bit fraction field (`UInt64`).
fn double_significand_bit_pattern(recv: SwiftValue) -> StdResult {
    let bits = as_double(&recv)?.to_bits();
    let frac = bits & 0x000f_ffff_ffff_ffff;
    Ok(SwiftValue::Int(IntValue::new(
        i128::from(frac),
        IntWidth::U64,
    )))
}

/// `Double.sign` — the `FloatingPointSign` of `self`: `.minus` when the sign
/// bit is set (including `-0.0` and negative NaN), otherwise `.plus`.
fn double_sign(recv: SwiftValue) -> StdResult {
    let x = as_double(&recv)?;
    let case = if x.is_sign_negative() {
        "minus"
    } else {
        "plus"
    };
    Ok(SwiftValue::Enum(Rc::new(EnumObj {
        type_name: "FloatingPointSign".into(),
        case: case.into(),
        payload: Vec::new(),
    })))
}

/// `Double.significandWidth` — the number of fractional bits needed to
/// represent the significand (`-1` for zero and non-finite values). Equals the
/// span between the most- and least-significant set bits of the significand
/// magnitude (implicit leading bit included for normal numbers).
fn double_significand_width(recv: SwiftValue) -> StdResult {
    let x = as_double(&recv)?;
    if !x.is_finite() || x == 0.0 {
        return Ok(SwiftValue::int(-1));
    }
    let bits = x.abs().to_bits();
    let exp = (bits >> 52) & 0x7ff;
    let frac = bits & 0x000f_ffff_ffff_ffff;
    // Significand magnitude as an integer: normal numbers carry the implicit
    // leading 1 at bit 52; subnormals are just the raw fraction.
    let m: u64 = if exp == 0 { frac } else { (1u64 << 52) | frac };
    let width = (63 - m.leading_zeros()) - m.trailing_zeros();
    Ok(SwiftValue::int(i128::from(width)))
}

/// `Double.isCanonical` — always true: every `Double` bit pattern is canonical.
fn double_is_canonical(recv: SwiftValue) -> StdResult {
    as_double(&recv)?;
    Ok(SwiftValue::Bool(true))
}

/// `Double.isSignalingNaN` — a NaN with the signaling (quiet) bit cleared.
fn double_is_signaling_nan(recv: SwiftValue) -> StdResult {
    let x = as_double(&recv)?;
    let signaling = x.is_nan() && (x.to_bits() & 0x0008_0000_0000_0000) == 0;
    Ok(SwiftValue::Bool(signaling))
}

/// `Double.hashValue` — a stable digest where equal values (including
/// `0.0`/`-0.0`) hash equally; the per-process seed Swift uses is not modelled.
fn double_hash_value(recv: SwiftValue) -> StdResult {
    let x = as_double(&recv)?;
    let bits = if x == 0.0 { 0u64 } else { x.to_bits() };
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bits.to_le_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Ok(SwiftValue::int(i128::from(h as i64)))
}

/// `Double.negate()` — flip the sign in place (`mutating func negate()`).
fn double_negate(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let d = as_double(&recv)?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Double(-d),
    })
}

/// The first argument as a `Double`, for the binary `Double` methods.
fn arg_double(args: &[SwiftValue], who: &str) -> Result<f64, StdError> {
    as_double(args.first().ok_or_else(|| arg_err(who))?)
}

/// `Double.remainder(dividingBy:)` — the IEEE remainder.
fn double_remainder(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let x = as_double(&recv)?;
    let y = arg_double(&args, "remainder(dividingBy:)")?;
    ok(SwiftValue::Double(ieee_remainder(x, y)), recv)
}

/// `Double.isEqual(to:)` — IEEE equality (false if either side is NaN).
fn double_is_equal(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let x = as_double(&recv)?;
    let y = arg_double(&args, "isEqual(to:)")?;
    ok(SwiftValue::Bool(x == y), recv)
}

/// `Double.isLess(than:)` — IEEE ordered less-than.
fn double_is_less(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let x = as_double(&recv)?;
    let y = arg_double(&args, "isLess(than:)")?;
    ok(SwiftValue::Bool(x < y), recv)
}

/// `Double.isLessThanOrEqualTo(_:)` — IEEE ordered less-than-or-equal.
fn double_is_less_or_equal(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    let x = as_double(&recv)?;
    let y = arg_double(&args, "isLessThanOrEqualTo(_:)")?;
    ok(SwiftValue::Bool(x <= y), recv)
}

/// `Double.advanced(by:)` — `self + amount` (the `Strideable` conformance).
fn double_advanced(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let x = as_double(&recv)?;
    let by = arg_double(&args, "advanced(by:)")?;
    ok(SwiftValue::Double(x + by), recv)
}

/// `Double.distance(to:)` — `other - self` (the `Strideable` conformance).
fn double_distance(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let x = as_double(&recv)?;
    let other = arg_double(&args, "distance(to:)")?;
    ok(SwiftValue::Double(other - x), recv)
}

/// `Double.formSquareRoot()` — replace `self` with its square root.
fn double_form_square_root(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _a: Vec<SwiftValue>,
) -> Outcomes {
    let x = as_double(&recv)?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Double(x.sqrt()),
    })
}

/// `Double.formTruncatingRemainder(dividingBy:)` — in-place `self %= by`.
fn double_form_truncating_remainder(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    let x = as_double(&recv)?;
    let by = arg_double(&args, "formTruncatingRemainder(dividingBy:)")?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Double(x % by),
    })
}

/// `Double.formRemainder(dividingBy:)` — in-place IEEE remainder.
fn double_form_remainder(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    let x = as_double(&recv)?;
    let by = arg_double(&args, "formRemainder(dividingBy:)")?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Double(ieee_remainder(x, by)),
    })
}

/// `Double.round()` — round to the nearest integer (ties away from zero) in place.
fn double_round(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let x = as_double(&recv)?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Double(x.round()),
    })
}

/// `Double.addProduct(_:_:)` — fused-multiply-add accumulate: `self += a * b`.
fn double_add_product(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    let x = as_double(&recv)?;
    let a = as_double(args.first().ok_or_else(|| arg_err("addProduct(_:_:)"))?)?;
    let b = as_double(args.get(1).ok_or_else(|| arg_err("addProduct(_:_:)"))?)?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Double(a.mul_add(b, x)),
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
    fn full_width_multiply_divide_and_words() {
        let mut c = MockCtx;
        // 1000 * 1000 fits in the low word.
        let m =
            int_multiplied_full_width(&mut c, SwiftValue::int(1000), vec![SwiftValue::int(1000)])
                .unwrap()
                .result;
        assert_eq!(
            m,
            SwiftValue::tuple_labeled(
                vec![
                    SwiftValue::int(0),
                    SwiftValue::Int(IntValue::new(1_000_000, IntWidth::U64))
                ],
                vec![Some("high".into()), Some("low".into())],
            )
        );
        // dividingFullWidth reconstructs the dividend and divides.
        let dividend = SwiftValue::tuple_labeled(
            vec![SwiftValue::int(0), SwiftValue::int(100)],
            vec![Some("high".into()), Some("low".into())],
        );
        let d = int_dividing_full_width(&mut c, SwiftValue::int(10), vec![dividend])
            .unwrap()
            .result;
        assert_eq!(
            d,
            SwiftValue::tuple_labeled(
                vec![SwiftValue::int(10), SwiftValue::int(0)],
                vec![Some("quotient".into()), Some("remainder".into())],
            )
        );
        // A zero divisor traps.
        let z = SwiftValue::tuple_labeled(
            vec![SwiftValue::int(0), SwiftValue::int(1)],
            vec![Some("high".into()), Some("low".into())],
        );
        assert!(int_dividing_full_width(&mut c, SwiftValue::int(0), vec![z]).is_err());
        // words sign-extends to a single UInt word.
        assert_eq!(
            int_words(SwiftValue::Int(IntValue::new(-1, IntWidth::I8))).unwrap(),
            SwiftValue::Array(std::rc::Rc::new(vec![SwiftValue::Int(IntValue::new(
                i128::from(u64::MAX),
                IntWidth::U64
            ))]))
        );
    }

    #[test]
    fn divided_reporting_overflow_and_stride() {
        let mut c = MockCtx;
        let d =
            int_divided_reporting_overflow(&mut c, SwiftValue::int(17), vec![SwiftValue::int(5)])
                .unwrap()
                .result;
        assert_eq!(
            d,
            SwiftValue::tuple(vec![SwiftValue::int(3), SwiftValue::Bool(false)])
        );
        // Division by zero reports overflow and keeps the dividend.
        let dz =
            int_divided_reporting_overflow(&mut c, SwiftValue::int(1), vec![SwiftValue::int(0)])
                .unwrap()
                .result;
        assert_eq!(
            dz,
            SwiftValue::tuple(vec![SwiftValue::int(1), SwiftValue::Bool(true)])
        );
        assert_eq!(
            int_distance(&mut c, SwiftValue::int(10), vec![SwiftValue::int(25)])
                .unwrap()
                .result,
            SwiftValue::int(15)
        );
        assert_eq!(
            int_advanced(&mut c, SwiftValue::int(10), vec![SwiftValue::int(7)])
                .unwrap()
                .result,
            SwiftValue::int(17)
        );
    }

    #[test]
    fn description_and_hash() {
        assert_eq!(
            int_description(SwiftValue::int(-42)).unwrap(),
            SwiftValue::Str("-42".into())
        );
        assert_eq!(
            int_hash_value(SwiftValue::int(42)).unwrap(),
            int_hash_value(SwiftValue::int(42)).unwrap()
        );
        assert_ne!(
            int_hash_value(SwiftValue::int(42)).unwrap(),
            int_hash_value(SwiftValue::int(43)).unwrap()
        );
    }

    #[test]
    fn reporting_overflow() {
        let mut c = MockCtx;
        let r =
            int_adding_reporting_overflow(&mut c, SwiftValue::int(10), vec![SwiftValue::int(5)])
                .unwrap()
                .result;
        assert_eq!(
            r,
            SwiftValue::tuple(vec![SwiftValue::int(15), SwiftValue::Bool(false)])
        );
        // Int8 overflow: 100 * 2 = 200 wraps and reports overflow.
        let max = SwiftValue::Int(IntValue::new(100, tswift_core::IntWidth::I8));
        let r = int_multiplied_reporting_overflow(
            &mut c,
            max,
            vec![SwiftValue::Int(IntValue::new(2, tswift_core::IntWidth::I8))],
        )
        .unwrap()
        .result;
        if let SwiftValue::Tuple(elems, _) = r {
            assert_eq!(elems[1], SwiftValue::Bool(true));
        } else {
            panic!("expected a tuple");
        }
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
    fn double_ieee_methods() {
        let mut c = MockCtx;
        assert_eq!(
            double_remainder(
                &mut c,
                SwiftValue::Double(5.0),
                vec![SwiftValue::Double(3.0)]
            )
            .unwrap()
            .result,
            SwiftValue::Double(-1.0)
        );
        assert_eq!(
            double_is_less(
                &mut c,
                SwiftValue::Double(2.0),
                vec![SwiftValue::Double(3.0)]
            )
            .unwrap()
            .result,
            SwiftValue::Bool(true)
        );
        assert_eq!(
            double_distance(
                &mut c,
                SwiftValue::Double(10.0),
                vec![SwiftValue::Double(13.0)]
            )
            .unwrap()
            .result,
            SwiftValue::Double(3.0)
        );
        // Mutating methods write the new value through the receiver.
        assert_eq!(
            double_form_square_root(&mut c, SwiftValue::Double(9.0), vec![])
                .unwrap()
                .receiver,
            SwiftValue::Double(3.0)
        );
        assert_eq!(
            double_add_product(
                &mut c,
                SwiftValue::Double(1.0),
                vec![SwiftValue::Double(2.0), SwiftValue::Double(3.0)]
            )
            .unwrap()
            .receiver,
            SwiftValue::Double(7.0)
        );
        // NaN comparisons are all false (no ordering with NaN).
        assert_eq!(
            double_is_equal(
                &mut c,
                SwiftValue::Double(f64::NAN),
                vec![SwiftValue::Double(f64::NAN)]
            )
            .unwrap()
            .result,
            SwiftValue::Bool(false)
        );
    }

    #[test]
    fn double_bit_decomposition() {
        let d = SwiftValue::Double(3.0);
        assert_eq!(
            double_bit_pattern(d.clone()).unwrap(),
            SwiftValue::Int(IntValue::new(4_613_937_818_241_073_152, IntWidth::U64))
        );
        assert_eq!(double_exponent(d.clone()).unwrap(), SwiftValue::int(1));
        assert_eq!(
            double_significand(d.clone()).unwrap(),
            SwiftValue::Double(1.5)
        );
        assert_eq!(double_binade(d).unwrap(), SwiftValue::Double(2.0));
        // Sub-one and negative values.
        assert_eq!(
            double_exponent(SwiftValue::Double(0.75)).unwrap(),
            SwiftValue::int(-1)
        );
        assert_eq!(
            double_binade(SwiftValue::Double(-3.0)).unwrap(),
            SwiftValue::Double(-2.0)
        );
        // Special-value exponents and the canonical/signaling flags.
        assert_eq!(
            double_exponent(SwiftValue::Double(0.0)).unwrap(),
            SwiftValue::int(i128::from(i64::MIN))
        );
        assert_eq!(
            double_is_canonical(SwiftValue::Double(f64::NAN)).unwrap(),
            SwiftValue::Bool(true)
        );
        assert_eq!(
            double_is_signaling_nan(SwiftValue::Double(f64::NAN)).unwrap(),
            SwiftValue::Bool(false)
        );
        // Equal values (and signed zeros) hash equally.
        assert_eq!(
            double_hash_value(SwiftValue::Double(0.0)).unwrap(),
            double_hash_value(SwiftValue::Double(-0.0)).unwrap()
        );
    }

    #[test]
    fn double_text_and_neighbours() {
        assert_eq!(
            double_description(SwiftValue::Double(3.5)).unwrap(),
            SwiftValue::Str("3.5".into())
        );
        assert_eq!(
            double_is_sign_minus(SwiftValue::Double(-0.0)).unwrap(),
            SwiftValue::Bool(true)
        );
        assert_eq!(
            double_is_sign_minus(SwiftValue::Double(1.0)).unwrap(),
            SwiftValue::Bool(false)
        );
        // nextUp is strictly greater; ulp is the positive gap to it.
        let up = double_next_up(SwiftValue::Double(2.0)).unwrap();
        assert!(matches!(up, SwiftValue::Double(d) if d > 2.0));
        let ulp = double_ulp(SwiftValue::Double(1.0)).unwrap();
        assert!(matches!(ulp, SwiftValue::Double(d) if d > 0.0));
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
