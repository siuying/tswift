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
        // Mixed integer/floating arithmetic promotes the integer side to
        // `Double`, modelling an integer literal used in a floating context
        // (`x += 1` on a `Double`, `radius * 2`, …).
        (SwiftValue::Int(a), SwiftValue::Double(b)) => double_binary(op, a.raw as f64, *b),
        (SwiftValue::Double(a), SwiftValue::Int(b)) => double_binary(op, *a, b.raw as f64),
        (SwiftValue::Bool(a), SwiftValue::Bool(b)) => bool_binary(op, *a, *b),
        (SwiftValue::Str(a), SwiftValue::Str(b)) => str_binary(op, a, b),
        (SwiftValue::Array(a), SwiftValue::Array(b)) => array_binary(op, a, b),
        (SwiftValue::Set(a), SwiftValue::Set(b)) => set_binary(op, a, b),
        (SwiftValue::Dict(a), SwiftValue::Dict(b)) => dict_binary(op, a, b),
        (
            SwiftValue::Range { lo, hi, inclusive },
            SwiftValue::Range {
                lo: lo2,
                hi: hi2,
                inclusive: inc2,
            },
        ) => range_binary(op, (*lo, *hi, *inclusive), (*lo2, *hi2, *inc2)),
        // `IndexPath` is `Comparable`: compare its element list lexicographically.
        (SwiftValue::Struct(a), SwiftValue::Struct(b))
            if a.type_name == "IndexPath" && b.type_name == "IndexPath" =>
        {
            index_path_binary(op, a, b)
        }
        // Metatype identity: `Int.self == type(of: x)`.
        (SwiftValue::Metatype(a), SwiftValue::Metatype(b)) => match op {
            "==" => Ok(SwiftValue::Bool(a == b)),
            "!=" => Ok(SwiftValue::Bool(a != b)),
            _ => Err(format!("operator `{op}` cannot apply to metatypes")),
        },
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
    // Range operators build a range value.
    match op {
        "..<" => {
            return Ok(SwiftValue::Range {
                lo: a.raw,
                hi: b.raw,
                inclusive: false,
            })
        }
        "..." => {
            return Ok(SwiftValue::Range {
                lo: a.raw,
                hi: b.raw,
                inclusive: true,
            })
        }
        _ => {}
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

/// Lexicographic comparison of two `IndexPath` values (Foundation builtins
/// backed by an `_indexes` array). Supports `==`/`!=` and the ordering ops.
fn index_path_binary(
    op: &str,
    a: &std::rc::Rc<crate::value::StructObj>,
    b: &std::rc::Rc<crate::value::StructObj>,
) -> Result<SwiftValue, String> {
    let indexes = |o: &crate::value::StructObj| -> Vec<i128> {
        match o.get("_indexes") {
            Some(SwiftValue::Array(items)) => items
                .iter()
                .filter_map(|v| match v {
                    SwiftValue::Int(i) => Some(i.raw),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    };
    let (la, lb) = (indexes(a), indexes(b));
    let ord = la.cmp(&lb);
    use std::cmp::Ordering;
    let res = match op {
        "==" => ord == Ordering::Equal,
        "!=" => ord != Ordering::Equal,
        "<" => ord == Ordering::Less,
        "<=" => ord != Ordering::Greater,
        ">" => ord == Ordering::Greater,
        ">=" => ord != Ordering::Less,
        _ => return Err(format!("operator `{op}` cannot apply to IndexPath")),
    };
    Ok(SwiftValue::Bool(res))
}

fn array_binary(
    op: &str,
    a: &std::rc::Rc<Vec<SwiftValue>>,
    b: &std::rc::Rc<Vec<SwiftValue>>,
) -> Result<SwiftValue, String> {
    match op {
        "+" => {
            let mut out = a.as_ref().clone();
            out.extend(b.as_ref().clone());
            Ok(SwiftValue::Array(std::rc::Rc::new(out)))
        }
        "==" => Ok(SwiftValue::Bool(a == b)),
        "!=" => Ok(SwiftValue::Bool(a != b)),
        _ => Err(format!("unknown array operator `{op}`")),
    }
}

/// `Set` equality is order-independent: equal size with mutual membership.
fn set_binary(
    op: &str,
    a: &std::rc::Rc<Vec<SwiftValue>>,
    b: &std::rc::Rc<Vec<SwiftValue>>,
) -> Result<SwiftValue, String> {
    let eq = a.len() == b.len() && a.iter().all(|x| b.contains(x));
    match op {
        "==" => Ok(SwiftValue::Bool(eq)),
        "!=" => Ok(SwiftValue::Bool(!eq)),
        _ => Err(format!("operator `{op}` cannot apply to Set and Set")),
    }
}

/// `Dictionary` equality is order-independent: equal size with each key bound
/// to an equal value on both sides.
fn dict_binary(
    op: &str,
    a: &std::rc::Rc<Vec<(SwiftValue, SwiftValue)>>,
    b: &std::rc::Rc<Vec<(SwiftValue, SwiftValue)>>,
) -> Result<SwiftValue, String> {
    let eq = a.len() == b.len()
        && a.iter()
            .all(|(k, v)| b.iter().any(|(k2, v2)| k2 == k && v2 == v));
    match op {
        "==" => Ok(SwiftValue::Bool(eq)),
        "!=" => Ok(SwiftValue::Bool(!eq)),
        _ => Err(format!("operator `{op}` cannot apply to Dictionary")),
    }
}

/// `Range`/`ClosedRange` equality compares the bounds and end style.
fn range_binary(
    op: &str,
    a: (i128, i128, bool),
    b: (i128, i128, bool),
) -> Result<SwiftValue, String> {
    match op {
        "==" => Ok(SwiftValue::Bool(a == b)),
        "!=" => Ok(SwiftValue::Bool(a != b)),
        _ => Err(format!("operator `{op}` cannot apply to ranges")),
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
    fn mixed_int_double_promotes_to_double() {
        let i = int(3);
        let d = SwiftValue::Double(2.0);
        assert_eq!(binary("+", &i, &d).unwrap(), SwiftValue::Double(5.0));
        assert_eq!(binary("*", &d, &int(4)).unwrap(), SwiftValue::Double(8.0));
        assert_eq!(
            binary("<", &int(1), &SwiftValue::Double(2.0)).unwrap(),
            SwiftValue::Bool(true)
        );
    }

    #[test]
    fn division_by_zero_traps() {
        assert!(binary("/", &int(1), &int(0)).is_err());
        assert!(binary("%", &int(1), &int(0)).is_err());
    }

    #[test]
    fn index_path_compares_lexicographically() {
        use crate::value::StructObj;
        use std::rc::Rc;
        let path = |xs: &[i128]| {
            SwiftValue::Struct(Rc::new(StructObj {
                type_name: "IndexPath".into(),
                fields: vec![(
                    "_indexes".into(),
                    SwiftValue::Array(Rc::new(xs.iter().copied().map(int).collect())),
                )],
            }))
        };
        let a = path(&[1, 2]);
        let b = path(&[1, 3]);
        let c = path(&[1, 2]);
        assert_eq!(binary("<", &a, &b).unwrap(), SwiftValue::Bool(true));
        assert_eq!(binary(">", &a, &b).unwrap(), SwiftValue::Bool(false));
        assert_eq!(binary("<=", &a, &c).unwrap(), SwiftValue::Bool(true));
        assert_eq!(binary(">=", &a, &c).unwrap(), SwiftValue::Bool(true));
        assert_eq!(binary("==", &a, &c).unwrap(), SwiftValue::Bool(true));
        assert_eq!(binary("!=", &a, &b).unwrap(), SwiftValue::Bool(true));
        // A shorter prefix orders before its extension.
        assert_eq!(
            binary("<", &path(&[1]), &path(&[1, 0])).unwrap(),
            SwiftValue::Bool(true)
        );
    }

    #[test]
    fn collection_equality_is_order_independent() {
        use std::rc::Rc;
        let set = |xs: &[i128]| SwiftValue::Set(Rc::new(xs.iter().copied().map(int).collect()));
        // Sets compare by membership, ignoring insertion order.
        assert_eq!(
            binary("==", &set(&[1, 2]), &set(&[2, 1])).unwrap(),
            SwiftValue::Bool(true)
        );
        assert_eq!(
            binary("!=", &set(&[1, 2]), &set(&[1, 3])).unwrap(),
            SwiftValue::Bool(true)
        );
        let dict = |pairs: &[(i128, i128)]| {
            SwiftValue::Dict(Rc::new(
                pairs.iter().map(|(k, v)| (int(*k), int(*v))).collect(),
            ))
        };
        assert_eq!(
            binary("==", &dict(&[(1, 10), (2, 20)]), &dict(&[(2, 20), (1, 10)])).unwrap(),
            SwiftValue::Bool(true)
        );
        assert_eq!(
            binary("==", &dict(&[(1, 10)]), &dict(&[(1, 11)])).unwrap(),
            SwiftValue::Bool(false)
        );
        // Ranges compare bounds and end style: half-open != closed.
        let range = |lo, hi, inc| SwiftValue::Range {
            lo,
            hi,
            inclusive: inc,
        };
        assert_eq!(
            binary("==", &range(1, 3, false), &range(1, 3, false)).unwrap(),
            SwiftValue::Bool(true)
        );
        assert_eq!(
            binary("==", &range(1, 3, true), &range(1, 3, false)).unwrap(),
            SwiftValue::Bool(false)
        );
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
