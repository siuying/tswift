//! Operator semantics over [`SwiftValue`].
//!
//! Integer arithmetic distinguishes the overflow-*trapping* operators
//! (`+`/`-`/`*`/`/`/`%`) from the *wrapping* family (`&+`/`&-`/`&*`) and masking
//! shifts (`&<<`/`&>>`), matching Swift. A trap surfaces as `Err(String)`, which
//! the interpreter promotes to a runtime trap (`EvalError::Trap`).

use crate::value::{IntValue, SwiftValue};

/// Apply a non-short-circuiting binary operator. (`&&`/`||` are handled by the
/// interpreter so the right operand can be evaluated lazily.)
pub fn binary(op: &str, l: &SwiftValue, r: &SwiftValue) -> Result<SwiftValue, String> {
    match (l, r) {
        (SwiftValue::Int(a), SwiftValue::Int(b)) => int_binary(op, *a, *b),
        (SwiftValue::Double(a), SwiftValue::Double(b)) => double_binary(op, *a, *b),
        (SwiftValue::Bool(a), SwiftValue::Bool(b)) => bool_binary(op, *a, *b),
        (SwiftValue::Str(a), SwiftValue::Str(b)) => str_binary(op, a, b),
        _ => Err(format!(
            "operator `{op}` cannot apply to {} and {}",
            l.type_name(),
            r.type_name()
        )),
    }
}

/// Apply a unary operator (`-`, `+`, `!`, `~`).
pub fn unary(op: &str, v: &SwiftValue) -> Result<SwiftValue, String> {
    match (op, v) {
        ("-", SwiftValue::Int(a)) => {
            let res = IntValue::new(-a.raw, a.width);
            if res.in_range() {
                Ok(SwiftValue::Int(res))
            } else {
                Err(format!("arithmetic overflow negating {}", a.raw))
            }
        }
        ("-", SwiftValue::Double(a)) => Ok(SwiftValue::Double(-a)),
        ("+", SwiftValue::Int(_)) | ("+", SwiftValue::Double(_)) => Ok(v.clone()),
        ("!", SwiftValue::Bool(b)) => Ok(SwiftValue::Bool(!b)),
        ("~", SwiftValue::Int(a)) => Ok(SwiftValue::Int(IntValue::wrapped(a.width, !a.raw))),
        _ => Err(format!("operator `{op}` cannot apply to {}", v.type_name())),
    }
}

fn int_binary(op: &str, a: IntValue, b: IntValue) -> Result<SwiftValue, String> {
    let w = a.width;
    // Comparisons first — they yield Bool regardless of width.
    if let Some(res) = compare_op(op, a.raw, b.raw) {
        return Ok(SwiftValue::Bool(res));
    }
    let trapping = |raw: i128, what: &str| -> Result<SwiftValue, String> {
        let v = IntValue::new(raw, w);
        if v.in_range() {
            Ok(SwiftValue::Int(v))
        } else {
            Err(format!("arithmetic overflow during {what}"))
        }
    };
    match op {
        "+" => trapping(a.raw + b.raw, "addition"),
        "-" => trapping(a.raw - b.raw, "subtraction"),
        "*" => trapping(a.raw * b.raw, "multiplication"),
        "/" => {
            if b.raw == 0 {
                Err("division by zero".into())
            } else {
                trapping(a.raw / b.raw, "division")
            }
        }
        "%" => {
            if b.raw == 0 {
                Err("division by zero".into())
            } else {
                trapping(a.raw % b.raw, "remainder")
            }
        }
        "&+" => Ok(SwiftValue::Int(IntValue::wrapped(w, a.raw + b.raw))),
        "&-" => Ok(SwiftValue::Int(IntValue::wrapped(w, a.raw - b.raw))),
        "&*" => Ok(SwiftValue::Int(IntValue::wrapped(w, a.raw * b.raw))),
        "&" => Ok(SwiftValue::Int(IntValue::new(a.raw & b.raw, w))),
        "|" => Ok(SwiftValue::Int(IntValue::new(a.raw | b.raw, w))),
        "^" => Ok(SwiftValue::Int(IntValue::wrapped(w, a.raw ^ b.raw))),
        "<<" => trapping(shift_left(a.raw, b.raw), "left shift"),
        ">>" => Ok(SwiftValue::Int(IntValue::new(
            a.raw >> b.raw.clamp(0, 127),
            w,
        ))),
        "&<<" => {
            let s = b.raw.rem_euclid(w.bits() as i128) as u32;
            Ok(SwiftValue::Int(IntValue::wrapped(w, a.raw << s)))
        }
        "&>>" => {
            let s = b.raw.rem_euclid(w.bits() as i128) as u32;
            Ok(SwiftValue::Int(IntValue::new(a.raw >> s, w)))
        }
        _ => Err(format!("unknown integer operator `{op}`")),
    }
}

fn shift_left(a: i128, b: i128) -> i128 {
    if !(0..128).contains(&b) {
        0
    } else {
        a << b
    }
}

fn double_binary(op: &str, a: f64, b: f64) -> Result<SwiftValue, String> {
    if let Some(res) = compare_op_f(op, a, b) {
        return Ok(SwiftValue::Bool(res));
    }
    match op {
        "+" => Ok(SwiftValue::Double(a + b)),
        "-" => Ok(SwiftValue::Double(a - b)),
        "*" => Ok(SwiftValue::Double(a * b)),
        "/" => Ok(SwiftValue::Double(a / b)),
        "%" => Ok(SwiftValue::Double(a % b)),
        _ => Err(format!("unknown floating-point operator `{op}`")),
    }
}

fn bool_binary(op: &str, a: bool, b: bool) -> Result<SwiftValue, String> {
    match op {
        "==" => Ok(SwiftValue::Bool(a == b)),
        "!=" => Ok(SwiftValue::Bool(a != b)),
        "&&" => Ok(SwiftValue::Bool(a && b)),
        "||" => Ok(SwiftValue::Bool(a || b)),
        _ => Err(format!("unknown boolean operator `{op}`")),
    }
}

fn str_binary(op: &str, a: &str, b: &str) -> Result<SwiftValue, String> {
    match op {
        "+" => Ok(SwiftValue::Str(format!("{a}{b}"))),
        "==" => Ok(SwiftValue::Bool(a == b)),
        "!=" => Ok(SwiftValue::Bool(a != b)),
        "<" => Ok(SwiftValue::Bool(a < b)),
        "<=" => Ok(SwiftValue::Bool(a <= b)),
        ">" => Ok(SwiftValue::Bool(a > b)),
        ">=" => Ok(SwiftValue::Bool(a >= b)),
        _ => Err(format!("unknown string operator `{op}`")),
    }
}

fn compare_op(op: &str, a: i128, b: i128) -> Option<bool> {
    Some(match op {
        "==" => a == b,
        "!=" => a != b,
        "<" => a < b,
        "<=" => a <= b,
        ">" => a > b,
        ">=" => a >= b,
        _ => return None,
    })
}

fn compare_op_f(op: &str, a: f64, b: f64) -> Option<bool> {
    Some(match op {
        "==" => a == b,
        "!=" => a != b,
        "<" => a < b,
        "<=" => a <= b,
        ">" => a > b,
        ">=" => a >= b,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::IntWidth;

    fn int(raw: i128) -> SwiftValue {
        SwiftValue::int(raw)
    }

    #[test]
    fn add_traps_on_overflow() {
        let max = SwiftValue::Int(IntValue::new(IntWidth::I8.max(), IntWidth::I8));
        let one = SwiftValue::Int(IntValue::new(1, IntWidth::I8));
        assert!(binary("+", &max, &one).is_err());
        // wrapping form wraps to the minimum.
        let wrapped = binary("&+", &max, &one).unwrap();
        assert_eq!(
            wrapped,
            SwiftValue::Int(IntValue::new(IntWidth::I8.min(), IntWidth::I8))
        );
    }

    #[test]
    fn division_by_zero_traps() {
        assert!(binary("/", &int(1), &int(0)).is_err());
        assert!(binary("%", &int(1), &int(0)).is_err());
    }

    #[test]
    fn comparisons_yield_bool() {
        assert_eq!(
            binary("<", &int(1), &int(2)).unwrap(),
            SwiftValue::Bool(true)
        );
        assert_eq!(
            binary("==", &int(2), &int(2)).unwrap(),
            SwiftValue::Bool(true)
        );
    }
}
