//! `Array` method intrinsics.

use std::rc::Rc;

use qswift_core::{
    BuiltinReceiver, EvalError, Interpreter, MethodEntry, Outcome, StdContext, StdError, StdResult,
    SwiftValue,
};

/// Register the `Array` intrinsics of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    let m = |interp: &mut Interpreter<'_>, name: &str, func: qswift_core::IntrinsicFn| {
        interp.register_intrinsic(BuiltinReceiver::Array, name, MethodEntry { mutating: true, func });
    };
    m(interp, "append", append);
    m(interp, "insert", insert);
    m(interp, "remove", remove_at);
    m(interp, "removeLast", remove_last);
    m(interp, "removeFirst", remove_first);
    m(interp, "removeAll", remove_all);
    m(interp, "reserveCapacity", reserve_capacity);

    interp.register_property(BuiltinReceiver::Array, "count", count);
    interp.register_property(BuiltinReceiver::Array, "isEmpty", is_empty);
    interp.register_property(BuiltinReceiver::Array, "first", first);
    interp.register_property(BuiltinReceiver::Array, "last", last);
    interp.register_property(BuiltinReceiver::Array, "startIndex", start_index);
    interp.register_property(BuiltinReceiver::Array, "endIndex", end_index);
    interp.register_property(BuiltinReceiver::Array, "capacity", count);
}

/// Unwrap an array receiver into its backing `Rc<Vec>`.
fn items(recv: SwiftValue) -> Result<Rc<Vec<SwiftValue>>, StdError> {
    match recv {
        SwiftValue::Array(v) => Ok(v),
        other => Err(StdError::Error(EvalError::Type(format!(
            "expected an array receiver, got {}",
            other.type_name()
        )))),
    }
}

fn index_arg(args: &[SwiftValue], who: &str) -> Result<usize, StdError> {
    match args.iter().rev().find_map(|a| match a {
        SwiftValue::Int(i) if i.raw >= 0 => Some(i.raw as usize),
        _ => None,
    }) {
        Some(i) => Ok(i),
        None => Err(StdError::Error(EvalError::Type(format!("{who} expects an index")))),
    }
}

/// `Array.append(_:)` — push one element onto the end, in place.
///
/// Mutating: the receiver arrives by value and the updated array is returned for
/// the dispatcher to write back. Copy-on-write is preserved by `Rc::make_mut`,
/// so a shared copy is cloned only when actually mutated.
fn append(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let SwiftValue::Array(mut items) = recv else {
        return Err(StdError::Error(EvalError::Type(
            "append called on a non-array receiver".into(),
        )));
    };
    let element = args.into_iter().next().ok_or_else(|| {
        StdError::Error(EvalError::Type("append expects one argument".into()))
    })?;
    Rc::make_mut(&mut items).push(element);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Array(items),
    })
}

/// `Array.insert(_:at:)` — insert one element at an index.
fn insert(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Result<Outcome, StdError> {
    let mut v = items(recv)?;
    let element = args
        .first()
        .cloned()
        .ok_or_else(|| StdError::Error(EvalError::Type("insert expects an element".into())))?;
    let at = index_arg(&args[1..], "insert(_:at:)")?;
    if at > v.len() {
        return Err(StdError::Error(EvalError::Trap(format!(
            "insert index {at} out of range"
        ))));
    }
    Rc::make_mut(&mut v).insert(at, element);
    Ok(Outcome { result: SwiftValue::Void, receiver: SwiftValue::Array(v) })
}

/// `Array.remove(at:)` — remove and return the element at an index.
fn remove_at(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Result<Outcome, StdError> {
    let mut v = items(recv)?;
    let at = index_arg(&args, "remove(at:)")?;
    if at >= v.len() {
        return Err(StdError::Error(EvalError::Trap(format!(
            "remove index {at} out of range"
        ))));
    }
    let removed = Rc::make_mut(&mut v).remove(at);
    Ok(Outcome { result: removed, receiver: SwiftValue::Array(v) })
}

/// `Array.removeLast()` — remove and return the final element.
fn remove_last(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Result<Outcome, StdError> {
    let mut v = items(recv)?;
    let removed = Rc::make_mut(&mut v)
        .pop()
        .ok_or_else(|| StdError::Error(EvalError::Trap("removeLast on empty array".into())))?;
    Ok(Outcome { result: removed, receiver: SwiftValue::Array(v) })
}

/// `Array.removeFirst()` — remove and return the first element.
fn remove_first(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Result<Outcome, StdError> {
    let mut v = items(recv)?;
    if v.is_empty() {
        return Err(StdError::Error(EvalError::Trap("removeFirst on empty array".into())));
    }
    let removed = Rc::make_mut(&mut v).remove(0);
    Ok(Outcome { result: removed, receiver: SwiftValue::Array(v) })
}

/// `Array.removeAll(keepingCapacity:)` — empty the array (capacity ignored).
fn remove_all(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Result<Outcome, StdError> {
    let mut v = items(recv)?;
    Rc::make_mut(&mut v).clear();
    Ok(Outcome { result: SwiftValue::Void, receiver: SwiftValue::Array(v) })
}

/// `Array.reserveCapacity(_:)` — a no-op on our `Vec`-backed arrays.
fn reserve_capacity(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Result<Outcome, StdError> {
    Ok(Outcome { result: SwiftValue::Void, receiver: recv })
}

// ---- properties ------------------------------------------------------------

fn count(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(items(recv)?.len() as i128))
}

fn is_empty(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(items(recv)?.is_empty()))
}

/// `first` / `last` are `Optional`; an empty array yields `nil`.
fn first(recv: SwiftValue) -> StdResult {
    Ok(items(recv)?.first().cloned().unwrap_or(SwiftValue::Nil))
}

fn last(recv: SwiftValue) -> StdResult {
    Ok(items(recv)?.last().cloned().unwrap_or(SwiftValue::Nil))
}

fn start_index(_recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(0))
}

fn end_index(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(items(recv)?.len() as i128))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `StdContext` that satisfies the trait without an interpreter, so
    /// intrinsics can be unit-tested in isolation against the seam.
    struct MockCtx {
        sink: Vec<u8>,
    }

    impl StdContext for MockCtx {
        fn call_closure(
            &mut self,
            _id: usize,
            _args: Vec<SwiftValue>,
        ) -> qswift_core::StdResult {
            Ok(SwiftValue::Void)
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            &mut self.sink
        }
    }

    #[test]
    fn append_pushes_and_returns_updated_receiver() {
        let mut ctx = MockCtx { sink: Vec::new() };
        let recv = SwiftValue::Array(Rc::new(vec![SwiftValue::int(1), SwiftValue::int(2)]));
        let out = append(&mut ctx, recv, vec![SwiftValue::int(3)]).unwrap();

        assert_eq!(out.result, SwiftValue::Void);
        match out.receiver {
            SwiftValue::Array(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[2], SwiftValue::int(3));
            }
            other => panic!("expected array, got {other:?}"),
        }
    }

    #[test]
    fn append_preserves_copy_on_write() {
        let mut ctx = MockCtx { sink: Vec::new() };
        let shared = Rc::new(vec![SwiftValue::int(1)]);
        let recv = SwiftValue::Array(Rc::clone(&shared));

        let out = append(&mut ctx, recv, vec![SwiftValue::int(2)]).unwrap();

        // The original storage is untouched; the mutation cloned it (CoW).
        assert_eq!(shared.as_slice(), &[SwiftValue::int(1)]);
        match out.receiver {
            SwiftValue::Array(items) => assert_eq!(items.len(), 2),
            other => panic!("expected array, got {other:?}"),
        }
    }

    #[test]
    fn insert_and_remove_preserve_cow() {
        let mut ctx = MockCtx { sink: Vec::new() };
        let shared = Rc::new(vec![SwiftValue::int(1), SwiftValue::int(2)]);

        let inserted = insert(
            &mut ctx,
            SwiftValue::Array(Rc::clone(&shared)),
            vec![SwiftValue::int(9), SwiftValue::int(0)],
        )
        .unwrap();
        // Original storage untouched (CoW cloned on mutation).
        assert_eq!(shared.as_slice(), &[SwiftValue::int(1), SwiftValue::int(2)]);
        match inserted.receiver {
            SwiftValue::Array(v) => assert_eq!(v[0], SwiftValue::int(9)),
            other => panic!("expected array, got {other:?}"),
        }

        let out = remove_at(
            &mut ctx,
            SwiftValue::Array(Rc::clone(&shared)),
            vec![SwiftValue::int(0)],
        )
        .unwrap();
        assert_eq!(out.result, SwiftValue::int(1));
        assert_eq!(shared.len(), 2, "original untouched after remove");
    }

    #[test]
    fn properties_report_shape() {
        let arr = SwiftValue::Array(Rc::new(vec![SwiftValue::int(5), SwiftValue::int(6)]));
        assert_eq!(count(arr.clone()).unwrap(), SwiftValue::int(2));
        assert_eq!(is_empty(arr.clone()).unwrap(), SwiftValue::Bool(false));
        assert_eq!(first(arr.clone()).unwrap(), SwiftValue::int(5));
        assert_eq!(last(arr.clone()).unwrap(), SwiftValue::int(6));
        assert_eq!(end_index(arr).unwrap(), SwiftValue::int(2));
        assert_eq!(first(SwiftValue::Array(Rc::new(vec![]))).unwrap(), SwiftValue::Nil);
    }

    #[test]
    fn append_rejects_non_array_receiver() {
        let mut ctx = MockCtx { sink: Vec::new() };
        let err = append(&mut ctx, SwiftValue::int(1), vec![SwiftValue::int(2)]).unwrap_err();
        assert!(matches!(err, StdError::Error(_)));
    }
}
