//! `Decimal` — exact base-10 number. The arithmetic core lives in
//! `tswift_core::decimal` (so operator dispatch can reach it); this module wires
//! the initializers, properties, and methods onto the builtin registry.

use std::rc::Rc;

use tswift_core::{
    decimal::{self as dec, Dec, RoundingMode},
    Arg, BuiltinReceiver, Interpreter, IntrinsicFn, MethodEntry, Outcome, StdContext, StdError,
    StdResult, SwiftValue,
};

use crate::type_error;

pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_builtin_enum_with_raw(
        "Decimal.RoundingMode",
        &[("plain", 0), ("down", 1), ("up", 2), ("bankers", 3)],
    );
    interp.register_builtin_enum_with_raw("FloatingPointSign", &[("plus", 0), ("minus", 1)]);
    interp.register_builtin_enum(
        "FloatingPointClassification",
        &[
            "quietNaN",
            "signalingNaN",
            "negativeInfinity",
            "negativeNormal",
            "negativeSubnormal",
            "negativeZero",
            "positiveZero",
            "positiveSubnormal",
            "positiveNormal",
            "positiveInfinity",
        ],
    );

    interp.register_free_fn("Decimal", decimal_init);
    interp.register_property(BuiltinReceiver::Decimal, "isZero", decimal_is_zero);
    interp.register_property(BuiltinReceiver::Decimal, "isNaN", decimal_is_nan);
    interp.register_property(BuiltinReceiver::Decimal, "magnitude", decimal_magnitude);
    interp.register_property(BuiltinReceiver::Decimal, "description", decimal_description);
    interp.register_property(BuiltinReceiver::Decimal, "exponent", decimal_exponent);
    interp.register_property(BuiltinReceiver::Decimal, "significand", decimal_significand);
    interp.register_property(BuiltinReceiver::Decimal, "sign", decimal_sign);
    interp.register_property(BuiltinReceiver::Decimal, "isFinite", decimal_is_finite);
    interp.register_property(BuiltinReceiver::Decimal, "isInfinite", decimal_is_infinite);
    interp.register_property(
        BuiltinReceiver::Decimal,
        "isSignMinus",
        decimal_is_sign_minus,
    );
    interp.register_property(BuiltinReceiver::Decimal, "hashValue", decimal_hash_value);
    interp.register_property(BuiltinReceiver::Decimal, "isNormal", decimal_is_normal);
    interp.register_property(
        BuiltinReceiver::Decimal,
        "isSubnormal",
        decimal_is_subnormal,
    );
    interp.register_property(
        BuiltinReceiver::Decimal,
        "isCanonical",
        decimal_is_canonical,
    );
    interp.register_property(
        BuiltinReceiver::Decimal,
        "isSignaling",
        decimal_is_false_pred,
    );
    interp.register_property(
        BuiltinReceiver::Decimal,
        "isSignalingNaN",
        decimal_is_false_pred,
    );
    interp.register_property(BuiltinReceiver::Decimal, "ulp", decimal_ulp);
    interp.register_property(BuiltinReceiver::Decimal, "nextUp", decimal_next_up);
    interp.register_property(BuiltinReceiver::Decimal, "nextDown", decimal_next_down);
    interp.register_property(
        BuiltinReceiver::Decimal,
        "floatingPointClass",
        decimal_floating_point_class,
    );
    // formatted() is a no-arg method call in Swift (not a computed property).

    interp.register_static(BuiltinReceiver::Decimal, "pi", decimal_pi);
    interp.register_static(BuiltinReceiver::Decimal, "nan", decimal_nan_static);
    interp.register_static(BuiltinReceiver::Decimal, "quietNaN", decimal_nan_static);
    interp.register_static(BuiltinReceiver::Decimal, "radix", decimal_radix);
    interp.register_static(
        BuiltinReceiver::Decimal,
        "greatestFiniteMagnitude",
        decimal_greatest_finite_magnitude,
    );
    interp.register_static(
        BuiltinReceiver::Decimal,
        "leastFiniteMagnitude",
        decimal_least_finite_magnitude,
    );
    interp.register_static(
        BuiltinReceiver::Decimal,
        "leastNonzeroMagnitude",
        decimal_least_nonzero_magnitude,
    );
    interp.register_static(
        BuiltinReceiver::Decimal,
        "leastNormalMagnitude",
        decimal_least_nonzero_magnitude,
    );

    for (name, mutating, func) in [
        ("rounded", false, decimal_rounded as IntrinsicFn),
        ("round", true, decimal_round),
        ("negate", true, decimal_negate),
        ("add", true, decimal_add),
        ("subtract", true, decimal_subtract),
        ("multiply", true, decimal_multiply),
        ("divide", true, decimal_divide),
        ("advanced", false, decimal_advanced),
        ("distance", false, decimal_distance),
        ("isEqual", false, decimal_is_equal_to),
        ("isLess", false, decimal_is_less),
        ("isLessThanOrEqualTo", false, decimal_is_less_or_equal),
        ("isTotallyOrdered", false, decimal_is_totally_ordered),
        ("formatted", false, decimal_formatted_method),
    ] {
        interp.register_intrinsic(
            BuiltinReceiver::Decimal,
            name,
            MethodEntry { mutating, func },
        );
    }
}

fn floating_point_sign(minus: bool) -> SwiftValue {
    SwiftValue::Enum(Rc::new(tswift_core::EnumObj {
        type_name: "FloatingPointSign".into(),
        case: if minus { "minus" } else { "plus" }.into(),
        payload: Vec::new(),
    }))
}

fn decimal_exponent(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(decimal_value(&recv)?.exponent as i128))
}

fn decimal_significand(recv: SwiftValue) -> StdResult {
    let value = decimal_value(&recv)?;
    if value.nan {
        return Ok(dec::to_value(Dec::NAN));
    }
    Ok(dec::to_value(Dec::new(value.mantissa, 0)))
}

fn decimal_sign(recv: SwiftValue) -> StdResult {
    Ok(floating_point_sign(decimal_value(&recv)?.mantissa < 0))
}

fn decimal_is_finite(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(!decimal_value(&recv)?.nan))
}

fn decimal_is_infinite(recv: SwiftValue) -> StdResult {
    // `Decimal` has no infinity; the value is always finite-or-NaN.
    decimal_value(&recv)?;
    Ok(SwiftValue::Bool(false))
}

fn decimal_is_sign_minus(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(decimal_value(&recv)?.mantissa < 0))
}

fn decimal_hash_value(recv: SwiftValue) -> StdResult {
    let value = decimal_value(&recv)?;
    // Hashable requires equal values to hash equally. `==` (core `decimal::compare`)
    // compares the real magnitude `mantissa * 10^exponent`, falling back to f64
    // when exponent alignment overflows i128. Hashing that same magnitude keeps
    // the hash consistent with `==` across both regimes (extra collisions are
    // acceptable for Hashable). NaN never equals itself, so it gets a fixed hash.
    if value.nan {
        return Ok(SwiftValue::int(0));
    }
    let magnitude = value.mantissa as f64 * 10_f64.powi(value.exponent) + 0.0;
    // Narrow through i64 to stay within the platform `Int` width.
    Ok(SwiftValue::int((magnitude.to_bits() as i64) as i128))
}

fn decimal_pi(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    // 3.14159265358979323846264338327950288 (36 significant digits, fits i128).
    Ok(dec::to_value(Dec::new(
        314159265358979323846264338327950288,
        -35,
    )))
}

fn decimal_nan_static(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(dec::to_value(Dec::NAN))
}

fn decimal_is_normal(recv: SwiftValue) -> StdResult {
    let value = decimal_value(&recv)?;
    Ok(SwiftValue::Bool(!value.nan && !value.is_zero()))
}

fn decimal_is_subnormal(recv: SwiftValue) -> StdResult {
    // A base-10 fixed-point value is never subnormal.
    decimal_value(&recv)?;
    Ok(SwiftValue::Bool(false))
}

fn decimal_is_canonical(recv: SwiftValue) -> StdResult {
    // Values are normalized on construction, so every Decimal is canonical.
    decimal_value(&recv)?;
    Ok(SwiftValue::Bool(true))
}

fn decimal_is_false_pred(recv: SwiftValue) -> StdResult {
    // `Decimal` has no signaling representation.
    decimal_value(&recv)?;
    Ok(SwiftValue::Bool(false))
}

fn decimal_advanced(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let by = decimal_operand(&args, "advanced(by:)")?;
    let value = decimal_value(&recv)?;
    Ok(Outcome {
        result: dec::to_value(dec::add(value, by)),
        receiver: recv,
    })
}

fn decimal_distance(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let other = decimal_operand(&args, "distance(to:)")?;
    let value = decimal_value(&recv)?;
    Ok(Outcome {
        result: dec::to_value(dec::sub(other, value)),
        receiver: recv,
    })
}

fn decimal_compare_pred(
    recv: SwiftValue,
    args: Vec<SwiftValue>,
    method: &str,
    want: &[std::cmp::Ordering],
) -> Result<Outcome, StdError> {
    let other = decimal_operand(&args, method)?;
    let value = decimal_value(&recv)?;
    let result = if value.nan || other.nan {
        false
    } else {
        want.contains(&dec::compare(value, other))
    };
    Ok(Outcome {
        result: SwiftValue::Bool(result),
        receiver: recv,
    })
}

fn decimal_is_equal_to(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    decimal_compare_pred(recv, args, "isEqual(to:)", &[std::cmp::Ordering::Equal])
}

fn decimal_is_less(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    decimal_compare_pred(recv, args, "isLess(than:)", &[std::cmp::Ordering::Less])
}

fn decimal_is_less_or_equal(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    decimal_compare_pred(
        recv,
        args,
        "isLessThanOrEqualTo(_:)",
        &[std::cmp::Ordering::Less, std::cmp::Ordering::Equal],
    )
}

fn decimal_negate(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !args.is_empty() {
        return Err(type_error("Decimal.negate takes no arguments"));
    }
    let value = decimal_value(&recv)?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: dec::to_value(value.negated()),
    })
}

fn decimal_operand(args: &[SwiftValue], method: &str) -> Result<Dec, StdError> {
    match args {
        [arg] => dec::coerce(arg).ok_or_else(|| {
            type_error(format!(
                "Decimal.{method} expects a Decimal operand, got {}",
                arg.type_name()
            ))
        }),
        _ => Err(type_error(format!("Decimal.{method} expects one operand"))),
    }
}

fn decimal_binary_mutating(
    recv: SwiftValue,
    args: Vec<SwiftValue>,
    method: &str,
    op: impl Fn(Dec, Dec) -> Dec,
) -> Result<Outcome, StdError> {
    let lhs = decimal_value(&recv)?;
    let rhs = decimal_operand(&args, method)?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: dec::to_value(op(lhs, rhs)),
    })
}

fn decimal_add(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    decimal_binary_mutating(recv, args, "add", dec::add)
}

fn decimal_subtract(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    decimal_binary_mutating(recv, args, "subtract", dec::sub)
}

fn decimal_multiply(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    decimal_binary_mutating(recv, args, "multiply", dec::mul)
}

fn decimal_divide(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    decimal_binary_mutating(recv, args, "divide", dec::div)
}

fn decimal_value(value: &SwiftValue) -> Result<Dec, StdError> {
    dec::from_value(value)
        .ok_or_else(|| type_error(format!("expected Decimal, got {}", value.type_name())))
}

fn decimal_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    match args.as_slice() {
        [] => Ok(dec::to_value(Dec::zero())),
        [arg] if arg.label.is_none() => match &arg.value {
            SwiftValue::Int(i) => Ok(dec::to_value(Dec::new(i.raw, 0))),
            SwiftValue::Double(d) => match dec::parse(&format!("{d}")) {
                Some(v) => Ok(dec::to_value(v)),
                None => Err(type_error("Decimal(_:) could not represent Double")),
            },
            other => Err(type_error(format!(
                "Decimal(_:) expects Int or Double, got {}",
                other.type_name()
            ))),
        },
        [arg] if arg.label.as_deref() == Some("string") => match &arg.value {
            SwiftValue::Str(s) => Ok(match dec::parse(s) {
                // `init?(string:)`: bare value on success, nil on failure.
                Some(v) => dec::to_value(v),
                None => SwiftValue::Nil,
            }),
            other => Err(type_error(format!(
                "Decimal(string:) expects String, got {}",
                other.type_name()
            ))),
        },
        _ => Err(type_error("unsupported Decimal initializer arguments")),
    }
}

fn decimal_is_zero(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(decimal_value(&recv)?.is_zero()))
}

fn decimal_is_nan(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(decimal_value(&recv)?.nan))
}

fn decimal_magnitude(recv: SwiftValue) -> StdResult {
    Ok(dec::to_value(decimal_value(&recv)?.magnitude()))
}

fn decimal_description(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Str(dec::to_string(decimal_value(&recv)?)))
}

fn rounding_mode(value: &SwiftValue) -> Result<RoundingMode, StdError> {
    let name = match value {
        SwiftValue::Enum(obj) => obj.case.clone(),
        SwiftValue::Str(s) => s.to_string(),
        other => {
            return Err(type_error(format!(
                "expected Decimal.RoundingMode, got {}",
                other.type_name()
            )))
        }
    };
    RoundingMode::from_name(&name)
        .ok_or_else(|| type_error(format!("unsupported rounding mode `{name}`")))
}

fn scale_arg(value: &SwiftValue) -> Result<i32, StdError> {
    match value {
        SwiftValue::Int(i) => Ok(i.raw as i32),
        other => Err(type_error(format!(
            "rounding scale expects Int, got {}",
            other.type_name()
        ))),
    }
}

/// `rounded(_ scale: Int, _ mode: Decimal.RoundingMode) -> Decimal`. The mode
/// defaults to `.plain` when omitted.
fn decimal_rounded(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (scale, mode) = parse_round_args(&args)?;
    let value = decimal_value(&recv)?;
    Ok(Outcome {
        result: dec::to_value(dec::rounded(value, scale, mode)),
        receiver: recv,
    })
}

fn decimal_round(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (scale, mode) = parse_round_args(&args)?;
    let value = decimal_value(&recv)?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: dec::to_value(dec::rounded(value, scale, mode)),
    })
}

fn parse_round_args(args: &[SwiftValue]) -> Result<(i32, RoundingMode), StdError> {
    match args {
        [scale] => Ok((scale_arg(scale)?, RoundingMode::Plain)),
        [scale, mode] => Ok((scale_arg(scale)?, rounding_mode(mode)?)),
        _ => Err(type_error(
            "Decimal.rounded expects (scale) or (scale, roundingMode)",
        )),
    }
}

// ── Static constants ────────────────────────────────────────────────────────

fn decimal_radix(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(SwiftValue::int(10))
}

/// Largest finite `Decimal` in this runtime's `i128` mantissa model.
///
/// Foundation's real `greatestFiniteMagnitude` is `(2^128 − 1) × 10^127`
/// (mantissa = 340282366920938463463374607431768211455, 39 digits).
/// Our signed `i128` can only hold up to `i128::MAX = 2^127 − 1` as the
/// positive mantissa (≈ 1.70×10^38 vs NSDecimal's ≈ 3.40×10^38), so our
/// value is approximately *half* of the real `greatestFiniteMagnitude`.
/// The description string therefore differs from Apple's Foundation.
/// This deviation is documented in `notes.md`.
fn decimal_greatest_finite_magnitude(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(dec::to_value(Dec {
        nan: false,
        mantissa: i128::MAX,
        exponent: 127,
    }))
}

/// Most negative `Decimal` in this runtime (mirror of `greatestFiniteMagnitude`).
/// Same signed-mantissa limitation applies; see `decimal_greatest_finite_magnitude`.
fn decimal_least_finite_magnitude(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(dec::to_value(Dec {
        nan: false,
        mantissa: i128::MIN,
        exponent: 127,
    }))
}

/// Smallest positive non-zero `Decimal` that matches Foundation's value.
///
/// Foundation's `leastNonzeroMagnitude` == `leastNormalMagnitude` == `10^(-127)`,
/// even though smaller values (e.g. `1e-128`) are representable in NSDecimal.
/// The value `10^(-127)` is what `Decimal.leastNonzeroMagnitude` returns in
/// real Swift/Foundation; we match it exactly.
fn decimal_least_nonzero_magnitude(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(dec::to_value(Dec {
        nan: false,
        mantissa: 1,
        exponent: -127,
    }))
}

// ── ULP / neighbours ────────────────────────────────────────────────────────

/// Unit in the last place, matching NSDecimal's 38-significant-digit model.
///
/// Formula: `ulp(x) = 10^(max(floor(log10(|x|)) - 38, -128))` where
/// `floor(log10(|x|))` for a normalised `m * 10^e` is `e + num_digits(|m|) - 1`.
/// The floor at `-128` reflects NSDecimal's minimum representable exponent.
fn decimal_ulp(recv: SwiftValue) -> StdResult {
    let v = decimal_value(&recv)?;
    if v.nan || v.is_zero() {
        return Ok(dec::to_value(Dec::NAN));
    }
    let num_digits = v.mantissa.unsigned_abs().to_string().len() as i32;
    let mag_exp = v.exponent + num_digits - 1; // floor(log10(|v|))
    let ulp_exp = (mag_exp - 38).max(-128);
    Ok(dec::to_value(Dec {
        nan: false,
        mantissa: 1,
        exponent: ulp_exp,
    }))
}

/// Compute the ulp for a non-NaN, non-zero `Dec`.
/// See `decimal_ulp` for the formula.
fn dec_ulp(v: Dec) -> Dec {
    let num_digits = v.mantissa.unsigned_abs().to_string().len() as i32;
    let mag_exp = v.exponent + num_digits - 1;
    let ulp_exp = (mag_exp - 38).max(-128);
    Dec {
        nan: false,
        mantissa: 1,
        exponent: ulp_exp,
    }
}

/// Next representable value above `self` (= self + ulp(self)). NaN → NaN.
fn decimal_next_up(recv: SwiftValue) -> StdResult {
    let v = decimal_value(&recv)?;
    if v.nan || v.is_zero() {
        return Ok(dec::to_value(Dec::NAN));
    }
    Ok(dec::to_value(dec::add(v, dec_ulp(v))))
}

/// Next representable value below `self` (= self - ulp(self)). NaN → NaN.
fn decimal_next_down(recv: SwiftValue) -> StdResult {
    let v = decimal_value(&recv)?;
    if v.nan || v.is_zero() {
        return Ok(dec::to_value(Dec::NAN));
    }
    Ok(dec::to_value(dec::sub(v, dec_ulp(v))))
}

// ── isTotallyOrdered ────────────────────────────────────────────────────────

/// Total ordering predicate — matches Foundation.Decimal behaviour.
///
/// Foundation's `Decimal.isTotallyOrdered(belowOrEqualTo:)` returns `false`
/// whenever **either** operand is NaN (verified against real Swift 5.9+).
/// This deviates from the pure IEEE 754-2008 total-order spec (which places
/// NaN at a defined position) but is what the runtime must match.
fn decimal_is_totally_ordered(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let other = decimal_operand(&args, "isTotallyOrdered(belowOrEqualTo:)")?;
    let value = decimal_value(&recv)?;
    let result = if value.nan || other.nan {
        false // Foundation returns false for any NaN operand
    } else {
        matches!(
            dec::compare(value, other),
            std::cmp::Ordering::Less | std::cmp::Ordering::Equal
        )
    };
    Ok(Outcome {
        result: SwiftValue::Bool(result),
        receiver: recv,
    })
}

// ── floatingPointClass ───────────────────────────────────────────────────────

fn floating_point_classification(case: &str) -> SwiftValue {
    SwiftValue::Enum(std::rc::Rc::new(tswift_core::EnumObj {
        type_name: "FloatingPointClassification".into(),
        case: case.into(),
        payload: Vec::new(),
    }))
}

/// Returns the `FloatingPointClassification` of this `Decimal`.
///
/// `Decimal` has no infinity and no subnormals, so only four cases arise:
/// `.quietNaN`, `.positiveZero`, `.positiveNormal`, `.negativeNormal`.
fn decimal_floating_point_class(recv: SwiftValue) -> StdResult {
    let v = decimal_value(&recv)?;
    let case = if v.nan {
        "quietNaN"
    } else if v.mantissa == 0 {
        "positiveZero"
    } else if v.mantissa > 0 {
        "positiveNormal"
    } else {
        "negativeNormal"
    };
    Ok(floating_point_classification(case))
}

// ── formatted() ─────────────────────────────────────────────────────────────

/// Basic en_US number formatting: integer part with `,` grouping, decimal
/// separator `.`, fractional digits preserved as-is.
///
/// Approximates `Decimal.formatted()` with the default number format style.
/// Full `FormatStyle` locale sensitivity is out of scope for this runtime.
fn decimal_format_to_string(v: Dec) -> String {
    if v.nan {
        return "NaN".to_string();
    }
    let s = dec::to_string(v);
    let (negative, body) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else {
        (false, s.as_str())
    };
    let (int_part, frac_part) = match body.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (body, None),
    };
    let grouped = group_with_commas(int_part);
    let mut result = if negative {
        format!("-{grouped}")
    } else {
        grouped
    };
    if let Some(frac) = frac_part {
        result.push('.');
        result.push_str(frac);
    }
    result
}

/// `formatted()` intrinsic wrapper (no-arg method call form).
fn decimal_formatted_method(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !args.is_empty() {
        return Err(type_error("Decimal.formatted() takes no arguments"));
    }
    let v = decimal_value(&recv)?;
    Ok(Outcome {
        result: SwiftValue::Str(decimal_format_to_string(v)),
        receiver: recv,
    })
}

/// Add thousand-group commas to a non-negative integer digit string.
fn group_with_commas(digits: &str) -> String {
    let chars: Vec<char> = digits.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(len + len / 3);
    for (idx, ch) in chars.iter().enumerate() {
        if idx > 0 && (len - idx).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decimal(mantissa: i128, exponent: i32) -> SwiftValue {
        dec::to_value(Dec::new(mantissa, exponent))
    }

    #[test]
    fn exponent_reports_internal_scale() {
        // 1.5 == 15 * 10^-1.
        assert_eq!(
            decimal_exponent(decimal(15, -1)).unwrap(),
            SwiftValue::int(-1)
        );
    }

    #[test]
    fn significand_drops_exponent() {
        let sig = decimal_significand(decimal(15, -1)).unwrap();
        assert_eq!(sig, dec::to_value(Dec::new(15, 0)));
    }

    #[test]
    fn sign_distinguishes_negative_from_nonnegative() {
        assert_eq!(
            decimal_sign(decimal(-5, 0)).unwrap(),
            floating_point_sign(true)
        );
        assert_eq!(
            decimal_sign(decimal(5, 0)).unwrap(),
            floating_point_sign(false)
        );
        assert_eq!(
            decimal_sign(decimal(0, 0)).unwrap(),
            floating_point_sign(false)
        );
    }

    #[test]
    fn finiteness_predicates() {
        assert_eq!(
            decimal_is_finite(decimal(5, 0)).unwrap(),
            SwiftValue::Bool(true)
        );
        assert_eq!(
            decimal_is_infinite(decimal(5, 0)).unwrap(),
            SwiftValue::Bool(false)
        );
        assert_eq!(
            decimal_is_finite(dec::to_value(Dec::NAN)).unwrap(),
            SwiftValue::Bool(false)
        );
    }

    #[test]
    fn sign_minus_tracks_mantissa() {
        assert_eq!(
            decimal_is_sign_minus(decimal(-1, 0)).unwrap(),
            SwiftValue::Bool(true)
        );
        assert_eq!(
            decimal_is_sign_minus(decimal(1, 0)).unwrap(),
            SwiftValue::Bool(false)
        );
    }

    #[test]
    fn equal_decimals_hash_equal() {
        // 0.30 and 0.3 normalize to the same value.
        let a = decimal_hash_value(decimal(30, -2)).unwrap();
        let b = decimal_hash_value(decimal(3, -1)).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn hash_matches_magnitude_and_handles_nan() {
        // Hash is the bit pattern of the real magnitude, so any two values that
        // map to the same magnitude (the basis of `==`) hash equally.
        let mag = 1.5_f64;
        assert_eq!(
            decimal_hash_value(decimal(15, -1)).unwrap(),
            SwiftValue::int(mag.to_bits() as i128)
        );
        // NaN never equals itself; it hashes to a fixed sentinel.
        assert_eq!(
            decimal_hash_value(dec::to_value(Dec::NAN)).unwrap(),
            SwiftValue::int(0)
        );
    }

    #[test]
    fn negate_rejects_arguments() {
        let err = decimal_negate(&mut dummy_ctx(), decimal(5, 0), vec![decimal(1, 0)]);
        assert!(err.is_err());
    }

    fn dummy_ctx() -> impl StdContext {
        struct Ctx(Vec<u8>);
        impl StdContext for Ctx {
            fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> StdResult {
                unreachable!("decimal helpers never call closures")
            }
            fn out(&mut self) -> &mut dyn std::io::Write {
                &mut self.0
            }
        }
        Ctx(Vec::new())
    }

    #[test]
    fn strideable_advance_and_distance() {
        let three = decimal(3, 0);
        let advanced = decimal_advanced(&mut dummy_ctx(), three.clone(), vec![decimal(2, 0)])
            .unwrap()
            .result;
        assert_eq!(advanced, decimal(5, 0));
        let distance = decimal_distance(&mut dummy_ctx(), three, vec![decimal(5, 0)])
            .unwrap()
            .result;
        assert_eq!(distance, decimal(2, 0));
        // Distance is signed: 5.distance(to: 3) == -2.
        let back = decimal_distance(&mut dummy_ctx(), decimal(5, 0), vec![decimal(3, 0)])
            .unwrap()
            .result;
        assert_eq!(back, decimal(-2, 0));
    }

    #[test]
    fn floating_point_predicates() {
        assert_eq!(
            decimal_is_normal(decimal(5, 0)).unwrap(),
            SwiftValue::Bool(true)
        );
        assert_eq!(
            decimal_is_normal(decimal(0, 0)).unwrap(),
            SwiftValue::Bool(false)
        );
        assert_eq!(
            decimal_is_canonical(decimal(5, 0)).unwrap(),
            SwiftValue::Bool(true)
        );
        // isEqual ignores the (stripped) label and compares the operand.
        let eq = decimal_is_equal_to(&mut dummy_ctx(), decimal(3, 0), vec![decimal(3, 0)])
            .unwrap()
            .result;
        assert_eq!(eq, SwiftValue::Bool(true));
        // NaN is never equal, less, or less-than-or-equal.
        for pred in [
            decimal_is_equal_to as fn(&mut dyn StdContext, SwiftValue, Vec<SwiftValue>) -> _,
            decimal_is_less,
            decimal_is_less_or_equal,
        ] {
            let out = pred(
                &mut dummy_ctx(),
                dec::to_value(Dec::NAN),
                vec![decimal(3, 0)],
            )
            .unwrap()
            .result;
            assert_eq!(out, SwiftValue::Bool(false));
        }
    }

    #[test]
    fn static_constants_are_well_formed() {
        let greatest = decimal_greatest_finite_magnitude(&mut dummy_ctx(), vec![]).unwrap();
        let least = decimal_least_finite_magnitude(&mut dummy_ctx(), vec![]).unwrap();
        let least_nonzero = decimal_least_nonzero_magnitude(&mut dummy_ctx(), vec![]).unwrap();
        let radix = decimal_radix(&mut dummy_ctx(), vec![]).unwrap();
        assert_eq!(radix, SwiftValue::int(10));
        // greatestFiniteMagnitude > 0, uses i128::MAX mantissa (documented deviation from NSDecimal)
        let gfm = dec::from_value(&greatest).unwrap();
        assert!(gfm.mantissa > 0);
        assert_eq!(gfm.exponent, 127);
        assert_eq!(gfm.mantissa, i128::MAX);
        // leastFiniteMagnitude < 0
        let lfm = dec::from_value(&least).unwrap();
        assert!(lfm.mantissa < 0);
        // leastNonzeroMagnitude matches Foundation's real value: 10^(-127)
        let lnzm = dec::from_value(&least_nonzero).unwrap();
        assert_eq!(lnzm.mantissa, 1);
        assert_eq!(lnzm.exponent, -127); // Foundation returns 10^-127, not 10^-128
        assert_eq!(dec::to_string(lnzm), "0.0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001");
    }

    #[test]
    fn ulp_uses_38_significant_digit_model() {
        // ulp(1) = 10^(floor(log10(1)) - 38) = 10^(0 - 38) = 10^-38
        let ulp1 = decimal_ulp(decimal(1, 0)).unwrap();
        assert_eq!(
            dec::to_string(dec::from_value(&ulp1).unwrap()),
            "0.00000000000000000000000000000000000001"
        );
        // ulp(1.5): floor(log10(1.5)) = 0, ulp = 10^-38 (same as ulp(1))
        let ulp15 = decimal_ulp(decimal(15, -1)).unwrap();
        assert_eq!(ulp15, ulp1);
        // ulp(0.01): floor(log10(0.01)) = -2, ulp = 10^(-2-38) = 10^-40
        let ulp001 = decimal_ulp(decimal(1, -2)).unwrap();
        assert_eq!(
            dec::to_string(dec::from_value(&ulp001).unwrap()),
            "0.0000000000000000000000000000000000000001"
        );
        // ulp(lnzm = 10^-127): floor(log10(10^-127)) = -127, ulp = max(-165, -128) = 10^-128
        let lnzm = dec::to_value(Dec {
            nan: false,
            mantissa: 1,
            exponent: -127,
        });
        let ulp_lnzm = decimal_ulp(lnzm).unwrap();
        assert_eq!(dec::from_value(&ulp_lnzm).unwrap().exponent, -128);
        // NaN → NaN
        assert!(
            dec::from_value(&decimal_ulp(dec::to_value(Dec::NAN)).unwrap())
                .unwrap()
                .nan
        );
    }

    #[test]
    fn next_up_and_next_down() {
        // nextUp(1.5) = 1.5 + 10^-38 = 1.50000000000000000000000000000000000001
        let up = decimal_next_up(decimal(15, -1)).unwrap();
        assert_eq!(
            dec::to_string(dec::from_value(&up).unwrap()),
            "1.50000000000000000000000000000000000001"
        );
        // nextDown(1.5) = 1.5 - 10^-38 = 1.49999999999999999999999999999999999999
        let down = decimal_next_down(decimal(15, -1)).unwrap();
        assert_eq!(
            dec::to_string(dec::from_value(&down).unwrap()),
            "1.49999999999999999999999999999999999999"
        );
        // NaN propagates
        assert!(
            dec::from_value(&decimal_next_up(dec::to_value(Dec::NAN)).unwrap())
                .unwrap()
                .nan
        );
    }

    #[test]
    fn is_totally_ordered_predicate() {
        let r = |recv, arg| {
            decimal_is_totally_ordered(&mut dummy_ctx(), recv, vec![arg])
                .unwrap()
                .result
        };
        assert_eq!(r(decimal(3, 0), decimal(5, 0)), SwiftValue::Bool(true));
        assert_eq!(r(decimal(5, 0), decimal(3, 0)), SwiftValue::Bool(false));
        assert_eq!(r(decimal(3, 0), decimal(3, 0)), SwiftValue::Bool(true));
        // Foundation: any NaN operand → false (verified against real Swift)
        assert_eq!(
            r(dec::to_value(Dec::NAN), decimal(5, 0)),
            SwiftValue::Bool(false)
        );
        assert_eq!(
            r(decimal(5, 0), dec::to_value(Dec::NAN)),
            SwiftValue::Bool(false)
        );
        // NaN vs NaN → false
        assert_eq!(
            r(dec::to_value(Dec::NAN), dec::to_value(Dec::NAN)),
            SwiftValue::Bool(false)
        );
    }

    #[test]
    fn floating_point_class_cases() {
        assert_eq!(
            decimal_floating_point_class(decimal(5, 0)).unwrap(),
            floating_point_classification("positiveNormal")
        );
        assert_eq!(
            decimal_floating_point_class(decimal(-5, 0)).unwrap(),
            floating_point_classification("negativeNormal")
        );
        assert_eq!(
            decimal_floating_point_class(decimal(0, 0)).unwrap(),
            floating_point_classification("positiveZero")
        );
        assert_eq!(
            decimal_floating_point_class(dec::to_value(Dec::NAN)).unwrap(),
            floating_point_classification("quietNaN")
        );
    }

    #[test]
    fn formatted_adds_thousand_separators() {
        fn fmt(v: Dec) -> String {
            decimal_format_to_string(v)
        }
        assert_eq!(fmt(Dec::new(1234567, 0)), "1,234,567");
        assert_eq!(fmt(dec::parse("1234.56").unwrap()), "1,234.56");
        assert_eq!(fmt(Dec::new(-1234, 0)), "-1,234");
        assert_eq!(fmt(Dec::zero()), "0");
        assert_eq!(fmt(Dec::new(999, 0)), "999");
        assert_eq!(fmt(Dec::NAN), "NaN");
    }

    #[test]
    fn group_with_commas_boundaries() {
        assert_eq!(group_with_commas("1"), "1");
        assert_eq!(group_with_commas("100"), "100");
        assert_eq!(group_with_commas("1000"), "1,000");
        assert_eq!(group_with_commas("1234567"), "1,234,567");
    }

    #[test]
    fn pi_is_thirty_six_digit_constant() {
        // Mirror the literal `decimal_pi` returns; the static wrapper is covered
        // end-to-end by the `foundation_decimal` golden fixture.
        let pi = Dec::new(314159265358979323846264338327950288, -35);
        assert_eq!(dec::to_string(pi), "3.14159265358979323846264338327950288");
    }
}
