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
    interp.register_builtin_enum("Decimal.RoundingMode", &["plain", "down", "up", "bankers"]);
    interp.register_builtin_enum("FloatingPointSign", &["plus", "minus"]);

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

    interp.register_static(BuiltinReceiver::Decimal, "pi", decimal_pi);
    interp.register_static(BuiltinReceiver::Decimal, "nan", decimal_nan_static);
    interp.register_static(BuiltinReceiver::Decimal, "quietNaN", decimal_nan_static);

    for (name, mutating, func) in [
        ("rounded", false, decimal_rounded as IntrinsicFn),
        ("round", true, decimal_round),
        ("negate", true, decimal_negate),
        ("add", true, decimal_add),
        ("subtract", true, decimal_subtract),
        ("multiply", true, decimal_multiply),
        ("divide", true, decimal_divide),
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
    let magnitude = value.mantissa as f64 * 10_f64.powi(value.exponent);
    Ok(SwiftValue::int(magnitude.to_bits() as i128))
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
    Ok(SwiftValue::Str(
        dec::to_string(decimal_value(&recv)?).into(),
    ))
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
    fn pi_is_thirty_six_digit_constant() {
        // Mirror the literal `decimal_pi` returns; the static wrapper is covered
        // end-to-end by the `foundation_decimal` golden fixture.
        let pi = Dec::new(314159265358979323846264338327950288, -35);
        assert_eq!(dec::to_string(pi), "3.14159265358979323846264338327950288");
    }
}
