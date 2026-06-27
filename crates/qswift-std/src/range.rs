//! `Range`/`ClosedRange` method and property intrinsics.

use qswift_core::{
    BuiltinReceiver, EvalError, Interpreter, MethodEntry, Outcome, StdContext, StdError, StdResult,
    SwiftValue,
};

/// Register the range intrinsics of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_property(BuiltinReceiver::Range, "lowerBound", lower_bound);
    interp.register_property(BuiltinReceiver::Range, "upperBound", upper_bound);
    interp.register_property(BuiltinReceiver::Range, "count", count);
    interp.register_property(BuiltinReceiver::Range, "isEmpty", is_empty);

    interp.register_intrinsic(
        BuiltinReceiver::Range,
        "contains",
        MethodEntry {
            mutating: false,
            func: contains,
        },
    );
    interp.register_intrinsic(
        BuiltinReceiver::Range,
        "clamped",
        MethodEntry {
            mutating: false,
            func: clamped,
        },
    );
}

/// Decompose a range value into `(lo, hi, inclusive)`.
fn parts(v: &SwiftValue) -> Result<(i128, i128, bool), StdError> {
    match v {
        SwiftValue::Range { lo, hi, inclusive } => Ok((*lo, *hi, *inclusive)),
        other => Err(StdError::Error(EvalError::Type(format!(
            "expected a range, got {}",
            other.type_name()
        )))),
    }
}

/// Exclusive element count of a range (`upperBound - lowerBound`), never below 0.
fn element_count(lo: i128, hi: i128, inclusive: bool) -> i128 {
    let end = if inclusive { hi + 1 } else { hi };
    (end - lo).max(0)
}

fn lower_bound(v: SwiftValue) -> StdResult {
    let (lo, _, _) = parts(&v)?;
    Ok(SwiftValue::int(lo))
}

fn upper_bound(v: SwiftValue) -> StdResult {
    let (_, hi, _) = parts(&v)?;
    Ok(SwiftValue::int(hi))
}

fn count(v: SwiftValue) -> StdResult {
    let (lo, hi, inclusive) = parts(&v)?;
    Ok(SwiftValue::int(element_count(lo, hi, inclusive)))
}

fn is_empty(v: SwiftValue) -> StdResult {
    let (lo, hi, inclusive) = parts(&v)?;
    Ok(SwiftValue::Bool(element_count(lo, hi, inclusive) == 0))
}

/// `Range.contains(_:)` — membership of an integer element.
fn contains(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (lo, hi, inclusive) = parts(&recv)?;
    let x = match args.first() {
        Some(SwiftValue::Int(i)) => i.raw,
        _ => {
            return Err(StdError::Error(EvalError::Type(
                "contains expects an integer".into(),
            )))
        }
    };
    let inside = if inclusive {
        x >= lo && x <= hi
    } else {
        x >= lo && x < hi
    };
    Ok(Outcome {
        result: SwiftValue::Bool(inside),
        receiver: recv,
    })
}

/// `Range.clamped(to:)` — intersection with another range, same end style.
fn clamped(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (lo, hi, inclusive) = parts(&recv)?;
    let (olo, ohi, _) =
        parts(args.first().ok_or_else(|| {
            StdError::Error(EvalError::Type("clamped(to:) expects a range".into()))
        })?)?;
    let new_lo = lo.max(olo);
    let new_hi = hi.min(ohi).max(new_lo);
    Ok(Outcome {
        result: SwiftValue::Range {
            lo: new_lo,
            hi: new_hi,
            inclusive,
        },
        receiver: recv,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockCtx;
    impl StdContext for MockCtx {
        fn call_closure(&mut self, _id: usize, _a: Vec<SwiftValue>) -> StdResult {
            Ok(SwiftValue::Nil)
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            unreachable!()
        }
    }

    fn exclusive(lo: i128, hi: i128) -> SwiftValue {
        SwiftValue::Range {
            lo,
            hi,
            inclusive: false,
        }
    }

    #[test]
    fn bounds_and_count() {
        let r = exclusive(1, 5);
        assert_eq!(lower_bound(r.clone()).unwrap(), SwiftValue::int(1));
        assert_eq!(upper_bound(r.clone()).unwrap(), SwiftValue::int(5));
        assert_eq!(count(r.clone()).unwrap(), SwiftValue::int(4));
        assert_eq!(is_empty(r).unwrap(), SwiftValue::Bool(false));
        let closed = SwiftValue::Range {
            lo: 1,
            hi: 5,
            inclusive: true,
        };
        assert_eq!(count(closed).unwrap(), SwiftValue::int(5));
    }

    #[test]
    fn contains_membership() {
        let mut c = MockCtx;
        let r = exclusive(1, 5);
        assert_eq!(
            contains(&mut c, r.clone(), vec![SwiftValue::int(3)])
                .unwrap()
                .result,
            SwiftValue::Bool(true)
        );
        assert_eq!(
            contains(&mut c, r, vec![SwiftValue::int(5)])
                .unwrap()
                .result,
            SwiftValue::Bool(false)
        );
    }

    #[test]
    fn clamped_intersects() {
        let mut c = MockCtx;
        let out = clamped(&mut c, exclusive(0, 10), vec![exclusive(3, 20)])
            .unwrap()
            .result;
        assert_eq!(out, exclusive(3, 10));
    }
}
