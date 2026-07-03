//! `ReversedCollection` method intrinsics.
//!
//! In Swift, `[1,2,3].reversed()` returns a `ReversedCollection<[Int]>`,
//! a lazy view over the base array in reverse order.  In this runtime it is
//! represented as:
//!
//! ```text
//! SwiftValue::Struct { type_name: "ReversedCollection",
//!                      fields: [("_base", Array(...))] }
//! ```
//!
//! Iteration (for-in, map, filter, …) materialises the reversed elements via
//! `materialize_builtin_sequence`.  The methods below implement the
//! `Collection` members listed in the stdlib inventory.
//!
//! `reversed()` on a `ReversedCollection` returns the original base `Array`
//! (round-trip: `[1,2,3].reversed().reversed() == [1,2,3]`).

use std::rc::Rc;

use tswift_core::{
    BuiltinReceiver, EvalError, Interpreter, MethodEntry, Outcome, StdContext, StdError, StdResult,
    StructObj, SwiftValue,
};

/// Register all `ReversedCollection` intrinsics.
pub fn install(interp: &mut Interpreter<'_>) {
    let nm = |interp: &mut Interpreter<'_>, name: &str, func: tswift_core::IntrinsicFn| {
        interp.register_intrinsic(
            BuiltinReceiver::ReversedCollection,
            name,
            MethodEntry {
                mutating: false,
                func,
            },
        );
    };

    interp.register_property(BuiltinReceiver::ReversedCollection, "count", count);
    interp.register_property(BuiltinReceiver::ReversedCollection, "isEmpty", is_empty);
    interp.register_property(BuiltinReceiver::ReversedCollection, "first", first);
    interp.register_property(BuiltinReceiver::ReversedCollection, "last", last);
    interp.register_property(
        BuiltinReceiver::ReversedCollection,
        "startIndex",
        start_index,
    );
    interp.register_property(BuiltinReceiver::ReversedCollection, "endIndex", end_index);
    interp.register_property(BuiltinReceiver::ReversedCollection, "hashValue", hash_value);

    nm(interp, "makeIterator", make_iterator);
    nm(interp, "reversed", reversed_round_trip);
    nm(interp, "contains", contains);
    nm(interp, "distance", distance);
    nm(interp, "index", index);
}

/// Build a `ReversedCollection` wrapping `base_items`.
pub fn make_reversed_collection(base_items: Vec<SwiftValue>) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "ReversedCollection".into(),
        fields: vec![("_base".into(), SwiftValue::Array(Rc::new(base_items)))],
    }))
}

/// Extract the base array items from a `ReversedCollection`.
fn base_items(recv: &SwiftValue) -> Result<Rc<Vec<SwiftValue>>, StdError> {
    match recv {
        SwiftValue::Struct(obj) if obj.type_name == "ReversedCollection" => {
            if let Some(SwiftValue::Array(items)) = obj.get("_base") {
                Ok(items.clone())
            } else {
                Err(StdError::Error(EvalError::Type(
                    "ReversedCollection missing _base".into(),
                )))
            }
        }
        other => Err(StdError::Error(EvalError::Type(format!(
            "expected ReversedCollection, got {}",
            other.type_name()
        )))),
    }
}

fn count(v: SwiftValue) -> StdResult {
    let items = base_items(&v)?;
    Ok(SwiftValue::int(items.len() as i128))
}

fn is_empty(v: SwiftValue) -> StdResult {
    let items = base_items(&v)?;
    Ok(SwiftValue::Bool(items.is_empty()))
}

/// `startIndex` — 0 (indices go 0..count in the reversed view).
fn start_index(v: SwiftValue) -> StdResult {
    base_items(&v)?;
    Ok(SwiftValue::int(0))
}

/// `endIndex` — count of elements.
fn end_index(v: SwiftValue) -> StdResult {
    let items = base_items(&v)?;
    Ok(SwiftValue::int(items.len() as i128))
}

/// `first` — first element in reversed order (= last of base), or nil.
fn first(v: SwiftValue) -> StdResult {
    let items = base_items(&v)?;
    Ok(items.last().cloned().unwrap_or(SwiftValue::Nil))
}

/// `last` — last element in reversed order (= first of base), or nil.
fn last(v: SwiftValue) -> StdResult {
    let items = base_items(&v)?;
    Ok(items.first().cloned().unwrap_or(SwiftValue::Nil))
}

/// `hashValue` — hash over the reversed elements.
fn hash_value(v: SwiftValue) -> StdResult {
    let items = base_items(&v)?;
    let mut h: u64 = 0;
    for (i, item) in items.iter().rev().enumerate() {
        h ^= crate::array::slice_stable_hash(item).wrapping_mul(i.wrapping_add(1) as u64);
    }
    Ok(SwiftValue::int(h as i64 as i128))
}

/// `makeIterator()` — no-op; returns self (for-in uses `materialize_builtin_sequence`).
fn make_iterator(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    Ok(Outcome {
        result: recv.clone(),
        receiver: recv,
    })
}

/// `reversed()` — round-trip: returns the original base array.
fn reversed_round_trip(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let items = base_items(&recv)?;
    Ok(Outcome {
        result: SwiftValue::Array(items),
        receiver: recv,
    })
}

/// `contains(_:)` — linear scan over reversed elements.
fn contains(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let items = base_items(&recv)?;
    let needle = args.into_iter().next().ok_or_else(|| {
        StdError::Error(EvalError::Type("contains(_:) requires an argument".into()))
    })?;
    let found = items.iter().any(|e| e == &needle);
    Ok(Outcome {
        result: SwiftValue::Bool(found),
        receiver: recv,
    })
}

/// `distance(from:to:)` — signed element distance in the reversed-view index space.
fn distance(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let n = base_items(&recv)?.len() as i128;
    let from = match args.first() {
        Some(SwiftValue::Int(i)) => i.raw,
        _ => {
            return Err(StdError::Error(EvalError::Type(
                "distance(from:to:) expects integer indices".into(),
            )))
        }
    };
    let to = match args.get(1) {
        Some(SwiftValue::Int(i)) => i.raw,
        _ => {
            return Err(StdError::Error(EvalError::Type(
                "distance(from:to:) expects integer indices".into(),
            )))
        }
    };
    if from < 0 || from > n {
        return Err(StdError::Error(EvalError::Trap(format!(
            "distance(from:to:) `from` index {from} out of bounds [0, {n}]"
        ))));
    }
    if to < 0 || to > n {
        return Err(StdError::Error(EvalError::Trap(format!(
            "distance(from:to:) `to` index {to} out of bounds [0, {n}]"
        ))));
    }
    Ok(Outcome {
        result: SwiftValue::int(to - from),
        receiver: recv,
    })
}

/// `index(_:offsetBy:)` — advance a reversed-view index.
fn index(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let n = base_items(&recv)?.len() as i128;
    let base = match args.first() {
        Some(SwiftValue::Int(i)) => i.raw,
        _ => {
            return Err(StdError::Error(EvalError::Type(
                "index(_:offsetBy:) expects integer base index".into(),
            )))
        }
    };
    let offset = match args.get(1) {
        Some(SwiftValue::Int(i)) => i.raw,
        _ => 0,
    };
    let result = base + offset;
    if result < 0 || result > n {
        return Err(StdError::Error(EvalError::Trap(
            "ReversedCollection index out of bounds".into(),
        )));
    }
    Ok(Outcome {
        result: SwiftValue::int(result),
        receiver: recv,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Ctx;
    impl tswift_core::StdContext for Ctx {
        fn call_closure(&mut self, _: usize, _: Vec<SwiftValue>) -> StdResult {
            Ok(SwiftValue::Nil)
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            unreachable!()
        }
    }

    fn rc(items: Vec<i128>) -> SwiftValue {
        make_reversed_collection(items.into_iter().map(SwiftValue::int).collect())
    }

    #[test]
    fn count_and_empty() {
        assert_eq!(count(rc(vec![1, 2, 3])).unwrap(), SwiftValue::int(3));
        assert_eq!(is_empty(rc(vec![])).unwrap(), SwiftValue::Bool(true));
        assert_eq!(is_empty(rc(vec![1])).unwrap(), SwiftValue::Bool(false));
    }

    #[test]
    fn first_last() {
        // rc([1,2,3]) reversed → [3,2,1]; first=3, last=1
        assert_eq!(first(rc(vec![1, 2, 3])).unwrap(), SwiftValue::int(3));
        assert_eq!(last(rc(vec![1, 2, 3])).unwrap(), SwiftValue::int(1));
        assert_eq!(first(rc(vec![])).unwrap(), SwiftValue::Nil);
    }

    #[test]
    fn reversed_round_trip_returns_base() {
        let v = rc(vec![1, 2, 3]);
        let out = reversed_round_trip(&mut Ctx, v, vec![]).unwrap().result;
        assert_eq!(
            out,
            SwiftValue::Array(Rc::new(vec![
                SwiftValue::int(1),
                SwiftValue::int(2),
                SwiftValue::int(3)
            ]))
        );
    }

    #[test]
    fn contains_finds_element() {
        let v = rc(vec![10, 20, 30]);
        let r = contains(&mut Ctx, v, vec![SwiftValue::int(20)])
            .unwrap()
            .result;
        assert_eq!(r, SwiftValue::Bool(true));
    }
}
