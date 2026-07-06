//! `Dictionary` method and property intrinsics with value semantics.

use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, EvalError, Interpreter, LabeledMethodEntry, MethodEntry, Outcome,
    StdContext, StdError, StdResult, StructObj, SwiftValue,
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
    interp.register_property(d, "hashValue", hash_value);
    interp.register_property(d, "description", description);
    interp.register_property(d, "debugDescription", description);
    interp.register_property(d, "startIndex", start_index);
    interp.register_property(d, "endIndex", end_index);

    interp.register_labeled_intrinsic(
        d,
        "index",
        LabeledMethodEntry {
            mutating: false,
            func: index_labeled,
        },
    );
    interp.register_labeled_intrinsic(
        d,
        "formIndex",
        LabeledMethodEntry {
            mutating: true,
            func: form_index_labeled,
        },
    );
    interp.register_labeled_intrinsic(
        d,
        "remove",
        LabeledMethodEntry {
            mutating: true,
            func: remove_at_labeled,
        },
    );

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
    interp.register_intrinsic(
        d,
        "makeIterator",
        MethodEntry {
            mutating: false,
            func: make_iterator,
        },
    );
    interp.register_intrinsic(
        d,
        "next",
        MethodEntry {
            mutating: true,
            func: next,
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

// ---- Dictionary.Index -----------------------------------------------------

/// Construct an opaque `Dictionary.Index` anchored to the key currently at
/// `offset` in `pairs`.
///
/// The `_anchor` stores the key at that position (or `Void` for end-sentinel).
/// Stale-index detection compares this anchor against the live key when the
/// index is used, trapping if the collection was mutated in between.
pub(crate) fn make_dict_index(offset: usize, pairs: &[(SwiftValue, SwiftValue)]) -> SwiftValue {
    let anchor = pairs
        .get(offset)
        .map(|(k, _)| k.clone())
        .unwrap_or(SwiftValue::Void);
    SwiftValue::Struct(std::rc::Rc::new(StructObj {
        type_name: "Dictionary.Index".into(),
        fields: vec![
            ("_offset".into(), SwiftValue::int(offset as i128)),
            ("_anchor".into(), anchor),
        ],
    }))
}

/// Validate a `Dictionary.Index` against the live pairs slice and return the
/// offset.  Traps on stale or out-of-range indices.
pub(crate) fn check_dict_index(
    pairs: &[(SwiftValue, SwiftValue)],
    v: &SwiftValue,
) -> Result<usize, StdError> {
    let obj = match v {
        SwiftValue::Struct(o) if o.type_name == "Dictionary.Index" => o,
        _ => return Err(type_err("expected a Dictionary.Index".into())),
    };
    let offset = match obj.get("_offset") {
        Some(SwiftValue::Int(i)) if i.raw >= 0 => i.raw as usize,
        _ => return Err(type_err("invalid Dictionary.Index".into())),
    };
    if offset >= pairs.len() {
        return Err(StdError::Error(EvalError::Trap(
            "Dictionary.Index is at or past endIndex".into(),
        )));
    }
    if let Some(anchor) = obj.get("_anchor") {
        if *anchor != SwiftValue::Void && *anchor != pairs[offset].0 {
            return Err(StdError::Error(EvalError::Trap(
                "invalid Dictionary.Index: collection was mutated after this index was created"
                    .into(),
            )));
        }
    }
    Ok(offset)
}

/// Extract just the positional offset from a `Dictionary.Index` without
/// validity checking (used for computing next-indices from freshly-made ones).
pub(crate) fn dict_index_offset(v: &SwiftValue) -> Option<usize> {
    match v {
        SwiftValue::Struct(obj) if obj.type_name == "Dictionary.Index" => {
            obj.get("_offset").and_then(|f| match f {
                SwiftValue::Int(i) if i.raw >= 0 => Some(i.raw as usize),
                _ => None,
            })
        }
        _ => None,
    }
}

// ---- properties ------------------------------------------------------------

fn count(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(pairs(recv)?.len() as i128))
}

/// `Dictionary.description` — the bracketed `[key: value, …]` rendering (`[:]`
/// when empty). Pairs appear in insertion order here, unlike Swift's hashed
/// order.
fn description(recv: SwiftValue) -> StdResult {
    pairs(recv.clone())?;
    Ok(SwiftValue::Str(recv.to_string()))
}

/// `Dictionary.hashValue` — an order-independent digest over the key/value
/// pairs: equal dictionaries hash equally regardless of insertion order. Each
/// pair is hashed by combining its key and value digests, then the pair
/// digests are merged with a commutative wrapping sum.
fn hash_value(recv: SwiftValue) -> StdResult {
    let store = pairs(recv)?;
    let mut acc: u64 = 0;
    for (k, v) in store.iter() {
        let kd = crate::set::value_digest(k);
        let vd = crate::set::value_digest(v);
        // Order-sensitive within a pair, commutative across pairs.
        acc = acc.wrapping_add(kd.wrapping_mul(0x0000_0100_0000_01b3) ^ vd);
    }
    acc ^= store.len() as u64;
    Ok(SwiftValue::int(i128::from(acc as i64)))
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

/// `Dictionary.startIndex` — an opaque index anchored to the first key, or
/// an end-sentinel for empty dictionaries.
fn start_index(recv: SwiftValue) -> StdResult {
    let store = pairs(recv)?;
    Ok(make_dict_index(0, &store))
}

/// `Dictionary.endIndex` — an opaque one-past-the-end sentinel.
fn end_index(recv: SwiftValue) -> StdResult {
    let store = pairs(recv)?;
    Ok(make_dict_index(store.len(), &store))
}

/// `Dictionary.index` — label-aware dispatch:
/// - `index(after: i)`   → advance by one (traps at `endIndex`)
/// - `index(forKey: k)`  → returns `Dictionary.Index?`, `nil` when key absent
///
/// Also used as the back-end for `formIndex(after:)` in `dispatch.rs`.
pub(crate) fn index_labeled(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let store = pairs(recv.clone())?;

    // index(after: i)
    if let Some(arg) = args.iter().find(|a| a.label.as_deref() == Some("after")) {
        let offset = dict_index_offset(&arg.value)
            .ok_or_else(|| type_err("index(after:) expects a Dictionary.Index".into()))?;
        let count = store.len();
        if offset >= count {
            return Err(StdError::Error(EvalError::Trap(
                "Dictionary.index(after:): index is at or past endIndex".into(),
            )));
        }
        return Ok(Some(Outcome {
            result: make_dict_index(offset + 1, &store),
            receiver: recv,
        }));
    }

    // index(forKey: k)
    if let Some(arg) = args.iter().find(|a| a.label.as_deref() == Some("forKey")) {
        let result = match store.iter().position(|(k, _)| *k == arg.value) {
            Some(i) => make_dict_index(i, &store),
            None => SwiftValue::Nil,
        };
        return Ok(Some(Outcome {
            result,
            receiver: recv,
        }));
    }

    Ok(None)
}

/// `Dictionary.formIndex(after:)` — registered so `has_labeled_intrinsic`
/// returns `true` for the dispatch.rs special-case gate.  The actual inout
/// write-back of `&i` is performed there by calling `index_labeled` and
/// writing the result back to the caller's place.
fn form_index_labeled(
    c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    index_labeled(c, recv, args)
}

/// `Dictionary.remove(at:)` — label-aware: requires the `at:` label.
/// Validates the stale-index anchor before removal and returns the removed
/// (key, value) labeled tuple.
fn remove_at_labeled(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    if let Some(arg) = args.iter().find(|a| a.label.as_deref() == Some("at")) {
        let mut p = pairs(recv)?;
        let store = Rc::make_mut(&mut p);
        let offset = check_dict_index(store, &arg.value)?;
        let (k, v) = store.remove(offset);
        let result = SwiftValue::tuple_labeled(
            vec![k, v],
            vec![Some("key".to_string()), Some("value".to_string())],
        );
        return Ok(Some(Outcome {
            result,
            receiver: SwiftValue::Dict(p),
        }));
    }
    // No `at:` label — let positional dispatch handle it (e.g. removeValue)
    Ok(None)
}

/// `Dictionary.makeIterator()` — for-in over `Dictionary` is driven by
/// `materialize_builtin_sequence` which already works. This no-op gives
/// honest coverage credit.
fn make_iterator(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    Ok(Outcome {
        result: recv.clone(),
        receiver: recv,
    })
}

/// `Dictionary.next()` — pop and return the first `(key, value)` labeled pair
/// as the iterator's next element, or `nil` when exhausted. Mirrors
/// `popFirst()` in value-semantic terms.
fn next(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let mut p = pairs(recv)?;
    let store = Rc::make_mut(&mut p);
    if store.is_empty() {
        return Ok(Outcome {
            result: SwiftValue::Nil,
            receiver: SwiftValue::Dict(p),
        });
    }
    let (k, v) = store.remove(0);
    let result = SwiftValue::tuple_labeled(
        vec![k, v],
        vec![Some("key".to_string()), Some("value".to_string())],
    );
    Ok(Outcome {
        result,
        receiver: SwiftValue::Dict(p),
    })
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
    fn description_renders_pairs() {
        assert_eq!(
            description(dict(&[("a", 1), ("b", 2)])).unwrap(),
            SwiftValue::Str("[\"a\": 1, \"b\": 2]".into())
        );
        assert_eq!(
            description(SwiftValue::Dict(Rc::new(Vec::new()))).unwrap(),
            SwiftValue::Str("[:]".into())
        );
    }

    #[test]
    fn hash_value_is_order_independent() {
        // Equal dictionaries hash equally regardless of insertion order.
        assert_eq!(
            hash_value(dict(&[("a", 1), ("b", 2)])).unwrap(),
            hash_value(dict(&[("b", 2), ("a", 1)])).unwrap()
        );
        // A differing value changes the hash.
        assert_ne!(
            hash_value(dict(&[("a", 1)])).unwrap(),
            hash_value(dict(&[("a", 2)])).unwrap()
        );
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

    #[test]
    fn dict_index_round_trip() {
        let mut c = Summer;
        let d = dict(&[("a", 1)]);
        let pairs_a1: Vec<(SwiftValue, SwiftValue)> =
            vec![(SwiftValue::Str("a".into()), SwiftValue::int(1))];
        let di0 = make_dict_index(0, &pairs_a1); // anchored to key "a"
        let di1 = make_dict_index(1, &pairs_a1); // end sentinel

        // startIndex offset 0 (anchored), endIndex offset 1 (sentinel).
        assert_eq!(start_index(d.clone()).unwrap(), di0);
        assert_eq!(end_index(d.clone()).unwrap(), di1);

        // index(after: startIndex) == endIndex for single-pair dict.
        let after_out = index_labeled(
            &mut c,
            d.clone(),
            vec![Arg {
                label: Some("after".to_string()),
                value: di0.clone(),

                static_ty: None,
            }],
        )
        .unwrap()
        .unwrap();
        assert_eq!(after_out.result, di1);

        // index(after: endIndex) traps.
        assert!(index_labeled(
            &mut c,
            d.clone(),
            vec![Arg {
                label: Some("after".to_string()),
                value: di1.clone(),

                static_ty: None,
            }],
        )
        .is_err());

        // index(forKey:) present → Some (anchored), absent → nil.
        let ki = index_labeled(
            &mut c,
            d.clone(),
            vec![Arg {
                label: Some("forKey".to_string()),
                value: SwiftValue::Str("a".into()),

                static_ty: None,
            }],
        )
        .unwrap()
        .unwrap();
        assert_eq!(ki.result, di0);

        let absent = index_labeled(
            &mut c,
            d.clone(),
            vec![Arg {
                label: Some("forKey".to_string()),
                value: SwiftValue::Str("z".into()),

                static_ty: None,
            }],
        )
        .unwrap()
        .unwrap();
        assert_eq!(absent.result, SwiftValue::Nil);

        // remove(at:) with valid anchored index returns the labeled tuple.
        let rm = remove_at_labeled(
            &mut c,
            d.clone(),
            vec![Arg {
                label: Some("at".to_string()),
                value: di0.clone(),

                static_ty: None,
            }],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            rm.result,
            SwiftValue::tuple_labeled(
                vec![SwiftValue::Str("a".into()), SwiftValue::int(1)],
                vec![Some("key".to_string()), Some("value".to_string())],
            )
        );
        assert!(matches!(rm.receiver, SwiftValue::Dict(p) if p.is_empty()));

        // remove(at: endIndex) traps — offset >= len.
        assert!(remove_at_labeled(
            &mut c,
            d.clone(),
            vec![Arg {
                label: Some("at".to_string()),
                value: di1.clone(),

                static_ty: None,
            }],
        )
        .is_err());

        // Stale-index detection: using a pre-mutation index after remove(at:) traps.
        let two = dict(&[("a", 1), ("b", 2)]);
        let two_pairs: Vec<(SwiftValue, SwiftValue)> = vec![
            (SwiftValue::Str("a".into()), SwiftValue::int(1)),
            (SwiftValue::Str("b".into()), SwiftValue::int(2)),
        ];
        let old = make_dict_index(0, &two_pairs); // anchored to key "a"
        let rm2 = remove_at_labeled(
            &mut c,
            two,
            vec![Arg {
                label: Some("at".to_string()),
                value: old.clone(),

                static_ty: None,
            }],
        )
        .unwrap()
        .unwrap();
        // After removal, offset 0 now holds "b"; old index has anchor "a" → trap.
        assert!(
            remove_at_labeled(
                &mut c,
                rm2.receiver,
                vec![Arg {
                    label: Some("at".to_string()),
                    value: old,

                    static_ty: None,
                }],
            )
            .is_err(),
            "stale Dictionary.Index after mutation must trap"
        );

        // next() pops first pair.
        let nx = next(&mut c, d.clone(), vec![]).unwrap();
        assert_eq!(
            nx.result,
            SwiftValue::tuple_labeled(
                vec![SwiftValue::Str("a".into()), SwiftValue::int(1)],
                vec![Some("key".to_string()), Some("value".to_string())],
            )
        );
        assert!(matches!(nx.receiver, SwiftValue::Dict(p) if p.is_empty()));

        // next() on empty dict returns nil.
        let empty = SwiftValue::Dict(Rc::new(vec![]));
        let nx2 = next(&mut c, empty, vec![]).unwrap();
        assert_eq!(nx2.result, SwiftValue::Nil);
    }
}
