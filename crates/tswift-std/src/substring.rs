//! `Substring` method and property intrinsics.
//!
//! A `Substring` is represented as `SwiftValue::Substring { base, start, end }`
//! where `base` is the full parent `String` and `start`/`end` are
//! grapheme-cluster offsets into it (half-open `[start, end)`).
//!
//! **Index semantics** — indices on a `Substring` are **base-relative**,
//! matching Swift: `s[i..<j].startIndex == i`.  All index/distance operations
//! validate that the supplied `String.Index` lies within `[start, end]`.
//!
//! **Dispatch** — `BuiltinReceiver::of(SwiftValue::Substring) = Substring`, so
//! Substring values now dispatch to the `Substring.*` table rather than
//! falling through to `String.*`.  Shared text-extraction methods (lowercased,
//! hasPrefix, split, …) are registered under **both** receivers via
//! `string::install_shared_text_methods`.  Substring-unique members (`base`,
//! `isContiguousUTF8`, `characters`, `makeContiguousUTF8`) are registered
//! **only** under `Substring` so `"abc".base` correctly errors.

use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, EvalError, Interpreter, LabeledMethodEntry, MethodEntry, Outcome,
    StdContext, StdError, StdResult, SwiftValue,
};

use crate::string::{index_offset, install_shared_text_methods, make_index, str_of};
use tswift_core::graphemes;

/// Register all `Substring` intrinsics.
pub fn install(interp: &mut Interpreter<'_>) {
    let sub = BuiltinReceiver::Substring;

    // ---- Shared text-extraction methods (lowercased, hasPrefix, split, …) ----
    install_shared_text_methods(interp, sub);

    // ---- Substring-specific properties ----------------------------------------
    // These have different semantics than String (base-relative indices, views).
    interp.register_property(sub, "startIndex", sub_start_index);
    interp.register_property(sub, "endIndex", sub_end_index);
    interp.register_property(sub, "indices", sub_indices);

    // `base` — ONLY under Substring; must NOT be callable on String.
    interp.register_property(sub, "base", base);
    interp.register_property(sub, "isContiguousUTF8", is_contiguous_utf8);
    interp.register_property(sub, "characters", characters);

    // ---- Substring-specific labeled intrinsics --------------------------------
    interp.register_labeled_intrinsic(
        sub,
        "index",
        LabeledMethodEntry {
            mutating: false,
            func: sub_index_labeled,
        },
    );
    interp.register_labeled_intrinsic(
        sub,
        "distance",
        LabeledMethodEntry {
            mutating: false,
            func: sub_distance_labeled,
        },
    );
    // `formIndex` is intercepted by the dispatcher (inout write-back); the
    // registration records coverage and serves as a fallback delegating to
    // `index`.
    interp.register_labeled_intrinsic(
        sub,
        "formIndex",
        LabeledMethodEntry {
            mutating: true,
            func: sub_form_index_labeled,
        },
    );
    interp.register_labeled_intrinsic(
        sub,
        "replaceSubrange",
        LabeledMethodEntry {
            mutating: true,
            func: sub_replace_subrange,
        },
    );

    // ---- Substring-specific non-mutating methods ------------------------------
    let mut pure = |name: &str, f: tswift_core::IntrinsicFn| {
        interp.register_intrinsic(
            sub,
            name,
            MethodEntry {
                mutating: false,
                func: f,
            },
        );
    };
    pure("prefix", sub_prefix);
    pure("suffix", sub_suffix);

    // ---- Substring-specific mutating methods ----------------------------------
    interp.register_intrinsic(
        sub,
        "makeContiguousUTF8",
        MethodEntry {
            mutating: true,
            func: make_contiguous_utf8,
        },
    );
}

// ---- Helper: extract Substring fields ----------------------------------------

/// Borrow `(base, start, end)` from a `SwiftValue::Substring`.
fn sub_fields(recv: &SwiftValue) -> Result<(&String, usize, usize), StdError> {
    match recv {
        SwiftValue::Substring { base, start, end } => Ok((base.as_ref(), *start, *end)),
        _ => Err(StdError::Error(EvalError::Type(format!(
            "expected Substring, got {}",
            recv.type_name()
        )))),
    }
}

/// Helper for index operations: convert `String.Index`-valued `Arg` at the
/// given label, with a description of what failed.
fn labeled_index<'a>(args: &'a [Arg], label: &str) -> Option<&'a SwiftValue> {
    args.iter()
        .find(|a| a.label.as_deref() == Some(label))
        .map(|a| &a.value)
}

fn type_err(msg: String) -> StdError {
    StdError::Error(EvalError::Type(msg))
}

fn trap_err(msg: String) -> StdError {
    StdError::Error(EvalError::Trap(msg))
}

fn val(result: SwiftValue, receiver: SwiftValue) -> Result<Outcome, StdError> {
    Ok(Outcome { result, receiver })
}

// ---- Properties unique to Substring ------------------------------------------

/// `Substring.startIndex` — the grapheme-cluster offset of the first element
/// in this slice, expressed in the base string's coordinate space.
fn sub_start_index(recv: SwiftValue) -> StdResult {
    let (_, start, _) = sub_fields(&recv)?;
    Ok(make_index(start))
}

/// `Substring.endIndex` — the grapheme-cluster offset one past the last element,
/// in the base string's coordinate space.
fn sub_end_index(recv: SwiftValue) -> StdResult {
    let (_, _, end) = sub_fields(&recv)?;
    Ok(make_index(end))
}

/// `Substring.indices` — valid subscript positions of the slice, expressed in
/// the base string's coordinate space (`startIndex..<endIndex`), materialised
/// as an array of `String.Index` values.
fn sub_indices(recv: SwiftValue) -> StdResult {
    let (_, start, end) = sub_fields(&recv)?;
    let items: Vec<SwiftValue> = (start..end).map(make_index).collect();
    Ok(SwiftValue::Array(Rc::new(items)))
}

/// `Substring.base` — the full original `String` the Substring was sliced from.
///
/// Only registered under `BuiltinReceiver::Substring`, so `"abc".base` errors.
fn base(recv: SwiftValue) -> StdResult {
    match recv {
        SwiftValue::Substring { base, .. } => Ok(SwiftValue::Str((*base).clone())),
        _ => Err(type_err(format!(
            "base: expected Substring, got {}",
            recv.type_name()
        ))),
    }
}

/// `Substring.isContiguousUTF8` — always `true` (runtime stores contiguous UTF-8).
fn is_contiguous_utf8(recv: SwiftValue) -> StdResult {
    sub_fields(&recv)?;
    Ok(SwiftValue::Bool(true))
}

/// `Substring.characters` — the character view; returns `self` (deprecated in
/// Swift 4+ but still compiles; a Substring IS its character collection).
fn characters(recv: SwiftValue) -> StdResult {
    sub_fields(&recv)?;
    Ok(recv)
}

// ---- `prefix` / `suffix` returning Substring views --------------------------

/// `Substring.prefix(_:)` — returns a Substring view of the first `n` clusters.
fn sub_prefix(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (_, start, end) = sub_fields(&recv)?;
    let n = args
        .iter()
        .find_map(|a| match a {
            SwiftValue::Int(i) if i.raw >= 0 => Some(i.raw as usize),
            _ => None,
        })
        .unwrap_or(0);
    let new_end = (start + n).min(end);
    let base_rc = match &recv {
        SwiftValue::Substring { base, .. } => Rc::clone(base),
        _ => unreachable!(),
    };
    val(
        SwiftValue::Substring {
            base: base_rc,
            start,
            end: new_end,
        },
        recv,
    )
}

/// `Substring.suffix(_:)` — returns a Substring view of the last `n` clusters.
fn sub_suffix(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (_, start, end) = sub_fields(&recv)?;
    let n = args
        .iter()
        .find_map(|a| match a {
            SwiftValue::Int(i) if i.raw >= 0 => Some(i.raw as usize),
            _ => None,
        })
        .unwrap_or(0);
    let new_start = if n >= end - start { start } else { end - n };
    let base_rc = match &recv {
        SwiftValue::Substring { base, .. } => Rc::clone(base),
        _ => unreachable!(),
    };
    val(
        SwiftValue::Substring {
            base: base_rc,
            start: new_start,
            end,
        },
        recv,
    )
}

// ---- `index` (labeled, base-relative) ----------------------------------------

/// `Substring.index` — label-aware dispatch.  All offsets are in the base
/// string's coordinate space; valid range is `[sub.start, sub.end]`.
fn sub_form_index_labeled(
    c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    sub_index_labeled(c, recv, args)
}

fn sub_index_labeled(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let (base_str, sub_start, sub_end) = sub_fields(&recv)?;
    let count = graphemes(base_str).len();
    let trap = |what: &str, offset: usize| -> StdError {
        trap_err(format!(
            "Substring.{what}: index {offset} out of slice [{sub_start},{sub_end}]"
        ))
    };

    // index(after: i)
    if let Some(after_val) = labeled_index(&args, "after") {
        let offset = index_offset(after_val)
            .ok_or_else(|| type_err("index(after:) expects a String.Index".into()))?;
        if offset < sub_start || offset >= sub_end {
            return Err(trap("index(after:)", offset));
        }
        return Ok(Some(Outcome {
            result: make_index(offset + 1),
            receiver: recv,
        }));
    }

    // index(before: i)
    if let Some(before_val) = labeled_index(&args, "before") {
        let offset = index_offset(before_val)
            .ok_or_else(|| type_err("index(before:) expects a String.Index".into()))?;
        if offset <= sub_start || offset > sub_end {
            return Err(trap("index(before:)", offset));
        }
        return Ok(Some(Outcome {
            result: make_index(offset - 1),
            receiver: recv,
        }));
    }

    // index(_:offsetBy:) and index(_:offsetBy:limitedBy:)
    if let Some(off_arg) = args.iter().find(|a| a.label.as_deref() == Some("offsetBy")) {
        let base_idx = args
            .iter()
            .find(|a| a.label.is_none())
            .ok_or_else(|| type_err("index(_:offsetBy:) expects a base String.Index".into()))?;
        let base_off = index_offset(&base_idx.value)
            .ok_or_else(|| type_err("index(_:offsetBy:) base must be a String.Index".into()))?;
        if base_off < sub_start || base_off > sub_end {
            return Err(trap_err(format!(
                "Substring.index(_:offsetBy:): base index {base_off} out of slice [{sub_start},{sub_end}]"
            )));
        }
        let n = match &off_arg.value {
            SwiftValue::Int(i) => i.raw,
            _ => return Err(type_err("index(_:offsetBy:) offset must be Int".into())),
        };
        let new_off = base_off as i128 + n;

        // index(_:offsetBy:limitedBy:)
        if let Some(limit_arg) = args
            .iter()
            .find(|a| a.label.as_deref() == Some("limitedBy"))
        {
            let limit = index_offset(&limit_arg.value).ok_or_else(|| {
                type_err("index(_:offsetBy:limitedBy:) limit must be String.Index".into())
            })? as i128;
            // n == 0 means "don't move" so the limit never applies.
            let passed = (n > 0 && new_off > limit) || (n < 0 && new_off < limit);
            if passed {
                return Ok(Some(Outcome {
                    result: SwiftValue::Nil,
                    receiver: recv,
                }));
            }
            if new_off < sub_start as i128 || new_off > sub_end as i128 {
                return Err(trap_err(format!(
                    "Substring.index(_:offsetBy:limitedBy:): result {new_off} out of slice [{sub_start},{sub_end}]"
                )));
            }
            return Ok(Some(Outcome {
                result: make_index(new_off as usize),
                receiver: recv,
            }));
        }

        // Plain index(_:offsetBy:)
        let _ = count; // suppress unused warning
        if new_off < sub_start as i128 || new_off > sub_end as i128 {
            return Err(trap_err(format!(
                "Substring.index(_:offsetBy:): result {new_off} out of slice [{sub_start},{sub_end}]"
            )));
        }
        return Ok(Some(Outcome {
            result: make_index(new_off as usize),
            receiver: recv,
        }));
    }

    Ok(None)
}

// ---- `distance` (labeled, base-relative) -------------------------------------

fn sub_distance_labeled(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let (_, sub_start, sub_end) = sub_fields(&recv)?;
    let from_val = match labeled_index(&args, "from") {
        Some(v) => v,
        None => return Ok(None),
    };
    let to_val = match labeled_index(&args, "to") {
        Some(v) => v,
        None => return Ok(None),
    };
    let from = index_offset(from_val)
        .ok_or_else(|| type_err("distance(from:to:) expects String.Index".into()))?;
    let to = index_offset(to_val)
        .ok_or_else(|| type_err("distance(from:to:) expects String.Index".into()))?;
    if from < sub_start || from > sub_end {
        return Err(trap_err(format!(
            "Substring.distance(from:to:): 'from' index {from} out of slice [{sub_start},{sub_end}]"
        )));
    }
    if to < sub_start || to > sub_end {
        return Err(trap_err(format!(
            "Substring.distance(from:to:): 'to' index {to} out of slice [{sub_start},{sub_end}]"
        )));
    }
    Ok(Some(Outcome {
        result: SwiftValue::int(to as i128 - from as i128),
        receiver: recv,
    }))
}

// ---- `replaceSubrange` (labeled, mutating) -----------------------------------

/// `Substring.replaceSubrange(_:with:)` — replace a range with new text.
///
/// The range bounds are **base-relative** (same coordinate space as
/// `startIndex`/`endIndex`).  They are translated to slice-local offsets before
/// splicing.  After mutation the substring **detaches** from its original base:
/// the receiver becomes a fresh `Substring { base: new_text, start: 0,
/// end: new_count }`, matching Swift's copy-on-write behaviour.
fn sub_replace_subrange(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let (_, sub_start, sub_end) = sub_fields(&recv)?;
    let text = str_of(&recv)?; // materialises graphemes [sub_start..sub_end]

    let range_arg = args
        .iter()
        .find(|a| matches!(&a.value, SwiftValue::Range { .. }));
    let range = match range_arg {
        Some(a) => &a.value,
        None => return Ok(None),
    };
    let replacement = args
        .iter()
        .find_map(|a| match &a.value {
            SwiftValue::Str(s) => Some(s.clone()),
            SwiftValue::Substring { .. } => str_of(&a.value).ok(),
            _ => None,
        })
        .ok_or_else(|| type_err("replaceSubrange(_:with:) expects a String replacement".into()))?;

    // Translate base-relative range bounds to slice-local offsets.
    // e.g. for a Substring with start=2, a range [3, 4) becomes [1, 2).
    let local_range = match range {
        SwiftValue::Range { lo, hi, inclusive } => SwiftValue::Range {
            lo: lo - sub_start as i128,
            hi: hi - sub_start as i128,
            inclusive: *inclusive,
        },
        other => other.clone(),
    };
    let slice_count = sub_end - sub_start;
    let (start, end) =
        tswift_core::collection_range_bounds(&local_range, slice_count, "replaceSubrange")
            .map_err(StdError::Error)?;

    let mut grapheme_vec = graphemes(&text);
    grapheme_vec.splice(start..end, graphemes(&replacement));
    let new_text = grapheme_vec.concat();
    let new_count = graphemes(&new_text).len();

    // Detach: fresh backing string, indices restart at 0.
    Ok(Some(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Substring {
            base: Rc::new(new_text),
            start: 0,
            end: new_count,
        },
    }))
}

// ---- `makeContiguousUTF8` (mutating, no-op) ----------------------------------

fn make_contiguous_utf8(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    sub_fields(&recv)?; // validate type
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(base: &str, start: usize, end: usize) -> SwiftValue {
        SwiftValue::Substring {
            base: Rc::new(base.into()),
            start,
            end,
        }
    }

    fn s(t: &str) -> SwiftValue {
        SwiftValue::Str(t.into())
    }

    #[test]
    fn start_end_index_are_base_relative() {
        // "Hello, World!" sliced to "World" (offsets 7..12)
        let sv = sub("Hello, World!", 7, 12);
        assert_eq!(sub_start_index(sv.clone()).unwrap(), make_index(7));
        assert_eq!(sub_end_index(sv).unwrap(), make_index(12));
    }

    #[test]
    fn base_returns_full_original_string() {
        let sv = sub("Hello, World!", 7, 12);
        assert_eq!(base(sv).unwrap(), s("Hello, World!"));
    }

    #[test]
    fn is_contiguous_utf8_always_true() {
        assert_eq!(
            is_contiguous_utf8(sub("abc", 0, 3)).unwrap(),
            SwiftValue::Bool(true)
        );
    }

    #[test]
    fn characters_returns_self() {
        let sv = sub("abc", 0, 2);
        assert_eq!(characters(sv.clone()).unwrap(), sv);
    }

    struct M;
    impl StdContext for M {
        fn call_closure(&mut self, _i: usize, _a: Vec<SwiftValue>) -> StdResult {
            Ok(SwiftValue::Nil)
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            unreachable!()
        }
    }

    #[test]
    fn sub_prefix_returns_view() {
        let mut m = M;
        // "Hello" is s[0..5]; prefix(3) → s[0..3] = "Hel"
        let sv = sub("Hello", 0, 5);
        let out = sub_prefix(&mut m, sv, vec![SwiftValue::int(3)])
            .unwrap()
            .result;
        assert_eq!(format!("{out}"), "Hel");
        // Check it's a Substring view with correct offsets.
        assert!(matches!(
            out,
            SwiftValue::Substring {
                start: 0,
                end: 3,
                ..
            }
        ));
    }

    #[test]
    fn sub_suffix_returns_view() {
        let mut m = M;
        // "Hello" is s[0..5]; suffix(2) → s[3..5] = "lo"
        let sv = sub("Hello", 0, 5);
        let out = sub_suffix(&mut m, sv, vec![SwiftValue::int(2)])
            .unwrap()
            .result;
        assert_eq!(format!("{out}"), "lo");
        assert!(matches!(
            out,
            SwiftValue::Substring {
                start: 3,
                end: 5,
                ..
            }
        ));
    }

    #[test]
    fn sub_prefix_on_offset_slice() {
        let mut m = M;
        // Base "Hello, World!", slice is "World!" s[7..13], prefix(5) → s[7..12]
        let sv = sub("Hello, World!", 7, 13);
        let out = sub_prefix(&mut m, sv, vec![SwiftValue::int(5)])
            .unwrap()
            .result;
        assert_eq!(format!("{out}"), "World");
        assert!(matches!(
            out,
            SwiftValue::Substring {
                start: 7,
                end: 12,
                ..
            }
        ));
    }

    #[test]
    fn sub_distance_base_relative() {
        let mut m = M;
        let sv = sub("Hello, World!", 7, 13);
        // distance from offset 7 to offset 13 = 6
        let from_arg = Arg {
            label: Some("from".into()),
            value: make_index(7),

            static_ty: None,
        };
        let to_arg = Arg {
            label: Some("to".into()),
            value: make_index(13),

            static_ty: None,
        };
        let out = sub_distance_labeled(&mut m, sv, vec![from_arg, to_arg])
            .unwrap()
            .unwrap()
            .result;
        assert_eq!(out, SwiftValue::int(6));
    }

    #[test]
    fn sub_distance_out_of_slice_traps() {
        let mut m = M;
        let sv = sub("Hello, World!", 7, 13);
        // 'from' = 0 is outside [7, 13]
        let from_arg = Arg {
            label: Some("from".into()),
            value: make_index(0),

            static_ty: None,
        };
        let to_arg = Arg {
            label: Some("to".into()),
            value: make_index(13),

            static_ty: None,
        };
        let err = sub_distance_labeled(&mut m, sv, vec![from_arg, to_arg]).unwrap_err();
        assert!(
            matches!(err, StdError::Error(EvalError::Trap(_))),
            "expected trap, got {err:?}"
        );
    }

    #[test]
    fn make_contiguous_utf8_is_noop() {
        let mut m = M;
        let sv = sub("test", 0, 4);
        let out = make_contiguous_utf8(&mut m, sv.clone(), vec![]).unwrap();
        assert_eq!(out.result, SwiftValue::Void);
        assert_eq!(out.receiver, sv);
    }

    #[test]
    fn base_not_on_string() {
        // Calling `base` on a plain String must produce a Type error.
        let err = base(SwiftValue::Str("hello".into())).unwrap_err();
        assert!(
            matches!(err, StdError::Error(EvalError::Type(_))),
            "expected type error, got {err:?}"
        );
    }
}
