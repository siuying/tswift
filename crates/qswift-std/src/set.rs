//! `Set` method and property intrinsics with value semantics.
//!
//! Membership/`contains` is served by the shared algorithm layer (a `Set`
//! materializes to its elements); this module adds the set-specific surface:
//! insertion/removal, algebra, and the subset/superset predicates.

use std::rc::Rc;

use qswift_core::{
    BuiltinReceiver, EvalError, Interpreter, MethodEntry, Outcome, StdContext, StdError, StdResult,
    SwiftValue,
};

/// Register the `Set` intrinsics of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    let s = BuiltinReceiver::Set;
    interp.register_property(s, "count", count);
    interp.register_property(s, "isEmpty", is_empty);

    let mut mutating = |name: &str, f: qswift_core::IntrinsicFn| {
        interp.register_intrinsic(s, name, MethodEntry { mutating: true, func: f });
    };
    mutating("insert", insert);
    mutating("remove", remove);
    mutating("update", update);
    mutating("formUnion", form_union);
    mutating("formIntersection", form_intersection);
    mutating("subtract", subtract);
    mutating("formSymmetricDifference", form_symmetric_difference);

    let mut pure = |name: &str, f: qswift_core::IntrinsicFn| {
        interp.register_intrinsic(s, name, MethodEntry { mutating: false, func: f });
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

// ---- membership mutation ---------------------------------------------------

/// `insert(_:)` — returns `(inserted, memberAfterInsert)`.
fn insert(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut items = elements(&recv)?;
    let el = arg0(&args, "insert")?;
    let present = items.contains(&el);
    if !present {
        items.push(el.clone());
    }
    let result = SwiftValue::Tuple(vec![SwiftValue::Bool(!present), el]);
    Ok(Outcome { result, receiver: SwiftValue::Set(Rc::new(items)) })
}

/// `remove(_:)` — returns the removed element, or `nil` if absent.
fn remove(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut items = elements(&recv)?;
    let el = arg0(&args, "remove")?;
    let result = match items.iter().position(|x| *x == el) {
        Some(i) => items.remove(i),
        None => SwiftValue::Nil,
    };
    Ok(Outcome { result, receiver: SwiftValue::Set(Rc::new(items)) })
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
    Ok(Outcome { result, receiver: SwiftValue::Set(Rc::new(items)) })
}

// ---- algebra (non-mutating) ------------------------------------------------

fn union(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut all = elements(&recv)?;
    all.extend(other_elements(&args)?);
    Ok(value_outcome(set(all), recv))
}

fn intersection(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let other = other_elements(&args)?;
    let kept = elements(&recv)?.into_iter().filter(|x| other.contains(x)).collect();
    Ok(value_outcome(set(kept), recv))
}

fn subtracting(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let other = other_elements(&args)?;
    let kept = elements(&recv)?.into_iter().filter(|x| !other.contains(x)).collect();
    Ok(value_outcome(set(kept), recv))
}

fn symmetric_difference(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
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

fn form_symmetric_difference(c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
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

fn is_strict_superset(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
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
    Outcome { result: SwiftValue::Bool(b), receiver }
}

/// Turn a non-mutating result set into a mutating outcome: the computed set
/// becomes the new receiver and the call result is `Void`.
fn mutate(out: Outcome) -> Outcome {
    Outcome { receiver: out.result, result: SwiftValue::Void }
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
    fn insert_and_remove() {
        let mut m = M;
        let out = insert(&mut m, s(&[1, 2]), vec![SwiftValue::int(3)]).unwrap();
        assert_eq!(sorted_ints(out.receiver), vec![1, 2, 3]);
        // re-inserting reports inserted = false.
        let again = insert(&mut m, s(&[1, 2]), vec![SwiftValue::int(2)]).unwrap();
        match again.result {
            SwiftValue::Tuple(t) => assert_eq!(t[0], SwiftValue::Bool(false)),
            _ => panic!(),
        }
        let removed = remove(&mut m, s(&[1, 2]), vec![SwiftValue::int(2)]).unwrap();
        assert_eq!(removed.result, SwiftValue::int(2));
        assert_eq!(sorted_ints(removed.receiver), vec![1]);
    }

    #[test]
    fn algebra() {
        let mut m = M;
        assert_eq!(sorted_ints(union(&mut m, s(&[1, 2]), vec![s(&[2, 3])]).unwrap().result), vec![1, 2, 3]);
        assert_eq!(sorted_ints(intersection(&mut m, s(&[1, 2, 3]), vec![s(&[2, 3, 4])]).unwrap().result), vec![2, 3]);
        assert_eq!(sorted_ints(subtracting(&mut m, s(&[1, 2, 3]), vec![s(&[2])]).unwrap().result), vec![1, 3]);
        assert_eq!(sorted_ints(symmetric_difference(&mut m, s(&[1, 2]), vec![s(&[2, 3])]).unwrap().result), vec![1, 3]);
    }

    #[test]
    fn predicates_and_form_mutation() {
        let mut m = M;
        assert_eq!(is_subset(&mut m, s(&[1, 2]), vec![s(&[1, 2, 3])]).unwrap().result, SwiftValue::Bool(true));
        assert_eq!(is_superset(&mut m, s(&[1, 2, 3]), vec![s(&[1])]).unwrap().result, SwiftValue::Bool(true));
        assert_eq!(is_disjoint(&mut m, s(&[1, 2]), vec![s(&[3, 4])]).unwrap().result, SwiftValue::Bool(true));
        // formUnion replaces the receiver, returning Void.
        let fu = form_union(&mut m, s(&[1]), vec![s(&[2])]).unwrap();
        assert_eq!(fu.result, SwiftValue::Void);
        assert_eq!(sorted_ints(fu.receiver), vec![1, 2]);
    }
}
