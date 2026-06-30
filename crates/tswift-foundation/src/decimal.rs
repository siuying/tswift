//! `Decimal` — exact base-10 number. The arithmetic core lives in
//! `tswift_core::decimal` (so operator dispatch can reach it); this module wires
//! the initializers, properties, and methods onto the builtin registry.

use tswift_core::{
    decimal::{self as dec, Dec, RoundingMode},
    Arg, BuiltinReceiver, Interpreter, IntrinsicFn, MethodEntry, Outcome, StdContext, StdError,
    StdResult, SwiftValue,
};

use crate::type_error;

pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_builtin_enum("Decimal.RoundingMode", &["plain", "down", "up", "bankers"]);

    interp.register_free_fn("Decimal", decimal_init);
    interp.register_property(BuiltinReceiver::Decimal, "isZero", decimal_is_zero);
    interp.register_property(BuiltinReceiver::Decimal, "isNaN", decimal_is_nan);
    interp.register_property(BuiltinReceiver::Decimal, "magnitude", decimal_magnitude);
    interp.register_property(BuiltinReceiver::Decimal, "description", decimal_description);

    for (name, mutating, func) in [
        ("rounded", false, decimal_rounded as IntrinsicFn),
        ("round", true, decimal_round),
    ] {
        interp.register_intrinsic(
            BuiltinReceiver::Decimal,
            name,
            MethodEntry { mutating, func },
        );
    }
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
