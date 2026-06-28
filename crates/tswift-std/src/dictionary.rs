//! `Dictionary` method and property intrinsics with value semantics.

use std::rc::Rc;

use tswift_core::{
    BuiltinReceiver, EvalError, Interpreter, MethodEntry, Outcome, StdContext, StdError, StdResult,
    SwiftValue,
};

type Pairs = Vec<(SwiftValue, SwiftValue)>;

/// Register the `Dictionary` intrinsics of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    let d = BuiltinReceiver::Dictionary;
    interp.register_property(d, "count", count);
    interp.register_property(d, "isEmpty", is_empty);
    interp.register_property(d, "keys", keys);
    interp.register_property(d, "values", values);
    interp.register_property(d, "capacity", capacity);

    interp.register_intrinsic(
        d,
        "updateValue",
        MethodEntry {
            mutating: true,
            func: update_value,
        },
    );
    interp.register_intrinsic(
        d,
        "removeValue",
        MethodEntry {
            mutating: true,
            func: remove_value,
        },
    );
    interp.register_intrinsic(
        d,
        "merge",
        MethodEntry {
            mutating: true,
            func: merge,
        },
    );
    interp.register_intrinsic(
        d,
        "merging",
        MethodEntry {
            mutating: false,
            func: merging,
        },
    );
    interp.register_intrinsic(
        d,
        "mapValues",
        MethodEntry {
            mutating: false,
            func: map_values,
        },
    );
    interp.register_intrinsic(
        d,
        "filter",
        MethodEntry {
            mutating: false,
            func: filter,
        },
    );
    interp.register_intrinsic(
        d,
        "compactMapValues",
        MethodEntry {
            mutating: false,
            func: compact_map_values,
        },
    );
    interp.register_intrinsic(
        d,
        "removeAll",
        MethodEntry {
            mutating: true,
            func: remove_all,
        },
    );
    interp.register_intrinsic(
        d,
        "reserveCapacity",
        MethodEntry {
            mutating: true,
            func: reserve_capacity,
        },
    );
    interp.register_intrinsic(
        d,
        "popFirst",
        MethodEntry {
            mutating: true,
            func: pop_first,
        },
    );
}

fn pairs(recv: SwiftValue) -> Result<Rc<Pairs>, StdError> {
    match recv {
        SwiftValue::Dict(p) => Ok(p),
        other => Err(type_err(format!(
            "expected a dictionary receiver, got {}",
            other.type_name()
        ))),
    }
}

// ---- properties ------------------------------------------------------------

fn count(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(pairs(recv)?.len() as i128))
}

/// `Dictionary.capacity` — a lower bound modelled as the live element count
/// (Swift guarantees `capacity >= count`; exact reserve sizing is not modelled).
fn capacity(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(pairs(recv)?.len() as i128))
}

fn is_empty(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(pairs(recv)?.is_empty()))
}

fn keys(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Array(Rc::new(
        pairs(recv)?.iter().map(|(k, _)| k.clone()).collect(),
    )))
}

fn values(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Array(Rc::new(
        pairs(recv)?.iter().map(|(_, v)| v.clone()).collect(),
    )))
}

// ---- mutating methods ------------------------------------------------------

/// `updateValue(_:forKey:)` — set the value, returning the previous one (`nil`
/// if the key was absent).
fn update_value(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut p = pairs(recv)?;
    let value = args
        .first()
        .cloned()
        .ok_or_else(|| type_err("updateValue expects a value".into()))?;
    let key = args
        .get(1)
        .cloned()
        .ok_or_else(|| type_err("updateValue expects a key".into()))?;
    let store = Rc::make_mut(&mut p);
    let old = match store.iter_mut().find(|(k, _)| *k == key) {
        Some(slot) => std::mem::replace(&mut slot.1, value),
        None => {
            store.push((key, value));
            SwiftValue::Nil
        }
    };
    Ok(Outcome {
        result: old,
        receiver: SwiftValue::Dict(p),
    })
}

/// `removeValue(forKey:)` — remove and return the value (`nil` if absent).
fn remove_value(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut p = pairs(recv)?;
    let key = args
        .first()
        .cloned()
        .ok_or_else(|| type_err("removeValue expects a key".into()))?;
    let store = Rc::make_mut(&mut p);
    let removed = match store.iter().position(|(k, _)| *k == key) {
        Some(i) => store.remove(i).1,
        None => SwiftValue::Nil,
    };
    Ok(Outcome {
        result: removed,
        receiver: SwiftValue::Dict(p),
    })
}

/// `merge(_:uniquingKeysWith:)` — merge another dictionary in place, resolving
/// key collisions with the closure `(current, new) -> value`.
fn merge(ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut p = pairs(recv)?;
    let other = other_pairs(&args)?;
    let combine = closure(&args);
    let store = Rc::make_mut(&mut p);
    for (k, v) in other {
        match store.iter_mut().find(|(ek, _)| *ek == k) {
            Some(slot) => {
                slot.1 = match combine {
                    Some(id) => ctx.call_closure(id, vec![slot.1.clone(), v])?,
                    None => v,
                };
            }
            None => store.push((k, v)),
        }
    }
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Dict(p),
    })
}

// ---- non-mutating methods --------------------------------------------------

/// `merging(_:uniquingKeysWith:)` — like `merge`, returning a new dictionary.
fn merging(ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut store = pairs(recv.clone())?.as_ref().clone();
    let other = other_pairs(&args)?;
    let combine = closure(&args);
    for (k, v) in other {
        match store.iter_mut().find(|(ek, _)| *ek == k) {
            Some(slot) => {
                slot.1 = match combine {
                    Some(id) => ctx.call_closure(id, vec![slot.1.clone(), v])?,
                    None => v,
                };
            }
            None => store.push((k, v)),
        }
    }
    Ok(Outcome {
        result: SwiftValue::Dict(Rc::new(store)),
        receiver: recv,
    })
}

/// `filter(_:)` — keep the `(key, value)` pairs for which the predicate holds.
///
/// Unlike the generic `Sequence.filter` (which returns an array), the
/// dictionary form returns a `Dictionary`, so chained dictionary members
/// (`.keys`, `.mapValues`, …) keep working. The closure receives each element
/// as a `(key, value)` tuple.
fn filter(ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let id = closure(&args).ok_or_else(|| type_err("filter expects a closure".into()))?;
    let mut out: Pairs = Vec::new();
    for (k, v) in pairs(recv.clone())?.iter() {
        let element = SwiftValue::tuple_labeled(
            vec![k.clone(), v.clone()],
            vec![Some("key".to_string()), Some("value".to_string())],
        );
        if matches!(ctx.call_closure(id, vec![element])?, SwiftValue::Bool(true)) {
            out.push((k.clone(), v.clone()));
        }
    }
    Ok(Outcome {
        result: SwiftValue::Dict(Rc::new(out)),
        receiver: recv,
    })
}

/// `mapValues(_:)` — transform each value, keeping keys.
fn map_values(ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let id = closure(&args).ok_or_else(|| type_err("mapValues expects a closure".into()))?;
    let mut out: Pairs = Vec::new();
    for (k, v) in pairs(recv.clone())?.iter() {
        out.push((k.clone(), ctx.call_closure(id, vec![v.clone()])?));
    }
    Ok(Outcome {
        result: SwiftValue::Dict(Rc::new(out)),
        receiver: recv,
    })
}

/// `compactMapValues(_:)` — transform values, dropping keys whose value maps to
/// `nil`.
fn compact_map_values(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    let id = closure(&args).ok_or_else(|| type_err("compactMapValues expects a closure".into()))?;
    let mut out: Pairs = Vec::new();
    for (k, v) in pairs(recv.clone())?.iter() {
        match ctx.call_closure(id, vec![v.clone()])? {
            SwiftValue::Nil => {}
            mapped => out.push((k.clone(), mapped)),
        }
    }
    Ok(Outcome {
        result: SwiftValue::Dict(Rc::new(out)),
        receiver: recv,
    })
}

// ---- helpers ---------------------------------------------------------------

/// `Dictionary.removeAll(keepingCapacity:)` — drop every pair in place.
fn remove_all(_c: &mut dyn StdContext, _recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Dict(Rc::new(Vec::new())),
    })
}

/// `Dictionary.reserveCapacity(_:)` — a no-op here; storage grows implicitly.
fn reserve_capacity(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

/// `Dictionary.popFirst()` — remove and return the first `(key, value)` pair,
/// or `nil` when empty. Iteration order is unspecified, so callers should not
/// rely on which pair is returned for a multi-element dictionary.
fn pop_first(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let mut p = pairs(recv)?;
    let store = Rc::make_mut(&mut p);
    if store.is_empty() {
        return Ok(Outcome {
            result: SwiftValue::Nil,
            receiver: SwiftValue::Dict(p),
        });
    }
    let (k, v) = store.remove(0);
    let element = SwiftValue::tuple_labeled(
        vec![k, v],
        vec![Some("key".to_string()), Some("value".to_string())],
    );
    Ok(Outcome {
        result: element,
        receiver: SwiftValue::Dict(p),
    })
}

type Outcomes = Result<Outcome, StdError>;

fn type_err(msg: String) -> StdError {
    StdError::Error(EvalError::Type(msg))
}

fn closure(args: &[SwiftValue]) -> Option<usize> {
    args.iter().rev().find_map(|a| match a {
        SwiftValue::Closure(id) => Some(*id),
        _ => None,
    })
}

/// Extract the other dictionary's pairs from the first dictionary argument.
fn other_pairs(args: &[SwiftValue]) -> Result<Pairs, StdError> {
    args.iter()
        .find_map(|a| match a {
            SwiftValue::Dict(p) => Some(p.as_ref().clone()),
            _ => None,
        })
        .ok_or_else(|| type_err("merge expects another dictionary".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mock whose "closure" sums two ints (collision resolver / +N mapper).
    struct Summer;
    impl StdContext for Summer {
        fn call_closure(&mut self, id: usize, args: Vec<SwiftValue>) -> StdResult {
            let n = |v: &SwiftValue| match v {
                SwiftValue::Int(i) => i.raw,
                _ => 0,
            };
            Ok(match id {
                0 => SwiftValue::int(args.iter().map(n).sum()),
                1 => SwiftValue::int(n(&args[0]) * 10),
                // Predicate over a `(key, value)` element tuple: value >= 2.
                2 => match args.first() {
                    Some(SwiftValue::Tuple(t, _)) => SwiftValue::Bool(n(&t[1]) >= 2),
                    _ => SwiftValue::Bool(false),
                },
                _ => SwiftValue::Nil,
            })
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            unreachable!()
        }
    }

    fn dict(items: &[(&str, i128)]) -> SwiftValue {
        SwiftValue::Dict(Rc::new(
            items
                .iter()
                .map(|(k, v)| (SwiftValue::Str((*k).into()), SwiftValue::int(*v)))
                .collect(),
        ))
    }

    #[test]
    fn pop_first_remove_all_and_capacity() {
        let mut c = Summer;
        // popFirst returns the leading pair and shrinks the dictionary.
        let d = dict(&[("only", 9)]);
        let out = pop_first(&mut c, d, vec![]).unwrap();
        assert_eq!(
            out.result,
            SwiftValue::tuple(vec![SwiftValue::Str("only".into()), SwiftValue::int(9)])
        );
        assert!(matches!(out.receiver, SwiftValue::Dict(p) if p.is_empty()));
        // popFirst on an empty dictionary is nil.
        let empty = SwiftValue::Dict(Rc::new(Vec::new()));
        assert_eq!(
            pop_first(&mut c, empty, vec![]).unwrap().result,
            SwiftValue::Nil
        );
        // removeAll empties; capacity is at least the element count.
        let out = remove_all(&mut c, dict(&[("a", 1), ("b", 2)]), vec![]).unwrap();
        assert!(matches!(out.receiver, SwiftValue::Dict(p) if p.is_empty()));
        assert_eq!(
            capacity(dict(&[("a", 1), ("b", 2)])).unwrap(),
            SwiftValue::int(2)
        );
    }

    #[test]
    fn update_and_remove_preserve_cow() {
        let mut c = Summer;
        let shared = match dict(&[("a", 1)]) {
            SwiftValue::Dict(p) => p,
            _ => unreachable!(),
        };
        let out = update_value(
            &mut c,
            SwiftValue::Dict(Rc::clone(&shared)),
            vec![SwiftValue::int(2), SwiftValue::Str("a".into())],
        )
        .unwrap();
        assert_eq!(out.result, SwiftValue::int(1)); // previous value
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].1, SwiftValue::int(1), "original untouched (CoW)");

        let removed = remove_value(
            &mut c,
            SwiftValue::Dict(Rc::clone(&shared)),
            vec![SwiftValue::Str("a".into())],
        )
        .unwrap();
        assert_eq!(removed.result, SwiftValue::int(1));
    }

    #[test]
    fn merge_uses_collision_closure() {
        let mut c = Summer;
        let out = merge(
            &mut c,
            dict(&[("a", 1), ("b", 2)]),
            vec![dict(&[("a", 10), ("c", 3)]), SwiftValue::Closure(0)],
        )
        .unwrap();
        // a: 1+10=11, b: 2, c: 3
        assert_eq!(out.receiver, dict(&[("a", 11), ("b", 2), ("c", 3)]));
    }

    #[test]
    fn map_and_compact_map_values() {
        let mut c = Summer;
        let mapped = map_values(
            &mut c,
            dict(&[("a", 1), ("b", 2)]),
            vec![SwiftValue::Closure(1)],
        )
        .unwrap()
        .result;
        assert_eq!(mapped, dict(&[("a", 10), ("b", 20)]));
    }

    #[test]
    fn filter_returns_dictionary() {
        let mut c = Summer;
        let out = filter(
            &mut c,
            dict(&[("a", 1), ("b", 2), ("c", 3)]),
            vec![SwiftValue::Closure(2)],
        )
        .unwrap()
        .result;
        // Returns a Dictionary (not an array) of the pairs whose value >= 2.
        assert_eq!(out, dict(&[("b", 2), ("c", 3)]));
    }

    #[test]
    fn keys_and_values() {
        assert_eq!(
            count(dict(&[("a", 1), ("b", 2)])).unwrap(),
            SwiftValue::int(2)
        );
        match keys(dict(&[("a", 1)])).unwrap() {
            SwiftValue::Array(k) => assert_eq!(k[0], SwiftValue::Str("a".into())),
            _ => panic!("keys should be an array"),
        }
    }
}
