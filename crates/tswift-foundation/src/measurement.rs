//! `Measurement` + the common `Unit` dimensions.
//!
//! Each unit stores its linear/affine conversion to the dimension's base unit
//! (`base = value * _coefficient + _constant`), so `converted(to:)` is pure
//! arithmetic. Operators live in `tswift_core::ops` (operator dispatch is a core
//! concern); this module wires initializers, properties, and methods.

use std::rc::Rc;

use tswift_core::{
    format_double, Arg, BuiltinReceiver, Interpreter, IntrinsicFn, MethodEntry, Outcome,
    StdContext, StdError, StdResult, StructObj, SwiftValue,
};

use crate::type_error;

/// `(type, [(case, symbol, coefficient, constant)])` for every supported unit.
const UNITS: &[(&str, &[(&str, &str, f64, f64)])] = &[
    (
        "UnitLength",
        &[
            ("meters", "m", 1.0, 0.0),
            ("kilometers", "km", 1000.0, 0.0),
            ("centimeters", "cm", 0.01, 0.0),
            ("millimeters", "mm", 0.001, 0.0),
            ("miles", "mi", 1609.344, 0.0),
            ("feet", "ft", 0.3048, 0.0),
            ("inches", "in", 0.0254, 0.0),
            ("yards", "yd", 0.9144, 0.0),
        ],
    ),
    (
        "UnitMass",
        &[
            ("kilograms", "kg", 1.0, 0.0),
            ("grams", "g", 0.001, 0.0),
            ("milligrams", "mg", 1e-6, 0.0),
            ("pounds", "lb", 0.453_592_37, 0.0),
            ("ounces", "oz", 0.028_349_523_125, 0.0),
        ],
    ),
    (
        "UnitDuration",
        &[
            ("seconds", "s", 1.0, 0.0),
            ("minutes", "min", 60.0, 0.0),
            ("hours", "hr", 3600.0, 0.0),
        ],
    ),
    (
        "UnitTemperature",
        &[
            ("kelvin", "K", 1.0, 0.0),
            ("celsius", "\u{00B0}C", 1.0, 273.15),
            ("fahrenheit", "\u{00B0}F", 5.0 / 9.0, 255.372_222_222_222_2),
        ],
    ),
];

pub fn install(interp: &mut Interpreter<'_>) {
    for (unit_type, members) in UNITS {
        for (case, symbol, coefficient, constant) in *members {
            let value = unit_value(unit_type, symbol, *coefficient, *constant);
            interp.register_static_value(unit_type, case, value);
        }
    }

    interp.register_free_fn("Measurement", measurement_init);
    interp.register_property(BuiltinReceiver::Measurement, "value", measurement_value);
    interp.register_property(BuiltinReceiver::Measurement, "unit", measurement_unit);
    interp.register_property(
        BuiltinReceiver::Measurement,
        "description",
        measurement_description,
    );
    for (name, mutating, func) in [
        ("converted", false, measurement_converted as IntrinsicFn),
        ("convert", true, measurement_convert),
    ] {
        interp.register_intrinsic(
            BuiltinReceiver::Measurement,
            name,
            MethodEntry { mutating, func },
        );
    }
}

fn unit_value(unit_type: &str, symbol: &str, coefficient: f64, constant: f64) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: unit_type.into(),
        fields: vec![
            ("symbol".into(), SwiftValue::Str(symbol.into())),
            ("_coefficient".into(), SwiftValue::Double(coefficient)),
            ("_constant".into(), SwiftValue::Double(constant)),
        ],
    }))
}

fn measurement_struct(value: f64, unit: SwiftValue) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "Measurement".into(),
        fields: vec![
            ("value".into(), SwiftValue::Double(value)),
            ("unit".into(), unit),
        ],
    }))
}

fn measurement_obj(value: &SwiftValue) -> Result<&Rc<StructObj>, StdError> {
    match value {
        SwiftValue::Struct(obj) if obj.type_name == "Measurement" => Ok(obj),
        other => Err(type_error(format!(
            "expected Measurement, got {}",
            other.type_name()
        ))),
    }
}

fn number_f64(value: &SwiftValue) -> Option<f64> {
    match value {
        SwiftValue::Double(d) => Some(*d),
        SwiftValue::Int(i) => Some(i.raw as f64),
        _ => None,
    }
}

/// `(value, coefficient, constant, unit_type)` from a measurement.
fn measurement_parts(obj: &Rc<StructObj>) -> Result<(f64, f64, f64, String), StdError> {
    let value = obj
        .get("value")
        .and_then(number_f64)
        .ok_or_else(|| type_error("malformed Measurement value"))?;
    let SwiftValue::Struct(unit) = obj
        .get("unit")
        .ok_or_else(|| type_error("Measurement is missing its unit"))?
    else {
        return Err(type_error("Measurement unit is malformed"));
    };
    let (coeff, constant) = unit_coefficients(unit)?;
    Ok((value, coeff, constant, unit.type_name.to_string()))
}

fn unit_coefficients(unit: &Rc<StructObj>) -> Result<(f64, f64), StdError> {
    let coeff = unit
        .get("_coefficient")
        .and_then(number_f64)
        .ok_or_else(|| type_error("unit is missing its coefficient"))?;
    let constant = unit
        .get("_constant")
        .and_then(number_f64)
        .ok_or_else(|| type_error("unit is missing its constant"))?;
    Ok((coeff, constant))
}

fn measurement_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let (mut value, mut unit) = (None, None);
    for arg in args {
        match arg.label.as_deref() {
            Some("value") => {
                value = number_f64(&arg.value);
            }
            Some("unit") => unit = Some(arg.value),
            other => {
                return Err(type_error(format!(
                    "unexpected Measurement initializer label {other:?}"
                )))
            }
        }
    }
    let (Some(value), Some(unit)) = (value, unit) else {
        return Err(type_error(
            "Measurement(value:unit:) requires both arguments",
        ));
    };
    // Validate the unit shape eagerly.
    let SwiftValue::Struct(ref unit_obj) = unit else {
        return Err(type_error("Measurement unit must be a Unit value"));
    };
    unit_coefficients(unit_obj)?;
    Ok(measurement_struct(value, unit))
}

fn measurement_value(recv: SwiftValue) -> StdResult {
    let obj = measurement_obj(&recv)?;
    Ok(obj.get("value").cloned().unwrap_or(SwiftValue::Double(0.0)))
}

fn measurement_unit(recv: SwiftValue) -> StdResult {
    let obj = measurement_obj(&recv)?;
    Ok(obj.get("unit").cloned().unwrap_or(SwiftValue::Nil))
}

fn measurement_description(recv: SwiftValue) -> StdResult {
    let obj = measurement_obj(&recv)?;
    let value = obj.get("value").and_then(number_f64).unwrap_or(0.0);
    let symbol = match obj.get("unit") {
        Some(SwiftValue::Struct(unit)) => match unit.get("symbol") {
            Some(SwiftValue::Str(s)) => s.to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    Ok(SwiftValue::Str(
        format!("{} {symbol}", format_double(value)).into(),
    ))
}

/// Convert `obj`'s value into `target` unit; returns `(new_value, target)`.
fn convert_to(obj: &Rc<StructObj>, target: &SwiftValue) -> Result<f64, StdError> {
    let (value, coeff, constant, unit_type) = measurement_parts(obj)?;
    let SwiftValue::Struct(target_unit) = target else {
        return Err(type_error("converted(to:) expects a Unit value"));
    };
    if target_unit.type_name != unit_type {
        return Err(type_error(format!(
            "cannot convert {unit_type} to {}",
            target_unit.type_name
        )));
    }
    let (t_coeff, t_constant) = unit_coefficients(target_unit)?;
    let base = value * coeff + constant;
    Ok((base - t_constant) / t_coeff)
}

fn measurement_converted(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let [target] = args.as_slice() else {
        return Err(type_error("converted(to:) expects one argument"));
    };
    let obj = measurement_obj(&recv)?;
    let new_value = convert_to(obj, target)?;
    Ok(Outcome {
        result: measurement_struct(new_value, target.clone()),
        receiver: recv,
    })
}

fn measurement_convert(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let [target] = args.as_slice() else {
        return Err(type_error("convert(to:) expects one argument"));
    };
    let obj = measurement_obj(&recv)?;
    let new_value = convert_to(obj, target)?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: measurement_struct(new_value, target.clone()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(unit_type: &str, case: &str) -> SwiftValue {
        let (_, members) = UNITS.iter().find(|(t, _)| *t == unit_type).unwrap();
        let (_, symbol, coeff, constant) = members.iter().find(|(c, ..)| *c == case).unwrap();
        unit_value(unit_type, symbol, *coeff, *constant)
    }

    fn parts(value: &SwiftValue) -> (f64, f64, f64, String) {
        let SwiftValue::Struct(obj) = value else {
            panic!("not a measurement");
        };
        measurement_parts(obj).unwrap()
    }

    #[test]
    fn linear_length_conversion() {
        let m = measurement_struct(5.0, unit("UnitLength", "kilometers"));
        let obj = measurement_obj(&m).unwrap();
        let miles = convert_to(obj, &unit("UnitLength", "miles")).unwrap();
        assert!((miles - 3.106_855_961_180_775).abs() < 1e-9);
    }

    #[test]
    fn affine_temperature_conversion() {
        let c = measurement_struct(100.0, unit("UnitTemperature", "celsius"));
        let obj = measurement_obj(&c).unwrap();
        let f = convert_to(obj, &unit("UnitTemperature", "fahrenheit")).unwrap();
        assert!((f - 212.0).abs() < 1e-9);
        let frozen = measurement_struct(0.0, unit("UnitTemperature", "celsius"));
        let frozen_f = convert_to(
            measurement_obj(&frozen).unwrap(),
            &unit("UnitTemperature", "fahrenheit"),
        )
        .unwrap();
        assert!((frozen_f - 32.0).abs() < 1e-9);
    }

    #[test]
    fn description_uses_symbol() {
        let m = measurement_struct(5.0, unit("UnitLength", "kilometers"));
        assert_eq!(
            measurement_description(m).unwrap(),
            SwiftValue::Str("5.0 km".into())
        );
        // Sanity: parts round-trips.
        let m2 = measurement_struct(2.5, unit("UnitMass", "pounds"));
        let (v, _, _, t) = parts(&m2);
        assert_eq!(v, 2.5);
        assert_eq!(t, "UnitMass");
    }
}
