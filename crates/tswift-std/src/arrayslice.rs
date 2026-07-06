//! `ArraySlice` method and property intrinsics.
//!
//! An `ArraySlice` is represented as `SwiftValue::ArraySlice { base, start, end }`
//! where `base` is the full parent `Array` and `start`/`end` are element
//! indices into it (half-open `[start, end)`).
//!
//! **Index semantics** — indices on an `ArraySlice` are **base-relative**,
//! matching Swift: `a[i..<j].startIndex == i`.  All subscript operations
//! validate that the supplied index lies within `[start, end)`.
//!
//! **Dispatch** — `BuiltinReceiver::of(SwiftValue::ArraySlice) = ArraySlice`,
//! so ArraySlice values dispatch to the `ArraySlice.*` table.  Shared
//! mutating operations (append, insert, remove, etc.) are registered here and
//! detach the slice from its base on mutation (copy-on-write semantics).

use std::rc::Rc;

use tswift_core::{
    collection_range_bounds, Arg, BuiltinReceiver, EvalError, Interpreter, LabeledMethodEntry,
    MethodEntry, Outcome, StdContext, StdError, StdResult, SwiftValue,
};

/// Register all `ArraySlice` intrinsics.
pub fn install(interp: &mut Interpreter<'_>) {
    install_for(interp, BuiltinReceiver::ArraySlice);
}

/// Register ArraySlice intrinsics for the given receiver (for dual-registration).
pub fn install_for(interp: &mut Interpreter<'_>, recv: BuiltinReceiver) {
    let pure = |interp: &mut Interpreter<'_>, name: &str, func: tswift_core::IntrinsicFn| {
        interp.register_intrinsic(
            recv,
            name,
            MethodEntry {
                mutating: false,
                func,
            },
        );
    };
    let mutating = |interp: &mut Interpreter<'_>, name: &str, func: tswift_core::IntrinsicFn| {
        interp.register_intrinsic(
            recv,
            name,
            MethodEntry {
                mutating: true,
                func,
            },
        );
    };
    let label_aware =
        |interp: &mut Interpreter<'_>, name: &str, func: tswift_core::LabeledIntrinsicFn| {
            interp.register_labeled_intrinsic(
                recv,
                name,
                LabeledMethodEntry {
                    mutating: true,
                    func,
                },
            );
        };

    // Properties.
    interp.register_property(recv, "count", count);
    interp.register_property(recv, "isEmpty", is_empty);
    interp.register_property(recv, "first", first);
    interp.register_property(recv, "last", last);
    interp.register_property(recv, "startIndex", start_index);
    interp.register_property(recv, "endIndex", end_index);
    interp.register_property(recv, "capacity", count);
    interp.register_property(recv, "description", description);
    interp.register_property(recv, "debugDescription", description);
    interp.register_property(recv, "hashValue", hash_value);

    // Non-mutating methods.
    pure(interp, "distance", distance);
    pure(interp, "index", index);
    pure(interp, "contains", contains);
    pure(interp, "removeAll", remove_all_nonmutating);

    // Mutating methods — detach on mutation.
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
}

// ---- Helpers ----------------------------------------------------------------

/// Extract `(base, start, end)` from an `ArraySlice` receiver, materialising
/// the slice window as a plain `Vec` if needed.  Works for both `ArraySlice`
/// and `Array` (treating an `Array` as a full slice).
fn slice_fields(recv: &SwiftValue) -> Result<(&Vec<SwiftValue>, usize, usize), StdError> {
    match recv {
        SwiftValue::ArraySlice { base, start, end } => Ok((base.as_ref(), *start, *end)),
        SwiftValue::Array(items) => Ok((items.as_ref(), 0, items.len())),
        other => Err(StdError::Error(EvalError::Type(format!(
            "expected ArraySlice receiver, got {}",
            other.type_name()
        )))),
    }
}

/// Detach an ArraySlice: materialize its window into a fresh backing Vec
/// and return a 0-based `ArraySlice`.
fn detach(recv: &SwiftValue) -> Result<(Vec<SwiftValue>, usize), StdError> {
    let (base, start, end) = slice_fields(recv)?;
    let v = base[start..end].to_vec();
    let n = v.len();
    Ok((v, n))
}

/// Wrap a detached Vec back into an ArraySlice.
fn as_slice(items: Vec<SwiftValue>) -> SwiftValue {
    let n = items.len();
    SwiftValue::ArraySlice {
        base: Rc::new(items),
        start: 0,
        end: n,
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

/// Validate that `index` lies in the BASE-RELATIVE range `[sl_start, sl_end]`.
/// Used by `distance` and `index` which traffic in base coordinates.
fn ensure_slice_index(
    index: i128,
    sl_start: usize,
    sl_end: usize,
    who: &str,
) -> Result<(), StdError> {
    let lo = sl_start as i128;
    let hi = sl_end as i128;
    if (lo..=hi).contains(&index) {
        Ok(())
    } else {
        Err(StdError::Error(EvalError::Trap(format!(
            "{who}: index {index} out of slice [{sl_start},{sl_end}]"
        ))))
    }
}

fn range_bounds(range: &SwiftValue, len: usize, who: &str) -> Result<(usize, usize), StdError> {
    collection_range_bounds(range, len, who).map_err(StdError::Error)
}

fn index_arg_from(args: &[SwiftValue], who: &str) -> Result<usize, StdError> {
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

fn values_from_args(args: Vec<Arg>) -> Vec<SwiftValue> {
    args.into_iter().map(|a| a.value).collect()
}

fn contents_arg(args: &[Arg], method: &str) -> Result<Option<Vec<SwiftValue>>, StdError> {
    let mut contents = None;
    for arg in args {
        if arg.label.as_deref() == Some("contentsOf") {
            if contents.is_some() {
                return Err(StdError::Error(EvalError::Type(format!(
                    "ArraySlice.{method} called with duplicate contentsOf:"
                ))));
            }
            let value = tswift_core::materialize_builtin_sequence(&arg.value).ok_or_else(|| {
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

// ---- Properties -------------------------------------------------------------

fn count(recv: SwiftValue) -> StdResult {
    let (_, start, end) = slice_fields(&recv)?;
    Ok(SwiftValue::int((end - start) as i128))
}

fn is_empty(recv: SwiftValue) -> StdResult {
    let (_, start, end) = slice_fields(&recv)?;
    Ok(SwiftValue::Bool(start == end))
}

fn first(recv: SwiftValue) -> StdResult {
    let (base, start, end) = slice_fields(&recv)?;
    Ok(if start < end {
        base[start].clone()
    } else {
        SwiftValue::Nil
    })
}

fn last(recv: SwiftValue) -> StdResult {
    let (base, start, end) = slice_fields(&recv)?;
    Ok(if start < end {
        base[end - 1].clone()
    } else {
        SwiftValue::Nil
    })
}

fn start_index(recv: SwiftValue) -> StdResult {
    let (_, start, _) = slice_fields(&recv)?;
    Ok(SwiftValue::int(start as i128))
}

fn end_index(recv: SwiftValue) -> StdResult {
    let (_, _, end) = slice_fields(&recv)?;
    Ok(SwiftValue::int(end as i128))
}

fn description(recv: SwiftValue) -> StdResult {
    let _ = slice_fields(&recv)?;
    Ok(SwiftValue::Str(recv.to_string()))
}

fn hash_value(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(
        crate::array::slice_stable_hash(&recv) as i64 as i128,
    ))
}

// ---- Non-mutating methods ---------------------------------------------------

fn distance(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (_, sl_start, sl_end) = slice_fields(&recv)?;
    let indexes = int_args(&args, "distance(from:to:)")?;
    match indexes.as_slice() {
        [from, to] => {
            // Both indices must lie within [sl_start, sl_end] (base-relative).
            ensure_slice_index(*from, sl_start, sl_end, "distance(from:to:) 'from'")?;
            ensure_slice_index(*to, sl_start, sl_end, "distance(from:to:) 'to'")?;
            Ok(Outcome {
                result: SwiftValue::int(to - from),
                receiver: recv,
            })
        }
        _ => Err(StdError::Error(EvalError::Type(
            "distance(from:to:) expects two indexes".into(),
        ))),
    }
}

fn index(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (_, sl_start, sl_end) = slice_fields(&recv)?;
    let indexes = int_args(&args, "index")?;
    let result = match indexes.as_slice() {
        [i, distance] => {
            // Base index `i` must lie in [sl_start, sl_end].
            ensure_slice_index(*i, sl_start, sl_end, "index")?;
            let next = i + distance;
            // Result must also lie in [sl_start, sl_end].
            ensure_slice_index(next, sl_start, sl_end, "index result")?;
            SwiftValue::int(next)
        }
        [i, distance, limit] => {
            ensure_slice_index(*i, sl_start, sl_end, "index")?;
            ensure_slice_index(*limit, sl_start, sl_end, "index limit")?;
            let next = i + distance;
            let passed_limit = if *distance >= 0 {
                next > *limit
            } else {
                next < *limit
            };
            if passed_limit {
                SwiftValue::Nil
            } else {
                ensure_slice_index(next, sl_start, sl_end, "index result")?;
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

fn contains(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (base, start, end) = slice_fields(&recv)?;
    let needle = args.first().cloned().unwrap_or(SwiftValue::Nil);
    // Closure predicate?
    if let SwiftValue::Closure(id) = &needle {
        let id = *id;
        for item in base[start..end].iter().cloned() {
            if ctx.call_closure(id, vec![item])?.as_bool().unwrap_or(false) {
                return Ok(Outcome {
                    result: SwiftValue::Bool(true),
                    receiver: recv,
                });
            }
        }
        return Ok(Outcome {
            result: SwiftValue::Bool(false),
            receiver: recv,
        });
    }
    let found = base[start..end].iter().any(|el| el == &needle);
    Ok(Outcome {
        result: SwiftValue::Bool(found),
        receiver: recv,
    })
}

/// Non-mutating stub for `removeAll` when used as an expression.
fn remove_all_nonmutating(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

// ---- Mutating methods (detach-on-write) -------------------------------------

fn append(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let element = args
        .into_iter()
        .next()
        .ok_or_else(|| StdError::Error(EvalError::Type("append expects one argument".into())))?;
    let (mut v, _) = detach(&recv)?;
    v.push(element);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: as_slice(v),
    })
}

fn append_labeled(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let Some(extra) = contents_arg(&args, "append")? else {
        return append(ctx, recv, values_from_args(args)).map(Some);
    };
    // `append(contentsOf:)` takes only the contentsOf: argument.
    if args
        .iter()
        .any(|a| a.label.as_deref() != Some("contentsOf"))
    {
        return Err(StdError::Error(EvalError::Type(
            "ArraySlice.append(contentsOf:) takes only contentsOf:".into(),
        )));
    }
    let mut v = detach(&recv)?.0;
    v.extend(extra);
    Ok(Some(Outcome {
        result: SwiftValue::Void,
        receiver: as_slice(v),
    }))
}

fn sort(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (v, _) = detach(&recv)?;
    let labeled: Vec<Arg> = args.into_iter().map(Arg::positional).collect();
    let ordered = crate::sequence::sorted(ctx, v, labeled)?;
    // sorted returns SwiftValue::Array; wrap as slice
    let items = match ordered {
        SwiftValue::Array(rc) => rc.as_ref().clone(),
        other => {
            return Err(StdError::Error(EvalError::Type(format!(
                "sort expected Array from sorted, got {}",
                other.type_name()
            ))))
        }
    };
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: as_slice(items),
    })
}

fn insert(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (_, sl_start, sl_end) = slice_fields(&recv)?;
    let element = args
        .first()
        .cloned()
        .ok_or_else(|| StdError::Error(EvalError::Type("insert expects an element".into())))?;
    // `at` is a base-relative index; valid range is [sl_start, sl_end] (insert at endIndex is append).
    let at_base = index_arg_from(&args[1..], "insert(_:at:)")?;
    if at_base < sl_start || at_base > sl_end {
        return Err(StdError::Error(EvalError::Trap(format!(
            "insert index {at_base} out of ArraySlice [{sl_start},{sl_end}]"
        ))));
    }
    let local_at = at_base - sl_start;
    // Detach AFTER validation so the error message uses the original indices.
    let (base, start, end) = slice_fields(&recv)?;
    let mut v = base[start..end].to_vec();
    v.insert(local_at, element);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: as_slice(v),
    })
}

fn insert_labeled(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let Some(extra) = contents_arg(&args, "insert")? else {
        return insert(ctx, recv, values_from_args(args)).map(Some);
    };
    let (_, sl_start, sl_end) = slice_fields(&recv)?;
    let mut at_base: Option<usize> = None;
    for arg in &args {
        match arg.label.as_deref() {
            Some("contentsOf") => {}
            Some("at") => match &arg.value {
                SwiftValue::Int(i) if i.raw >= 0 && at_base.is_none() => {
                    at_base = Some(i.raw as usize)
                }
                _ => {
                    return Err(StdError::Error(EvalError::Type(
                        "insert(contentsOf:at:) needs one non-negative at: index".into(),
                    )))
                }
            },
            _ => {
                return Err(StdError::Error(EvalError::Type(
                    "ArraySlice.insert(contentsOf:at:) takes contentsOf: and at:".into(),
                )))
            }
        }
    }
    let at_base = at_base.ok_or_else(|| {
        StdError::Error(EvalError::Type(
            "insert(contentsOf:at:) needs an at: index".into(),
        ))
    })?;
    // `at_base` is base-relative; valid range [sl_start, sl_end].
    if at_base < sl_start || at_base > sl_end {
        return Err(StdError::Error(EvalError::Trap(format!(
            "insert index {at_base} out of ArraySlice [{sl_start},{sl_end}]"
        ))));
    }
    let local_at = at_base - sl_start;
    let (base, start, end) = slice_fields(&recv)?;
    let mut v = base[start..end].to_vec();
    v.splice(local_at..local_at, extra);
    Ok(Some(Outcome {
        result: SwiftValue::Void,
        receiver: as_slice(v),
    }))
}

fn remove_at(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    // Base-relative index -> translate to local offset.
    let (base, sl_start, sl_end) = slice_fields(&recv)?;
    let i = index_arg_from(&args, "remove(at:)")?;
    if i < sl_start || i >= sl_end {
        return Err(StdError::Error(EvalError::Trap(format!(
            "remove index {i} out of ArraySlice [{sl_start},{sl_end})"
        ))));
    }
    let mut v = base[sl_start..sl_end].to_vec();
    let local = i - sl_start;
    let removed = v.remove(local);
    Ok(Outcome {
        result: removed,
        receiver: as_slice(v),
    })
}

fn remove_last(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (mut v, _) = detach(&recv)?;
    let removed = v
        .pop()
        .ok_or_else(|| StdError::Error(EvalError::Trap("removeLast on empty ArraySlice".into())))?;
    Ok(Outcome {
        result: removed,
        receiver: as_slice(v),
    })
}

fn pop_last(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (mut v, _) = detach(&recv)?;
    let result = v.pop().unwrap_or(SwiftValue::Nil);
    Ok(Outcome {
        result,
        receiver: as_slice(v),
    })
}

fn pop_first(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (mut v, n) = detach(&recv)?;
    let result = if n == 0 { SwiftValue::Nil } else { v.remove(0) };
    Ok(Outcome {
        result,
        receiver: as_slice(v),
    })
}

fn remove_first(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (mut v, n) = detach(&recv)?;
    if n == 0 {
        return Err(StdError::Error(EvalError::Trap(
            "removeFirst on empty ArraySlice".into(),
        )));
    }
    let removed = v.remove(0);
    Ok(Outcome {
        result: removed,
        receiver: as_slice(v),
    })
}

fn remove_all(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if let Some(SwiftValue::Closure(id)) = args.iter().find(|a| matches!(a, SwiftValue::Closure(_)))
    {
        let id = *id;
        let (v, _) = detach(&recv)?;
        let mut kept = Vec::new();
        for elem in v {
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
            receiver: as_slice(kept),
        });
    }
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: as_slice(Vec::new()),
    })
}

fn remove_subrange(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (base, sl_start, sl_end) = slice_fields(&recv)?;
    let range = args.first().ok_or_else(|| {
        StdError::Error(EvalError::Type("removeSubrange(_:) expects a range".into()))
    })?;
    let mut v = base[sl_start..sl_end].to_vec();
    // Translate base-relative range to local coords.
    let local_range = translate_range(range, sl_start);
    let (start, end) = range_bounds(&local_range, v.len(), "removeSubrange(_:)")?;
    v.drain(start..end);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: as_slice(v),
    })
}

fn reverse(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (mut v, _) = detach(&recv)?;
    v.reverse();
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: as_slice(v),
    })
}

fn reserve_capacity(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

fn replace_subrange(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (base, sl_start, sl_end) = slice_fields(&recv)?;
    let [range, replacement] = args.as_slice() else {
        return Err(StdError::Error(EvalError::Type(
            "replaceSubrange(_:with:) expects a range and replacement elements".into(),
        )));
    };
    let mut v = base[sl_start..sl_end].to_vec();
    let local_range = translate_range(range, sl_start);
    let (start, end) = range_bounds(&local_range, v.len(), "replaceSubrange(_:with:)")?;
    let replacement_items = match replacement {
        SwiftValue::Array(rc) => rc.as_ref().clone(),
        SwiftValue::ArraySlice {
            base: rb,
            start: rs,
            end: re,
        } => rb[*rs..*re].to_vec(),
        other => {
            return Err(StdError::Error(EvalError::Type(format!(
                "replaceSubrange expects replacement Array, got {}",
                other.type_name()
            ))))
        }
    };
    v.splice(start..end, replacement_items);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: as_slice(v),
    })
}

/// Translate a base-relative range into a slice-local range.
/// e.g. base offset 2, range [3, 5) → local [1, 3).
fn translate_range(range: &SwiftValue, sl_start: usize) -> SwiftValue {
    match range {
        SwiftValue::Range { lo, hi, inclusive } => SwiftValue::Range {
            lo: lo - sl_start as i128,
            hi: hi - sl_start as i128,
            inclusive: *inclusive,
        },
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(items: Vec<SwiftValue>, start: usize, end: usize) -> SwiftValue {
        SwiftValue::ArraySlice {
            base: Rc::new(items),
            start,
            end,
        }
    }

    fn iv(n: i128) -> SwiftValue {
        SwiftValue::int(n)
    }

    #[test]
    fn count_and_empty_base_relative() {
        // base [0,1,2,3,4], slice [2..4)
        let s = slice(vec![iv(0), iv(1), iv(2), iv(3), iv(4)], 2, 4);
        assert_eq!(count(s.clone()).unwrap(), iv(2));
        assert_eq!(is_empty(s.clone()).unwrap(), SwiftValue::Bool(false));
        assert_eq!(first(s.clone()).unwrap(), iv(2));
        assert_eq!(last(s.clone()).unwrap(), iv(3));
        assert_eq!(start_index(s.clone()).unwrap(), iv(2));
        assert_eq!(end_index(s.clone()).unwrap(), iv(4));
    }

    #[test]
    fn count_empty_slice() {
        let s = slice(vec![iv(1), iv(2)], 1, 1);
        assert_eq!(count(s.clone()).unwrap(), iv(0));
        assert_eq!(is_empty(s.clone()).unwrap(), SwiftValue::Bool(true));
        assert_eq!(first(s).unwrap(), SwiftValue::Nil);
    }

    struct MockCtx;
    impl StdContext for MockCtx {
        fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> tswift_core::StdResult {
            Ok(SwiftValue::Void)
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            unreachable!()
        }
    }

    #[test]
    fn append_detaches() {
        let mut ctx = MockCtx;
        let s = slice(vec![iv(10), iv(20), iv(30)], 1, 3); // [20,30]
        let out = append(&mut ctx, s, vec![iv(40)]).unwrap();
        // Detached: starts at 0.
        assert!(matches!(
            out.receiver,
            SwiftValue::ArraySlice { start: 0, .. }
        ));
        assert_eq!(count(out.receiver).unwrap(), iv(3));
    }

    #[test]
    fn remove_at_base_relative() {
        let mut ctx = MockCtx;
        // base [0,1,2,3,4], slice [1..4) = [1,2,3]; remove at index 2
        let s = slice(vec![iv(0), iv(1), iv(2), iv(3), iv(4)], 1, 4);
        let out = remove_at(&mut ctx, s, vec![iv(2)]).unwrap();
        assert_eq!(out.result, iv(2));
        assert_eq!(count(out.receiver).unwrap(), iv(2));
    }

    #[test]
    fn remove_at_oob_traps() {
        let mut ctx = MockCtx;
        let s = slice(vec![iv(0), iv(1), iv(2)], 1, 2);
        // index 0 is outside slice [1,2)
        let err = remove_at(&mut ctx, s, vec![iv(0)]).unwrap_err();
        assert!(matches!(err, StdError::Error(EvalError::Trap(_))));
    }

    #[test]
    fn description_renders_slice() {
        let s = slice(vec![iv(1), iv(2), iv(3)], 1, 3);
        assert_eq!(description(s).unwrap(), SwiftValue::Str("[2, 3]".into()));
    }

    // ---- Fix 1: distance/index validate against slice bounds (base-relative) ----

    #[test]
    fn distance_uses_base_relative_bounds() {
        let mut ctx = MockCtx;
        // base [10,20,30,40,50], slice [2..4) = [30,40]; valid indices 2..=4
        let s = slice(vec![iv(10), iv(20), iv(30), iv(40), iv(50)], 2, 4);
        let out = distance(&mut ctx, s.clone(), vec![iv(2), iv(4)]).unwrap();
        assert_eq!(out.result, iv(2));
        // index 0 is below sl_start=2 → trap
        let err = distance(&mut ctx, s.clone(), vec![iv(0), iv(4)]).unwrap_err();
        assert!(matches!(err, StdError::Error(EvalError::Trap(_))));
        // index 5 is above sl_end=4 → trap
        let err = distance(&mut ctx, s, vec![iv(2), iv(5)]).unwrap_err();
        assert!(matches!(err, StdError::Error(EvalError::Trap(_))));
    }

    #[test]
    fn index_uses_base_relative_bounds() {
        let mut ctx = MockCtx;
        // slice [3..6) of base len 7
        let base = vec![iv(0), iv(1), iv(2), iv(3), iv(4), iv(5), iv(6)];
        let s = slice(base, 3, 6);
        // index(3, offsetBy: 2) = 5 — valid
        let out = index(&mut ctx, s.clone(), vec![iv(3), iv(2)]).unwrap();
        assert_eq!(out.result, iv(5));
        // index(0, offsetBy: 1) — base index 0 < sl_start=3 → trap
        let err = index(&mut ctx, s.clone(), vec![iv(0), iv(1)]).unwrap_err();
        assert!(matches!(err, StdError::Error(EvalError::Trap(_))));
        // index(3, offsetBy: 4) = 7 > sl_end=6 → trap
        let err = index(&mut ctx, s, vec![iv(3), iv(4)]).unwrap_err();
        assert!(matches!(err, StdError::Error(EvalError::Trap(_))));
    }

    // ---- Fix 2: insert translates base-relative index to slice-local ----------

    #[test]
    fn insert_at_start_of_non_zero_slice() {
        let mut ctx = MockCtx;
        // base [0,1,2,3,4], slice [2..4) = [2,3]
        // insert(99, at: 2) → sl_start=2, local=0 → [99,2,3]
        let s = slice(vec![iv(0), iv(1), iv(2), iv(3), iv(4)], 2, 4);
        let out = insert(&mut ctx, s, vec![iv(99), iv(2)]).unwrap();
        assert_eq!(count(out.receiver.clone()).unwrap(), iv(3));
        // After detach the first element is 99.
        if let SwiftValue::ArraySlice {
            base,
            start,
            end: _,
        } = &out.receiver
        {
            assert_eq!(base[*start], iv(99));
            assert_eq!(base[*start + 1], iv(2));
        } else {
            panic!("expected ArraySlice");
        }
    }

    #[test]
    fn insert_at_end_of_non_zero_slice() {
        let mut ctx = MockCtx;
        // base [0,1,2,3], slice [1..3) = [1,2]
        // insert(88, at: 3) = endIndex → appends → [1,2,88]
        let s = slice(vec![iv(0), iv(1), iv(2), iv(3)], 1, 3);
        let out = insert(&mut ctx, s, vec![iv(88), iv(3)]).unwrap();
        assert_eq!(count(out.receiver.clone()).unwrap(), iv(3));
        if let SwiftValue::ArraySlice {
            base,
            start: _,
            end,
        } = &out.receiver
        {
            assert_eq!(base[end - 1], iv(88));
        } else {
            panic!("expected ArraySlice");
        }
    }

    #[test]
    fn insert_out_of_slice_bounds_traps() {
        let mut ctx = MockCtx;
        // slice [2..4), inserting at base index 1 (below sl_start) → trap
        let s = slice(vec![iv(0), iv(1), iv(2), iv(3), iv(4)], 2, 4);
        let err = insert(&mut ctx, s, vec![iv(99), iv(1)]).unwrap_err();
        assert!(matches!(err, StdError::Error(EvalError::Trap(_))));
    }

    // ---- Fix 4: append(contentsOf:) rejects extra/mislabeled args ------------

    #[test]
    fn append_contents_of_rejects_extra_args() {
        let mut ctx = MockCtx;
        let s = slice(vec![iv(1), iv(2)], 0, 2);
        let args = vec![
            Arg {
                label: Some("contentsOf".into()),
                value: SwiftValue::Array(std::rc::Rc::new(vec![iv(3)])),

                static_ty: None,
            },
            Arg {
                label: None, // extra positional arg
                value: iv(4),

                static_ty: None,
            },
        ];
        let err = append_labeled(&mut ctx, s, args).unwrap_err();
        assert!(matches!(err, StdError::Error(EvalError::Type(_))));
    }
}
