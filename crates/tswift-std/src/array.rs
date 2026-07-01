//! `Array` method intrinsics.

use std::rc::Rc;

use tswift_core::{
    collection_range_bounds, materialize_builtin_sequence, Arg, BuiltinReceiver, EvalError,
    Interpreter, LabeledMethodEntry, MethodEntry, Outcome, StdContext, StdError, StdResult,
    SwiftValue,
};

/// Register the `Array` intrinsics of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    let mutating = |interp: &mut Interpreter<'_>, name: &str, func: tswift_core::IntrinsicFn| {
        interp.register_intrinsic(
            BuiltinReceiver::Array,
            name,
            MethodEntry {
                mutating: true,
                func,
            },
        );
    };
    let nonmutating = |interp: &mut Interpreter<'_>, name: &str, func: tswift_core::IntrinsicFn| {
        interp.register_intrinsic(
            BuiltinReceiver::Array,
            name,
            MethodEntry {
                mutating: false,
                func,
            },
        );
    };
    let label_aware =
        |interp: &mut Interpreter<'_>, name: &str, func: tswift_core::LabeledIntrinsicFn| {
            interp.register_labeled_intrinsic(
                BuiltinReceiver::Array,
                name,
                LabeledMethodEntry {
                    mutating: true,
                    func,
                },
            );
        };
    label_aware(interp, "append", append_labeled);
    mutating(interp, "sort", sort);
    label_aware(interp, "insert", insert_labeled);
    mutating(interp, "remove", remove_at);
    mutating(interp, "removeLast", remove_last);
    mutating(interp, "popLast", pop_last);
    mutating(interp, "popFirst", pop_first);
    mutating(interp, "removeFirst", remove_first);
    mutating(interp, "removeAll", remove_all);
    mutating(interp, "removeSubrange", remove_subrange);
    mutating(interp, "reverse", reverse);
    mutating(interp, "reserveCapacity", reserve_capacity);
    mutating(interp, "replaceSubrange", replace_subrange);
    nonmutating(interp, "distance", distance);
    nonmutating(interp, "index", index);

    interp.register_property(BuiltinReceiver::Array, "count", count);
    interp.register_property(BuiltinReceiver::Array, "isEmpty", is_empty);
    interp.register_property(BuiltinReceiver::Array, "first", first);
    interp.register_property(BuiltinReceiver::Array, "last", last);
    interp.register_property(BuiltinReceiver::Array, "startIndex", start_index);
    interp.register_property(BuiltinReceiver::Array, "endIndex", end_index);
    interp.register_property(BuiltinReceiver::Array, "capacity", count);
    interp.register_property(BuiltinReceiver::Array, "description", description);
    interp.register_property(BuiltinReceiver::Array, "debugDescription", description);
    interp.register_property(BuiltinReceiver::Array, "hashValue", hash_value);
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
        None => Err(StdError::Error(EvalError::Type(format!(
            "{who} expects an index"
        )))),
    }
}

fn int_args(args: &[SwiftValue], who: &str) -> Result<Vec<i128>, StdError> {
    args.iter()
        .map(|a| match a {
            SwiftValue::Int(i) => Ok(i.raw),
            _ => Err(StdError::Error(EvalError::Type(format!(
                "{who} expects integer indexes"
            )))),
        })
        .collect()
}

fn ensure_index(index: i128, len: i128, who: &str) -> Result<(), StdError> {
    if (0..=len).contains(&index) {
        Ok(())
    } else {
        Err(StdError::Error(EvalError::Trap(format!(
            "{who} index {index} out of range"
        ))))
    }
}

fn range_bounds(range: &SwiftValue, len: usize, who: &str) -> Result<(usize, usize), StdError> {
    collection_range_bounds(range, len, who).map_err(StdError::Error)
}

fn array_arg(value: &SwiftValue, who: &str) -> Result<Vec<SwiftValue>, StdError> {
    match value {
        SwiftValue::Array(items) => Ok(items.as_ref().clone()),
        other => Err(StdError::Error(EvalError::Type(format!(
            "{who} expects replacement elements, got {}",
            other.type_name()
        )))),
    }
}

fn values(args: Vec<Arg>) -> Vec<SwiftValue> {
    args.into_iter().map(|a| a.value).collect()
}

fn contents_arg(args: &[Arg], method: &str) -> Result<Option<Vec<SwiftValue>>, StdError> {
    let mut contents = None;
    for arg in args {
        if arg.label.as_deref() == Some("contentsOf") {
            if contents.is_some() {
                return Err(StdError::Error(EvalError::Type(format!(
                    "Array.{method} called with duplicate contentsOf:"
                ))));
            }
            let value = materialize_builtin_sequence(&arg.value).ok_or_else(|| {
                StdError::Error(EvalError::Type(format!(
                    "cannot use {} as a sequence for contentsOf:",
                    arg.value.type_name()
                )))
            })?;
            contents = Some(value);
        }
    }
    Ok(contents)
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
    let element = args
        .into_iter()
        .next()
        .ok_or_else(|| StdError::Error(EvalError::Type("append expects one argument".into())))?;
    Rc::make_mut(&mut items).push(element);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Array(items),
    })
}

/// `Array.append(_:)` / `Array.append(contentsOf:)` — label-aware overloads.
fn append_labeled(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let Some(extra) = contents_arg(&args, "append")? else {
        return append(ctx, recv, values(args)).map(Some);
    };
    if args
        .iter()
        .any(|a| a.label.as_deref() != Some("contentsOf"))
    {
        return Err(StdError::Error(EvalError::Type(
            "Array.append(contentsOf:) takes only contentsOf:".into(),
        )));
    }
    let mut items = items(recv)?;
    Rc::make_mut(&mut items).extend(extra);
    Ok(Some(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Array(items),
    }))
}

/// `Array.sort()` / `Array.sort(by:)` — sort in place.
///
/// Mutating sibling of `sorted`: delegates to the shared sort so the natural
/// `Comparable` order and the `(by:)` comparator behave identically, then
/// writes the ordered array back through the receiver lvalue.
fn sort(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let elements = items(recv)?.as_ref().clone();
    let labeled: Vec<Arg> = args.into_iter().map(Arg::positional).collect();
    let ordered = crate::sequence::sorted(ctx, elements, labeled)?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: ordered,
    })
}

/// `Array.insert(_:at:)` — insert one element at an index.
fn insert(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
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
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Array(v),
    })
}

/// `Array.insert(_:at:)` / `Array.insert(contentsOf:at:)` label-aware overloads.
fn insert_labeled(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let Some(extra) = contents_arg(&args, "insert")? else {
        return insert(ctx, recv, values(args)).map(Some);
    };
    let mut at = None;
    for arg in &args {
        match arg.label.as_deref() {
            Some("contentsOf") => {}
            Some("at") => match &arg.value {
                SwiftValue::Int(i) if i.raw >= 0 && at.is_none() => at = Some(i.raw as usize),
                _ => {
                    return Err(StdError::Error(EvalError::Type(
                        "insert(contentsOf:at:) needs one non-negative at: index".into(),
                    )))
                }
            },
            _ => {
                return Err(StdError::Error(EvalError::Type(
                    "Array.insert(contentsOf:at:) takes contentsOf: and at:".into(),
                )))
            }
        }
    }
    let at = at.ok_or_else(|| {
        StdError::Error(EvalError::Type(
            "insert(contentsOf:at:) needs an at: index".into(),
        ))
    })?;
    let mut items = items(recv)?;
    if at > items.len() {
        return Err(StdError::Error(EvalError::Trap(format!(
            "insert index {at} out of range"
        ))));
    }
    Rc::make_mut(&mut items).splice(at..at, extra);
    Ok(Some(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Array(items),
    }))
}

/// `Array.remove(at:)` — remove and return the element at an index.
fn remove_at(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let mut v = items(recv)?;
    let at = index_arg(&args, "remove(at:)")?;
    if at >= v.len() {
        return Err(StdError::Error(EvalError::Trap(format!(
            "remove index {at} out of range"
        ))));
    }
    let removed = Rc::make_mut(&mut v).remove(at);
    Ok(Outcome {
        result: removed,
        receiver: SwiftValue::Array(v),
    })
}

/// `Array.removeLast()` — remove and return the final element.
fn remove_last(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _a: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let mut v = items(recv)?;
    let removed = Rc::make_mut(&mut v)
        .pop()
        .ok_or_else(|| StdError::Error(EvalError::Trap("removeLast on empty array".into())))?;
    Ok(Outcome {
        result: removed,
        receiver: SwiftValue::Array(v),
    })
}

/// `Array.popLast()` — remove and return the last element as `Optional`, or `nil` if empty.
fn pop_last(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _a: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let mut v = items(recv)?;
    let result = Rc::make_mut(&mut v).pop().unwrap_or(SwiftValue::Nil);
    Ok(Outcome {
        result,
        receiver: SwiftValue::Array(v),
    })
}

/// `Array.popFirst()` — remove and return the first element as `Optional`, or `nil` if empty.
fn pop_first(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _a: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let mut v = items(recv)?;
    let result = if v.is_empty() {
        SwiftValue::Nil
    } else {
        Rc::make_mut(&mut v).remove(0)
    };
    Ok(Outcome {
        result,
        receiver: SwiftValue::Array(v),
    })
}

/// `Array.removeFirst()` — remove and return the first element.
fn remove_first(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _a: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let mut v = items(recv)?;
    if v.is_empty() {
        return Err(StdError::Error(EvalError::Trap(
            "removeFirst on empty array".into(),
        )));
    }
    let removed = Rc::make_mut(&mut v).remove(0);
    Ok(Outcome {
        result: removed,
        receiver: SwiftValue::Array(v),
    })
}

/// `Array.removeAll(keepingCapacity:)` empties the array; the
/// `removeAll(where:)` overload (detected by a closure argument) instead drops
/// only the elements satisfying the predicate. `keepingCapacity` is a `Bool`,
/// so the two overloads are told apart by argument type.
fn remove_all(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let mut v = items(recv)?;
    if let Some(SwiftValue::Closure(id)) = args.iter().find(|a| matches!(a, SwiftValue::Closure(_)))
    {
        let id = *id;
        let mut kept = Vec::new();
        for elem in v.iter().cloned() {
            let keep = !ctx
                .call_closure(id, vec![elem.clone()])?
                .as_bool()
                .unwrap_or(false);
            if keep {
                kept.push(elem);
            }
        }
        return Ok(Outcome {
            result: SwiftValue::Void,
            receiver: SwiftValue::Array(Rc::new(kept)),
        });
    }
    Rc::make_mut(&mut v).clear();
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Array(v),
    })
}

/// `Array.removeSubrange(_:)` — delete the elements in a range, in place.
fn remove_subrange(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let mut v = items(recv)?;
    let range = args.first().ok_or_else(|| {
        StdError::Error(EvalError::Type("removeSubrange(_:) expects a range".into()))
    })?;
    let (start, end) = range_bounds(range, v.len(), "removeSubrange(_:)")?;
    Rc::make_mut(&mut v).drain(start..end);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Array(v),
    })
}

/// `Array.reverse()` — reverse the elements in place.
fn reverse(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _a: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let mut v = items(recv)?;
    Rc::make_mut(&mut v).reverse();
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Array(v),
    })
}

/// `Array.reserveCapacity(_:)` — a no-op on our `Vec`-backed arrays.
fn reserve_capacity(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _a: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

/// `Array.replaceSubrange(_:with:)` — splice replacement elements into place.
fn replace_subrange(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let mut v = items(recv)?;
    let [range, replacement] = args.as_slice() else {
        return Err(StdError::Error(EvalError::Type(
            "replaceSubrange(_:with:) expects a range and replacement elements".into(),
        )));
    };
    let (start, end) = range_bounds(range, v.len(), "replaceSubrange(_:with:)")?;
    let replacement = array_arg(replacement, "replaceSubrange(_:with:)")?;
    Rc::make_mut(&mut v).splice(start..end, replacement);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Array(v),
    })
}

/// `Array.distance(from:to:)` — integer indexes make distance simple subtraction.
fn distance(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let len = items(recv.clone())?.len() as i128;
    let indexes = int_args(&args, "distance(from:to:)")?;
    match indexes.as_slice() {
        [start, end] => {
            ensure_index(*start, len, "distance(from:to:)")?;
            ensure_index(*end, len, "distance(from:to:)")?;
            Ok(Outcome {
                result: SwiftValue::int(end - start),
                receiver: recv,
            })
        }
        _ => Err(StdError::Error(EvalError::Type(
            "distance(from:to:) expects two indexes".into(),
        ))),
    }
}

/// `Array.index` overloads over integer indexes.
fn index(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let len = items(recv.clone())?.len() as i128;
    let indexes = int_args(&args, "index")?;
    let result = match indexes.as_slice() {
        [i, distance] => {
            ensure_index(*i, len, "index")?;
            let next = i + distance;
            ensure_index(next, len, "index")?;
            SwiftValue::int(next)
        }
        [i, distance, limit] => {
            ensure_index(*i, len, "index")?;
            ensure_index(*limit, len, "index")?;
            let next = i + distance;
            let passed_limit = if *distance >= 0 {
                next > *limit
            } else {
                next < *limit
            };
            if passed_limit {
                SwiftValue::Nil
            } else {
                ensure_index(next, len, "index")?;
                SwiftValue::int(next)
            }
        }
        _ => {
            return Err(StdError::Error(EvalError::Type(
                "index expects two or three integer arguments".into(),
            )))
        }
    };
    Ok(Outcome {
        result,
        receiver: recv,
    })
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

fn description(recv: SwiftValue) -> StdResult {
    let _ = items(recv.clone())?;
    Ok(SwiftValue::Str(recv.to_string()))
}

fn hash_value(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(stable_hash(&recv) as i64 as i128))
}

fn stable_hash(value: &SwiftValue) -> u64 {
    fn mix(mut hash: u64, value: u64) -> u64 {
        hash ^= value;
        hash = hash.wrapping_mul(0x100_0000_01b3);
        hash.rotate_left(5)
    }

    match value {
        SwiftValue::Void => 0x01,
        SwiftValue::Nil => 0x02,
        SwiftValue::Bool(b) => u64::from(*b) + 0x10,
        SwiftValue::Int(i) => mix(0x20, i.raw as u64),
        SwiftValue::Double(d) => mix(0x30, d.to_bits()),
        SwiftValue::Str(s) => s.bytes().fold(0x40, |h, b| mix(h, b as u64)),
        SwiftValue::Tuple(items, _) => items.iter().fold(0x50, |h, v| mix(h, stable_hash(v))),
        SwiftValue::Array(items) => items.iter().fold(0x60, |h, v| mix(h, stable_hash(v))),
        SwiftValue::Dict(pairs) => pairs.iter().fold(0x70, |h, (k, v)| {
            mix(mix(h, stable_hash(k)), stable_hash(v))
        }),
        SwiftValue::Set(items) => items.iter().fold(0x80, |h, v| mix(h, stable_hash(v))),
        SwiftValue::Range { lo, hi, inclusive } => mix(
            mix(mix(0x90, *lo as u64), *hi as u64),
            u64::from(*inclusive),
        ),
        SwiftValue::Function(id)
        | SwiftValue::Closure(id)
        | SwiftValue::Task(id)
        | SwiftValue::TaskGroup(id)
        | SwiftValue::Continuation(id)
        | SwiftValue::StreamContinuation(id)
        | SwiftValue::AsyncStreamHandle(id) => mix(0xa0, *id as u64),
        SwiftValue::Struct(obj) => obj.fields.iter().fold(0xb0, |h, (name, field)| {
            mix(
                mix(h, stable_hash(&SwiftValue::Str(name.clone()))),
                stable_hash(field),
            )
        }),
        SwiftValue::Enum(obj) => obj.payload.iter().fold(
            mix(0xc0, stable_hash(&SwiftValue::Str(obj.case.clone()))),
            |h, payload| mix(h, stable_hash(payload)),
        ),
        SwiftValue::Object(obj) => Rc::as_ptr(obj) as usize as u64,
        SwiftValue::Weak(obj) => obj.as_ptr() as usize as u64,
        SwiftValue::Regex(r) => r.pattern().bytes().fold(0xd0, |h, b| mix(h, b as u64)),
        SwiftValue::Metatype(name) => name.bytes().fold(0xe0, |h, b| mix(h, b as u64)),
    }
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
        fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> tswift_core::StdResult {
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
    fn sort_orders_in_place_and_preserves_cow() {
        let mut ctx = MockCtx { sink: Vec::new() };
        let shared = Rc::new(vec![
            SwiftValue::int(3),
            SwiftValue::int(1),
            SwiftValue::int(2),
        ]);
        let recv = SwiftValue::Array(Rc::clone(&shared));

        let out = sort(&mut ctx, recv, vec![]).unwrap();

        assert_eq!(out.result, SwiftValue::Void);
        match out.receiver {
            SwiftValue::Array(items) => assert_eq!(
                items.as_slice(),
                &[SwiftValue::int(1), SwiftValue::int(2), SwiftValue::int(3)]
            ),
            other => panic!("expected array, got {other:?}"),
        }
        // The original storage is untouched (sort works on a value copy).
        assert_eq!(
            shared.as_slice(),
            &[SwiftValue::int(3), SwiftValue::int(1), SwiftValue::int(2)]
        );
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
        assert_eq!(end_index(arr.clone()).unwrap(), SwiftValue::int(2));
        assert_eq!(
            description(arr.clone()).unwrap(),
            SwiftValue::Str("[5, 6]".into())
        );
        assert_eq!(hash_value(arr.clone()).unwrap(), hash_value(arr).unwrap());
        assert_eq!(
            first(SwiftValue::Array(Rc::new(vec![]))).unwrap(),
            SwiftValue::Nil
        );
    }

    #[test]
    fn index_helpers_use_integer_indexes() {
        let mut ctx = MockCtx { sink: Vec::new() };
        let recv = SwiftValue::Array(Rc::new(vec![SwiftValue::int(1), SwiftValue::int(2)]));

        let out = distance(
            &mut ctx,
            recv.clone(),
            vec![SwiftValue::int(0), SwiftValue::int(2)],
        )
        .unwrap();
        assert_eq!(out.result, SwiftValue::int(2));
        assert_eq!(out.receiver, recv);

        let out = index(
            &mut ctx,
            SwiftValue::Array(Rc::new(vec![
                SwiftValue::int(0),
                SwiftValue::int(1),
                SwiftValue::int(2),
                SwiftValue::int(3),
            ])),
            vec![SwiftValue::int(1), SwiftValue::int(3)],
        )
        .unwrap();
        assert_eq!(out.result, SwiftValue::int(4));
    }

    #[test]
    fn replace_subrange_splices_replacement_elements() {
        let mut ctx = MockCtx { sink: Vec::new() };
        let shared = Rc::new(vec![
            SwiftValue::int(1),
            SwiftValue::int(2),
            SwiftValue::int(3),
            SwiftValue::int(4),
        ]);

        let out = replace_subrange(
            &mut ctx,
            SwiftValue::Array(Rc::clone(&shared)),
            vec![
                SwiftValue::Range {
                    lo: 1,
                    hi: 3,
                    inclusive: false,
                },
                SwiftValue::Array(Rc::new(vec![SwiftValue::int(8), SwiftValue::int(9)])),
            ],
        )
        .unwrap();

        assert_eq!(
            shared.as_slice(),
            &[
                SwiftValue::int(1),
                SwiftValue::int(2),
                SwiftValue::int(3),
                SwiftValue::int(4)
            ]
        );
        match out.receiver {
            SwiftValue::Array(items) => assert_eq!(
                items.as_slice(),
                &[
                    SwiftValue::int(1),
                    SwiftValue::int(8),
                    SwiftValue::int(9),
                    SwiftValue::int(4)
                ]
            ),
            other => panic!("expected array, got {other:?}"),
        }
    }

    #[test]
    fn append_rejects_non_array_receiver() {
        let mut ctx = MockCtx { sink: Vec::new() };
        let err = append(&mut ctx, SwiftValue::int(1), vec![SwiftValue::int(2)]).unwrap_err();
        assert!(matches!(err, StdError::Error(_)));
    }
}
