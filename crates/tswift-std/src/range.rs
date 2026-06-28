//! `Range`/`ClosedRange` method and property intrinsics.

use tswift_core::{
    BuiltinReceiver, EvalError, Interpreter, MethodEntry, Outcome, StdContext, StdError, StdResult,
    SwiftValue,
};

/// Register the range intrinsics of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_property(BuiltinReceiver::Range, "lowerBound", lower_bound);
    interp.register_property(BuiltinReceiver::Range, "upperBound", upper_bound);
    interp.register_property(BuiltinReceiver::Range, "count", count);
    interp.register_property(BuiltinReceiver::Range, "isEmpty", is_empty);
    interp.register_property(BuiltinReceiver::Range, "description", description);
    interp.register_property(BuiltinReceiver::Range, "debugDescription", description);
    interp.register_property(BuiltinReceiver::Range, "startIndex", lower_bound);
    interp.register_property(BuiltinReceiver::Range, "endIndex", upper_bound);
    interp.register_property(BuiltinReceiver::Range, "indices", indices);
    interp.register_property(BuiltinReceiver::Range, "hashValue", hash_value);

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
    interp.register_intrinsic(
        BuiltinReceiver::Range,
        "overlaps",
        MethodEntry {
            mutating: false,
            func: overlaps,
        },
    );
    interp.register_intrinsic(
        BuiltinReceiver::Range,
        "distance",
        MethodEntry {
            mutating: false,
            func: distance,
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

/// `Range.indices` — an integer `Range` is its own index collection, so the
/// indices are the range itself (valid for the half-open form modelled here).
fn indices(v: SwiftValue) -> StdResult {
    parts(&v)?;
    Ok(v)
}

/// `Range.hashValue` — a stable digest over the bounds and end style; equal
/// ranges hash equally (the per-process seed Swift uses is not modelled).
fn hash_value(v: SwiftValue) -> StdResult {
    let (lo, hi, inclusive) = parts(&v)?;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |x: u64| {
        for b in x.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    mix(lo as u64);
    mix(hi as u64);
    mix(u64::from(inclusive));
    Ok(SwiftValue::int(i128::from(h as i64)))
}

/// `Range.description` / `debugDescription` — `lo..<hi` or `lo...hi`.
fn description(v: SwiftValue) -> StdResult {
    let (lo, hi, inclusive) = parts(&v)?;
    let op = if inclusive { "..." } else { "..<" };
    Ok(SwiftValue::Str(format!("{lo}{op}{hi}")))
}

/// `Range.overlaps(_:)` — whether the two ranges share at least one element.
fn overlaps(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (lo, hi, inclusive) = parts(&recv)?;
    let (olo, ohi, oinc) = parts(
        args.first()
            .ok_or_else(|| StdError::Error(EvalError::Type("overlaps expects a range".into())))?,
    )?;
    let end = if inclusive { hi + 1 } else { hi };
    let oend = if oinc { ohi + 1 } else { ohi };
    // Non-empty half-open intervals [lo, end) and [olo, oend) overlap iff each
    // starts before the other ends.
    let inside = lo < end && olo < oend && lo < oend && olo < end;
    Ok(Outcome {
        result: SwiftValue::Bool(inside),
        receiver: recv,
    })
}

/// `Range.distance(from:to:)` — signed element distance `to - from` for the
/// integer index space.
fn distance(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let int = |v: Option<&SwiftValue>| match v {
        Some(SwiftValue::Int(i)) => Ok(i.raw),
        _ => Err(StdError::Error(EvalError::Type(
            "distance(from:to:) expects integer indices".into(),
        ))),
    };
    let from = int(args.first())?;
    let to = int(args.get(1))?;
    Ok(Outcome {
        result: SwiftValue::int(to - from),
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
    fn indices_bounds_and_hash() {
        let r = exclusive(2, 7);
        assert_eq!(lower_bound(r.clone()).unwrap(), SwiftValue::int(2));
        assert_eq!(upper_bound(r.clone()).unwrap(), SwiftValue::int(7));
        // An integer range is its own index collection.
        assert_eq!(indices(r.clone()).unwrap(), r.clone());
        // Equal ranges hash equally; different bounds differ.
        assert_eq!(
            hash_value(r.clone()).unwrap(),
            hash_value(exclusive(2, 7)).unwrap()
        );
        assert_ne!(hash_value(r).unwrap(), hash_value(exclusive(2, 8)).unwrap());
    }

    #[test]
    fn description_overlaps_distance() {
        let mut c = MockCtx;
        assert_eq!(
            description(exclusive(1, 5)).unwrap(),
            SwiftValue::Str("1..<5".into())
        );
        let closed = SwiftValue::Range {
            lo: 1,
            hi: 5,
            inclusive: true,
        };
        assert_eq!(
            description(closed).unwrap(),
            SwiftValue::Str("1...5".into())
        );
        assert_eq!(
            overlaps(&mut c, exclusive(1, 5), vec![exclusive(3, 8)])
                .unwrap()
                .result,
            SwiftValue::Bool(true)
        );
        // Adjacent half-open ranges do not overlap.
        assert_eq!(
            overlaps(&mut c, exclusive(1, 5), vec![exclusive(5, 8)])
                .unwrap()
                .result,
            SwiftValue::Bool(false)
        );
        assert_eq!(
            distance(
                &mut c,
                exclusive(0, 10),
                vec![SwiftValue::int(2), SwiftValue::int(7)]
            )
            .unwrap()
            .result,
            SwiftValue::int(5)
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
