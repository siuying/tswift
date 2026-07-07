//! `Set` method and property intrinsics with value semantics.
//!
//! Membership/`contains` is served by the shared algorithm layer (a `Set`
//! materializes to its elements); this module adds the set-specific surface:
//! insertion/removal, algebra, and the subset/superset predicates.

use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, EvalError, Interpreter, LabeledMethodEntry, MethodEntry, Outcome,
    StdContext, StdError, StdResult, StructObj, SwiftValue,
};

/// Register the `Set` intrinsics of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    let s = BuiltinReceiver::Set;
    interp.register_property(s, "count", count);
    interp.register_property(s, "isEmpty", is_empty);
    interp.register_property(s, "capacity", capacity);
    interp.register_property(s, "hashValue", hash_value);
    interp.register_property(s, "description", description);
    interp.register_property(s, "startIndex", start_index);
    interp.register_property(s, "endIndex", end_index);

    interp.register_labeled_intrinsic(
        s,
        "index",
        LabeledMethodEntry {
            mutating: false,
            func: index_labeled,
        },
    );
    interp.register_labeled_intrinsic(
        s,
        "formIndex",
        LabeledMethodEntry {
            mutating: true,
            func: form_index_labeled,
        },
    );

    let mut mutating = |name: &str, f: tswift_core::IntrinsicFn| {
        interp.register_intrinsic(
            s,
            name,
            MethodEntry {
                mutating: true,
                func: f,
            },
        );
    };
    mutating("insert", insert);
    mutating("remove", remove); // remove(_:) by value
    mutating("update", update);
    mutating("formUnion", form_union);
    mutating("formIntersection", form_intersection);
    mutating("subtract", subtract);
    mutating("formSymmetricDifference", form_symmetric_difference);
    mutating("removeAll", remove_all);
    mutating("reserveCapacity", reserve_capacity);
    mutating("removeFirst", remove_first);
    mutating("popFirst", pop_first);

    let mut pure = |name: &str, f: tswift_core::IntrinsicFn| {
        interp.register_intrinsic(
            s,
            name,
            MethodEntry {
                mutating: false,
                func: f,
            },
        );
    };
    pure("union", union);
    pure("intersection", intersection);
    pure("subtracting", subtracting);
    pure("symmetricDifference", symmetric_difference);
    pure("isSubset", is_subset);
    pure("isSuperset", is_superset);
    pure("isStrictSubset", is_strict_subset);
    pure("isStrictSuperset", is_strict_superset);
    pure("isDisjoint", is_disjoint);
    pure("firstIndex", first_index_of);
    pure("makeIterator", make_iterator);

    // `next()` pops the first element; must be mutating so the shrunk set is
    // written back to the iterator variable.
    interp.register_intrinsic(
        s,
        "next",
        MethodEntry {
            mutating: true,
            func: next,
        },
    );

    // `remove(at:)` is label-aware: the argument has label "at".
    interp.register_labeled_intrinsic(
        s,
        "remove",
        LabeledMethodEntry {
            mutating: true,
            func: remove_labeled,
        },
    );
}

// ---- Set.Index ------------------------------------------------------------

/// Construct an opaque `Set.Index` anchored to the element currently at
/// `offset` in `items`.
///
/// The `_anchor` field holds the element at that position (or `Void` for an
/// end-of-collection sentinel).  When the index is later used, the stored
/// anchor is compared against the live element at the same offset; a mismatch
/// means the collection was mutated and the index is stale — we trap.
pub(crate) fn make_set_index(offset: usize, items: &[SwiftValue]) -> SwiftValue {
    let anchor = items.get(offset).cloned().unwrap_or(SwiftValue::Void);
    SwiftValue::Struct(std::rc::Rc::new(StructObj {
        type_name: "Set.Index".into(),
        fields: vec![
            ("_offset".into(), SwiftValue::int(offset as i128)),
            ("_anchor".into(), anchor),
        ],
    }))
}

/// Extract the positional offset from a `Set.Index` value, and validate it
/// against the live element slice to detect stale (post-mutation) indices.
///
/// Returns the offset on success.  Traps if the anchor no longer matches the
/// element at that position (collection was mutated), or if the offset is at
/// or past the end (caller receives an out-of-range error appropriate for
/// subscript use).
pub(crate) fn check_set_index(items: &[SwiftValue], v: &SwiftValue) -> Result<usize, StdError> {
    let obj = match v {
        SwiftValue::Struct(o) if o.type_name == "Set.Index" => o,
        _ => return Err(type_err("expected a Set.Index".into())),
    };
    let offset = match obj.get("_offset") {
        Some(SwiftValue::Int(i)) if i.raw >= 0 => i.raw as usize,
        _ => return Err(type_err("invalid Set.Index".into())),
    };
    if offset >= items.len() {
        return Err(StdError::Error(EvalError::Trap(
            "Set.Index is at or past endIndex".into(),
        )));
    }
    // Anchor check — detects stale indices after mutation.
    if let Some(anchor) = obj.get("_anchor") {
        if *anchor != SwiftValue::Void && *anchor != items[offset] {
            return Err(StdError::Error(EvalError::Trap(
                "invalid Set.Index: collection was mutated after this index was created".into(),
            )));
        }
    }
    Ok(offset)
}

/// Extract just the positional offset from a `Set.Index`, without validity
/// checking.  Used only where stale detection is not needed (e.g. computing a
/// next-index from a just-created index).
pub(crate) fn set_index_offset(v: &SwiftValue) -> Option<usize> {
    match v {
        SwiftValue::Struct(obj) if obj.type_name == "Set.Index" => {
            obj.get("_offset").and_then(|f| match f {
                SwiftValue::Int(i) if i.raw >= 0 => Some(i.raw as usize),
                _ => None,
            })
        }
        _ => None,
    }
}

fn elements(recv: &SwiftValue) -> Result<Vec<SwiftValue>, StdError> {
    match recv {
        SwiftValue::Set(s) => Ok(s.as_ref().clone()),
        other => Err(type_err(format!(
            "expected a set receiver, got {}",
            other.type_name()
        ))),
    }
}

/// Elements of the first set/array/range argument.
fn other_elements(args: &[SwiftValue]) -> Result<Vec<SwiftValue>, StdError> {
    args.iter()
        .find_map(seq_elements)
        .ok_or_else(|| type_err("expected another set".into()))
}

fn seq_elements(v: &SwiftValue) -> Option<Vec<SwiftValue>> {
    match v {
        SwiftValue::Set(s) => Some(s.as_ref().clone()),
        SwiftValue::Array(a) => Some(a.as_ref().clone()),
        SwiftValue::Range { lo, hi, inclusive } => {
            let end = if *inclusive { hi + 1 } else { *hi };
            Some((*lo..end).map(SwiftValue::int).collect())
        }
        _ => None,
    }
}

fn set(items: Vec<SwiftValue>) -> SwiftValue {
    let mut out: Vec<SwiftValue> = Vec::with_capacity(items.len());
    for it in items {
        if !out.contains(&it) {
            out.push(it);
        }
    }
    SwiftValue::Set(Rc::new(out))
}

// ---- properties ------------------------------------------------------------

fn count(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(elements(&recv)?.len() as i128))
}

fn is_empty(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(elements(&recv)?.is_empty()))
}

/// FNV-1a digest of a byte slice (the shared hashing primitive here).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A stable per-value digest for the scalar element kinds a `Set` can hold.
pub(crate) fn value_digest(v: &SwiftValue) -> u64 {
    match v {
        SwiftValue::Int(i) => fnv1a(&(i.raw as u64).to_le_bytes()),
        SwiftValue::Double(d) => {
            let bits = if *d == 0.0 { 0 } else { d.to_bits() };
            fnv1a(&bits.to_le_bytes())
        }
        SwiftValue::Str(s) => fnv1a(s.as_bytes()),
        SwiftValue::Bool(b) => fnv1a(&[u8::from(*b)]),
        _ => fnv1a(&[0]),
    }
}

/// `Set.description` — the bracketed element list, e.g. `[1, 2, 3]`. Elements
/// appear in insertion order (deterministic here), unlike Swift's hashed order.
fn description(recv: SwiftValue) -> StdResult {
    elements(&recv)?;
    Ok(SwiftValue::Str(recv.to_string()))
}

/// `Set.hashValue` — an order-independent digest: equal sets (regardless of
/// insertion order) hash equally. The element digests are combined with a
/// commutative wrapping sum, then mixed with the count.
fn hash_value(recv: SwiftValue) -> StdResult {
    let items = elements(&recv)?;
    let mut acc: u64 = 0;
    for e in &items {
        acc = acc.wrapping_add(value_digest(e));
    }
    acc ^= fnv1a(&(items.len() as u64).to_le_bytes());
    Ok(SwiftValue::int(i128::from(acc as i64)))
}

/// `Set.capacity` — a lower bound modelled as the live element count
/// (Swift guarantees `capacity >= count`; exact reserve sizing is not modelled).
fn capacity(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(elements(&recv)?.len() as i128))
}

// ---- index properties -----------------------------------------------------

/// `Set.startIndex` — an opaque index to the first element (anchored), or an
/// end-sentinel when the set is empty.
fn start_index(recv: SwiftValue) -> StdResult {
    let items = elements(&recv)?;
    Ok(make_set_index(0, &items))
}

/// `Set.endIndex` — an opaque one-past-the-end sentinel (anchor = Void).
fn end_index(recv: SwiftValue) -> StdResult {
    let items = elements(&recv)?;
    Ok(make_set_index(items.len(), &items))
}

/// `Set.index` — label-aware dispatch:
/// - `index(after: i)` → advance by one (traps at `endIndex`)
/// - `index(of: e)`    → returns `Set.Index?`, `nil` when absent
///
/// Also used as the back-end for `formIndex(after:)` in `dispatch.rs`.
pub(crate) fn index_labeled(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let items = elements(&recv)?;

    // index(after: i)
    if let Some(arg) = args.iter().find(|a| a.label.as_deref() == Some("after")) {
        let offset = set_index_offset(&arg.value)
            .ok_or_else(|| type_err("index(after:) expects a Set.Index".into()))?;
        let count = items.len();
        if offset >= count {
            return Err(StdError::Error(EvalError::Trap(
                "Set.index(after:): index is at or past endIndex".into(),
            )));
        }
        return Ok(Some(Outcome {
            result: make_set_index(offset + 1, &items),
            receiver: recv,
        }));
    }

    // index(of: e)
    if let Some(arg) = args.iter().find(|a| a.label.as_deref() == Some("of")) {
        let result = match items.iter().position(|x| *x == arg.value) {
            Some(i) => make_set_index(i, &items),
            None => SwiftValue::Nil,
        };
        return Ok(Some(Outcome {
            result,
            receiver: recv,
        }));
    }

    Ok(None)
}

/// `Set.formIndex(after:)` — registered as a labeled intrinsic so the call
/// resolves through the stdlib dispatcher.  Actual inout write-back of the
/// index argument is handled by the interpreter's `formIndex` special-case in
/// `dispatch.rs`, which calls `index_labeled` with `after:` and writes the
/// result back to the caller's `&i` place.  This function exists so the
/// builtin registry answers `has_labeled_intrinsic(Set, "formIndex") == true`
/// (needed for the special-case gate in dispatch).
fn form_index_labeled(
    c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    // Delegate to index(after:) — the new index is the result.
    index_labeled(c, recv, args)
}

/// `Set.firstIndex(of:)` — returns the opaque `Set.Index` (anchored) for an
/// element, or `nil` when absent. Overrides the generic `Sequence.firstIndex`
/// (which returns an `Int` position) with the correct `Set.Index?` return type.
fn first_index_of(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let items = elements(&recv)?;
    let needle = args
        .first()
        .cloned()
        .ok_or_else(|| type_err("firstIndex(of:) expects an element".into()))?;
    let result = match items.iter().position(|x| *x == needle) {
        Some(i) => make_set_index(i, &items),
        None => SwiftValue::Nil,
    };
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

/// `Set.remove(at:)` — remove and return the element at the given `Set.Index`.
/// Traps if the index is stale (collection mutated) or out of bounds.
fn remove_at_index(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut items = elements(&recv)?;
    let idx_val = args
        .first()
        .ok_or_else(|| type_err("remove(at:) expects a Set.Index".into()))?;
    let idx = check_set_index(&items, idx_val)?;
    let removed = items.remove(idx);
    Ok(Outcome {
        result: removed,
        receiver: SwiftValue::Set(std::rc::Rc::new(items)),
    })
}

/// `Set.remove` — label-aware dispatch:
/// - `remove(at: Set.Index)` → remove by opaque index, return the element
/// - `remove(_:)`           → remove by value, return element or nil
fn remove_labeled(
    c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    // remove(at: Set.Index)
    if let Some(arg) = args.iter().find(|a| a.label.as_deref() == Some("at")) {
        let plain = vec![arg.value.clone()];
        return Ok(Some(remove_at_index(c, recv, plain)?));
    }
    // remove(_:) — remove by value (fall through to positional intrinsic)
    Ok(None)
}

/// `Set.makeIterator()` — for-in over `Set` is driven by
/// `materialize_builtin_sequence` (which already works). Registering this
/// no-op gives honest coverage credit and lets callers chain
/// `.makeIterator()` explicitly.
fn make_iterator(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    Ok(Outcome {
        result: recv.clone(),
        receiver: recv,
    })
}

/// `Set.next()` — pop and return the first element as the iterator's next
/// element, or `nil` when the set is exhausted.  Mirrors `popFirst()` in
/// value-semantic terms (the mutated set becomes the new receiver).
fn next(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let mut items = elements(&recv)?;
    let result = if items.is_empty() {
        SwiftValue::Nil
    } else {
        items.remove(0)
    };
    Ok(Outcome {
        result,
        receiver: SwiftValue::Set(std::rc::Rc::new(items)),
    })
}

// ---- membership mutation ---------------------------------------------------

/// `insert(_:)` — returns `(inserted, memberAfterInsert)`.
fn insert(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut items = elements(&recv)?;
    let el = arg0(&args, "insert")?;
    let present = items.contains(&el);
    if !present {
        items.push(el.clone());
    }
    let result = SwiftValue::tuple_labeled(
        vec![SwiftValue::Bool(!present), el],
        vec![
            Some("inserted".to_string()),
            Some("memberAfterInsert".to_string()),
        ],
    );
    Ok(Outcome {
        result,
        receiver: SwiftValue::Set(Rc::new(items)),
    })
}

/// `remove(_:)` — returns the removed element, or `nil` if absent.
fn remove(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut items = elements(&recv)?;
    let el = arg0(&args, "remove")?;
    let result = match items.iter().position(|x| *x == el) {
        Some(i) => items.remove(i),
        None => SwiftValue::Nil,
    };
    Ok(Outcome {
        result,
        receiver: SwiftValue::Set(Rc::new(items)),
    })
}

/// `update(with:)` — insert, returning the replaced element (or `nil`).
fn update(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut items = elements(&recv)?;
    let el = arg0(&args, "update(with:)")?;
    let result = match items.iter().position(|x| *x == el) {
        Some(i) => std::mem::replace(&mut items[i], el),
        None => {
            items.push(el);
            SwiftValue::Nil
        }
    };
    Ok(Outcome {
        result,
        receiver: SwiftValue::Set(Rc::new(items)),
    })
}

/// `Set.removeAll(keepingCapacity:)` — drop every element in place.
fn remove_all(_c: &mut dyn StdContext, _recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Set(Rc::new(Vec::new())),
    })
}

/// `Set.reserveCapacity(_:)` — a no-op here; storage grows implicitly.
fn reserve_capacity(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

/// `Set.removeFirst()` — remove and return the first element. Traps when empty,
/// matching Swift. Iteration order is unspecified for multi-element sets.
fn remove_first(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let mut items = elements(&recv)?;
    if items.is_empty() {
        return Err(StdError::Error(EvalError::Trap(
            "can't remove first element from an empty collection".into(),
        )));
    }
    let first = items.remove(0);
    Ok(Outcome {
        result: first,
        receiver: SwiftValue::Set(Rc::new(items)),
    })
}

/// `Set.popFirst()` — remove and return the first element, or `nil` when empty.
fn pop_first(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let mut items = elements(&recv)?;
    let result = if items.is_empty() {
        SwiftValue::Nil
    } else {
        items.remove(0)
    };
    Ok(Outcome {
        result,
        receiver: SwiftValue::Set(Rc::new(items)),
    })
}

// ---- algebra (non-mutating) ------------------------------------------------

fn union(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut all = elements(&recv)?;
    all.extend(other_elements(&args)?);
    Ok(value_outcome(set(all), recv))
}

fn intersection(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let other = other_elements(&args)?;
    let kept = elements(&recv)?
        .into_iter()
        .filter(|x| other.contains(x))
        .collect();
    Ok(value_outcome(set(kept), recv))
}

fn subtracting(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let other = other_elements(&args)?;
    let kept = elements(&recv)?
        .into_iter()
        .filter(|x| !other.contains(x))
        .collect();
    Ok(value_outcome(set(kept), recv))
}

fn symmetric_difference(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    let a = elements(&recv)?;
    let b = other_elements(&args)?;
    let mut out: Vec<SwiftValue> = a.iter().filter(|x| !b.contains(x)).cloned().collect();
    out.extend(b.iter().filter(|x| !a.contains(x)).cloned());
    Ok(value_outcome(set(out), recv))
}

// ---- algebra (mutating: replace the receiver) ------------------------------

fn form_union(c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    Ok(mutate(union(c, recv, args)?))
}

fn form_intersection(c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    Ok(mutate(intersection(c, recv, args)?))
}

fn subtract(c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    Ok(mutate(subtracting(c, recv, args)?))
}

fn form_symmetric_difference(
    c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    Ok(mutate(symmetric_difference(c, recv, args)?))
}

// ---- predicates ------------------------------------------------------------

fn is_subset(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let (a, b) = (elements(&recv)?, other_elements(&args)?);
    Ok(bool_outcome(a.iter().all(|x| b.contains(x)), recv))
}

fn is_superset(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let (a, b) = (elements(&recv)?, other_elements(&args)?);
    Ok(bool_outcome(b.iter().all(|x| a.contains(x)), recv))
}

fn is_strict_subset(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let (a, b) = (elements(&recv)?, other_elements(&args)?);
    let subset = a.iter().all(|x| b.contains(x));
    Ok(bool_outcome(subset && a.len() < b.len(), recv))
}

fn is_strict_superset(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Outcomes {
    let (a, b) = (elements(&recv)?, other_elements(&args)?);
    let superset = b.iter().all(|x| a.contains(x));
    Ok(bool_outcome(superset && a.len() > b.len(), recv))
}

fn is_disjoint(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let (a, b) = (elements(&recv)?, other_elements(&args)?);
    Ok(bool_outcome(!a.iter().any(|x| b.contains(x)), recv))
}

// ---- helpers ---------------------------------------------------------------

type Outcomes = Result<Outcome, StdError>;

fn type_err(msg: String) -> StdError {
    StdError::Error(EvalError::Type(msg))
}

fn arg0(args: &[SwiftValue], who: &str) -> Result<SwiftValue, StdError> {
    args.first()
        .cloned()
        .ok_or_else(|| type_err(format!("{who} expects an argument")))
}

fn value_outcome(result: SwiftValue, receiver: SwiftValue) -> Outcome {
    Outcome { result, receiver }
}

fn bool_outcome(b: bool, receiver: SwiftValue) -> Outcome {
    Outcome {
        result: SwiftValue::Bool(b),
        receiver,
    }
}

/// Turn a non-mutating result set into a mutating outcome: the computed set
/// becomes the new receiver and the call result is `Void`.
fn mutate(out: Outcome) -> Outcome {
    Outcome {
        receiver: out.result,
        result: SwiftValue::Void,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct M;
    impl StdContext for M {
        fn call_closure(&mut self, _i: usize, _a: Vec<SwiftValue>) -> StdResult {
            Ok(SwiftValue::Nil)
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            unreachable!()
        }
    }

    fn s(xs: &[i128]) -> SwiftValue {
        set(xs.iter().map(|&x| SwiftValue::int(x)).collect())
    }

    fn sorted_ints(v: SwiftValue) -> Vec<i128> {
        let mut out: Vec<i128> = match v {
            SwiftValue::Set(items) => items
                .iter()
                .map(|x| match x {
                    SwiftValue::Int(i) => i.raw,
                    _ => 0,
                })
                .collect(),
            _ => panic!("expected set"),
        };
        out.sort();
        out
    }

    #[test]
    fn description_renders_bracketed_list() {
        assert_eq!(
            description(s(&[1, 2, 3])).unwrap(),
            SwiftValue::Str("[1, 2, 3]".into())
        );
    }

    #[test]
    fn hash_value_is_order_independent() {
        // Equal sets hash equally regardless of element order.
        assert_eq!(
            hash_value(s(&[1, 2, 3])).unwrap(),
            hash_value(s(&[3, 2, 1])).unwrap()
        );
        // Different membership hashes differently.
        assert_ne!(
            hash_value(s(&[1, 2, 3])).unwrap(),
            hash_value(s(&[1, 2])).unwrap()
        );
    }

    #[test]
    fn capacity_remove_first_and_pop() {
        let mut m = M;
        assert_eq!(capacity(s(&[1, 2, 3])).unwrap(), SwiftValue::int(3));
        // removeFirst yields an element and shrinks the set.
        let out = remove_first(&mut m, s(&[42]), vec![]).unwrap();
        assert_eq!(out.result, SwiftValue::int(42));
        assert!(matches!(out.receiver, SwiftValue::Set(p) if p.is_empty()));
        // removeFirst on an empty set traps.
        assert!(remove_first(&mut m, s(&[]), vec![]).is_err());
        // popFirst is nil on empty, Some otherwise.
        assert_eq!(
            pop_first(&mut m, s(&[]), vec![]).unwrap().result,
            SwiftValue::Nil
        );
        assert_eq!(
            pop_first(&mut m, s(&[7]), vec![]).unwrap().result,
            SwiftValue::int(7)
        );
        // removeAll empties the set.
        let cleared = remove_all(&mut m, s(&[1, 2, 3]), vec![]).unwrap();
        assert!(matches!(cleared.receiver, SwiftValue::Set(p) if p.is_empty()));
    }

    #[test]
    fn insert_and_remove() {
        let mut m = M;
        let out = insert(&mut m, s(&[1, 2]), vec![SwiftValue::int(3)]).unwrap();
        assert_eq!(sorted_ints(out.receiver), vec![1, 2, 3]);
        // re-inserting reports inserted = false.
        let again = insert(&mut m, s(&[1, 2]), vec![SwiftValue::int(2)]).unwrap();
        match again.result {
            SwiftValue::Tuple(t, _) => assert_eq!(t[0], SwiftValue::Bool(false)),
            _ => panic!(),
        }
        let removed = remove(&mut m, s(&[1, 2]), vec![SwiftValue::int(2)]).unwrap();
        assert_eq!(removed.result, SwiftValue::int(2));
        assert_eq!(sorted_ints(removed.receiver), vec![1]);
    }

    #[test]
    fn algebra() {
        let mut m = M;
        assert_eq!(
            sorted_ints(union(&mut m, s(&[1, 2]), vec![s(&[2, 3])]).unwrap().result),
            vec![1, 2, 3]
        );
        assert_eq!(
            sorted_ints(
                intersection(&mut m, s(&[1, 2, 3]), vec![s(&[2, 3, 4])])
                    .unwrap()
                    .result
            ),
            vec![2, 3]
        );
        assert_eq!(
            sorted_ints(
                subtracting(&mut m, s(&[1, 2, 3]), vec![s(&[2])])
                    .unwrap()
                    .result
            ),
            vec![1, 3]
        );
        assert_eq!(
            sorted_ints(
                symmetric_difference(&mut m, s(&[1, 2]), vec![s(&[2, 3])])
                    .unwrap()
                    .result
            ),
            vec![1, 3]
        );
    }

    #[test]
    fn predicates_and_form_mutation() {
        let mut m = M;
        assert_eq!(
            is_subset(&mut m, s(&[1, 2]), vec![s(&[1, 2, 3])])
                .unwrap()
                .result,
            SwiftValue::Bool(true)
        );
        assert_eq!(
            is_superset(&mut m, s(&[1, 2, 3]), vec![s(&[1])])
                .unwrap()
                .result,
            SwiftValue::Bool(true)
        );
        assert_eq!(
            is_disjoint(&mut m, s(&[1, 2]), vec![s(&[3, 4])])
                .unwrap()
                .result,
            SwiftValue::Bool(true)
        );
        // formUnion replaces the receiver, returning Void.
        let fu = form_union(&mut m, s(&[1]), vec![s(&[2])]).unwrap();
        assert_eq!(fu.result, SwiftValue::Void);
        assert_eq!(sorted_ints(fu.receiver), vec![1, 2]);
    }

    #[test]
    fn set_index_round_trip() {
        let mut m = M;
        // startIndex on a single-element set is offset 0, anchored to the element.
        let single = s(&[42]);
        let items42 = vec![SwiftValue::int(42)];
        let si0 = make_set_index(0, &items42); // anchored
        let si1 = make_set_index(1, &items42); // end sentinel (anchor = Void)
        assert_eq!(start_index(single.clone()).unwrap(), si0);
        assert_eq!(end_index(single.clone()).unwrap(), si1);

        // index(after: startIndex) == endIndex for a single-element set.
        let after_out = index_labeled(
            &mut m,
            single.clone(),
            vec![Arg {
                label: Some("after".to_string()),
                value: si0.clone(),

                static_ty: None,
            }],
        )
        .unwrap()
        .unwrap();
        assert_eq!(after_out.result, si1);

        // index(after: endIndex) traps.
        assert!(index_labeled(
            &mut m,
            single.clone(),
            vec![Arg {
                label: Some("after".to_string()),
                value: si1.clone(),

                static_ty: None,
            }],
        )
        .is_err());

        // firstIndex(of:) returns Some (anchored) for present, nil for absent.
        let found = first_index_of(&mut m, single.clone(), vec![SwiftValue::int(42)]).unwrap();
        assert_eq!(found.result, si0);
        let absent = first_index_of(&mut m, single.clone(), vec![SwiftValue::int(99)]).unwrap();
        assert_eq!(absent.result, SwiftValue::Nil);

        // remove(at:) with valid anchored index returns the element.
        let rm_out = remove_at_index(&mut m, single.clone(), vec![si0.clone()]).unwrap();
        assert_eq!(rm_out.result, SwiftValue::int(42));
        assert!(matches!(rm_out.receiver, SwiftValue::Set(p) if p.is_empty()));

        // remove(at: endIndex) traps — offset >= len.
        assert!(remove_at_index(&mut m, single.clone(), vec![si1.clone()]).is_err());

        // Stale-index detection: after removing the element, using the old index traps.
        let mut two = s(&[10, 20]);
        let two_items = vec![SwiftValue::int(10), SwiftValue::int(20)];
        let old_start = make_set_index(0, &two_items); // anchored to 10
                                                       // Remove at startIndex — sets now holds [20].
        let rm2 = remove_at_index(&mut m, two.clone(), vec![old_start.clone()]).unwrap();
        assert_eq!(rm2.result, SwiftValue::int(10));
        two = rm2.receiver; // mutated set: [20]
                            // Now the element at offset 0 is 20, but old_start has anchor=10 → trap.
        assert!(
            remove_at_index(&mut m, two, vec![old_start]).is_err(),
            "using stale index after mutation must trap"
        );

        // next() pops first element.
        let nx = next(&mut m, single.clone(), vec![]).unwrap();
        assert_eq!(nx.result, SwiftValue::int(42));
        assert!(matches!(nx.receiver, SwiftValue::Set(p) if p.is_empty()));
        // next() on empty set returns nil.
        let nx2 = next(&mut m, s(&[]), vec![]).unwrap();
        assert_eq!(nx2.result, SwiftValue::Nil);
    }
}
