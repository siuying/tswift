//! The `Sequence`/`Collection` algorithm layer (layer 2 of the seam).
//!
//! Each algorithm is written once against the materialized elements of a builtin
//! sequence receiver and registered by method name, so it applies uniformly to
//! `Array`, `Range`, and `String` (as a sequence of single-character strings).
//! Closure-taking algorithms call back through [`StdContext`].

use std::cmp::Ordering;
use std::rc::Rc;

use tswift_core::{Arg, EvalError, Interpreter, StdContext, StdError, StdResult, SwiftValue};

/// Register every sequence algorithm of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    let mut a = |name: &str, f: tswift_core::AlgoFn| interp.register_algorithm(name, f);
    a("map", map);
    a("compactMap", compact_map);
    a("flatMap", flat_map);
    a("filter", filter);
    a("reduce", reduce);
    a("forEach", for_each);
    a("contains", contains);
    a("allSatisfy", all_satisfy);
    a("first", first_where);
    a("firstIndex", first_index);
    a("count", count_where);
    a("sorted", sorted);
    a("min", min_by);
    a("max", max_by);
    a("reversed", reversed);
    a("enumerated", enumerated);
    a("prefix", prefix);
    a("suffix", suffix);
    a("dropFirst", drop_first);
    a("dropLast", drop_last);
    a("drop", drop_while);
    a("split", split);
    a("joined", joined);
    a("elementsEqual", elements_equal);
    a("starts", starts_with);
    a("randomElement", random_element);
    a("shuffled", shuffled);

    // The algorithm table is the one implementation of the default protocol
    // operations. Register the protocol names separately so coverage reports
    // the shared capability rather than only concrete receiver call sites.
    interp.register_protocol_member("Sequence", "makeIterator");
    for name in [
        "count",
        "drop",
        "dropFirst",
        "dropLast",
        "first",
        "firstIndex",
        "flatMap",
        "indices",
        "isEmpty",
        "makeIterator",
        "map",
        "prefix",
        "split",
        "suffix",
    ] {
        interp.register_protocol_member("Collection", name);
    }
}

// ---- closure-taking transforms --------------------------------------------

fn map(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let id = closure(&args, "map")?;
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        out.push(ctx.call_closure(id, vec![it])?);
    }
    Ok(array(out))
}

fn compact_map(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let id = closure(&args, "compactMap")?;
    let mut out = Vec::new();
    for it in items {
        match ctx.call_closure(id, vec![it])? {
            SwiftValue::Nil => {}
            v => out.push(v),
        }
    }
    Ok(array(out))
}

fn flat_map(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let id = closure(&args, "flatMap")?;
    let mut out = Vec::new();
    for it in items {
        let transformed = ctx.call_closure(id, vec![it])?;
        if let Some(inner) = ctx.sequence_elements(&transformed) {
            out.extend(inner);
        } else {
            out.push(transformed);
        }
    }
    Ok(array(out))
}

fn filter(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let id = closure(&args, "filter")?;
    let mut out = Vec::new();
    for it in items {
        if truthy(ctx.call_closure(id, vec![it.clone()])?) {
            out.push(it);
        }
    }
    Ok(array(out))
}

fn reduce(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let id = closure(&args, "reduce")?;
    let mut acc = args
        .iter()
        .find(|a| a.label.is_none() || a.label.as_deref() == Some("into"))
        .map(|a| a.value.clone())
        .ok_or_else(|| type_err("reduce expects an initial value"))?;
    let into = args.iter().any(|a| a.label.as_deref() == Some("into"));
    for it in items {
        acc = if into {
            ctx.call_closure_inout(id, acc, it)?
        } else {
            ctx.call_closure(id, vec![acc, it])?
        };
    }
    Ok(acc)
}

fn for_each(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let id = closure(&args, "forEach")?;
    for it in items {
        ctx.call_closure(id, vec![it])?;
    }
    Ok(SwiftValue::Void)
}

// ---- predicates / search ---------------------------------------------------

fn contains(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    if let Ok(id) = closure(&args, "") {
        for it in items {
            if truthy(ctx.call_closure(id, vec![it])?) {
                return Ok(SwiftValue::Bool(true));
            }
        }
        return Ok(SwiftValue::Bool(false));
    }
    let needle = element_arg(&args).ok_or_else(|| type_err("contains expects an element"))?;
    Ok(SwiftValue::Bool(items.contains(&needle)))
}

fn all_satisfy(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let id = closure(&args, "allSatisfy")?;
    for it in items {
        if !truthy(ctx.call_closure(id, vec![it])?) {
            return Ok(SwiftValue::Bool(false));
        }
    }
    Ok(SwiftValue::Bool(true))
}

fn first_where(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let id = closure(&args, "first(where:)")?;
    for it in items {
        if truthy(ctx.call_closure(id, vec![it.clone()])?) {
            return Ok(it);
        }
    }
    Ok(SwiftValue::Nil)
}

fn first_index(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    if let Ok(id) = closure(&args, "") {
        for (i, it) in items.into_iter().enumerate() {
            if truthy(ctx.call_closure(id, vec![it])?) {
                return Ok(SwiftValue::int(i as i128));
            }
        }
        return Ok(SwiftValue::Nil);
    }
    let needle = element_arg(&args).ok_or_else(|| type_err("firstIndex expects an element"))?;
    Ok(items
        .iter()
        .position(|x| *x == needle)
        .map(|i| SwiftValue::int(i as i128))
        .unwrap_or(SwiftValue::Nil))
}

fn count_where(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let id = closure(&args, "count(where:)")?;
    let mut n = 0i128;
    for it in items {
        if truthy(ctx.call_closure(id, vec![it])?) {
            n += 1;
        }
    }
    Ok(SwiftValue::int(n))
}

// ---- ordering --------------------------------------------------------------

pub(crate) fn sorted(
    ctx: &mut dyn StdContext,
    items: Vec<SwiftValue>,
    args: Vec<Arg>,
) -> StdResult {
    let mut out = items;
    if let Ok(id) = closure(&args, "") {
        // sorted(by:) — closure is a strict-weak `<`-style comparator.
        let mut err = None;
        merge_sort_by(&mut out, &mut |a, b| {
            if err.is_some() {
                return Ordering::Equal;
            }
            match ctx.call_closure(id, vec![a.clone(), b.clone()]) {
                Ok(v) => {
                    if truthy(v) {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    }
                }
                Err(e) => {
                    err = Some(e);
                    Ordering::Equal
                }
            }
        });
        if let Some(e) = err {
            return Err(e);
        }
    } else {
        // Natural order via Comparable (scalars and types with a static `<`).
        merge_sort_by(&mut out, &mut |a, b| match ctx.value_less_than(a, b) {
            Some(true) => Ordering::Less,
            Some(false) => Ordering::Greater,
            None => Ordering::Equal,
        });
    }
    Ok(array(out))
}

fn min_by(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    extreme(ctx, items, args, Ordering::Less)
}

fn max_by(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    extreme(ctx, items, args, Ordering::Greater)
}

fn extreme(
    ctx: &mut dyn StdContext,
    items: Vec<SwiftValue>,
    args: Vec<Arg>,
    want: Ordering,
) -> StdResult {
    let by = closure(&args, "").ok();
    let mut iter = items.into_iter();
    let Some(mut best) = iter.next() else {
        return Ok(SwiftValue::Nil);
    };
    for it in iter {
        let less = match by {
            Some(id) => truthy(ctx.call_closure(id, vec![it.clone(), best.clone()])?),
            None => ctx.value_less_than(&it, &best).unwrap_or(false),
        };
        // `less` means it < best; pick it for min when less, for max when !less.
        let take = if want == Ordering::Less {
            less
        } else {
            !less && it != best
        };
        if take {
            best = it;
        }
    }
    Ok(best)
}

// ---- shape transforms ------------------------------------------------------

fn reversed(_c: &mut dyn StdContext, items: Vec<SwiftValue>, _a: Vec<Arg>) -> StdResult {
    Ok(crate::reversedcollection::make_reversed_collection(items))
}

fn enumerated(_c: &mut dyn StdContext, items: Vec<SwiftValue>, _a: Vec<Arg>) -> StdResult {
    let out = items
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            SwiftValue::tuple_labeled(
                vec![SwiftValue::int(i as i128), v],
                vec![Some("offset".to_string()), Some("element".to_string())],
            )
        })
        .collect();
    Ok(array(out))
}

fn prefix(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    if let Ok(id) = closure(&args, "") {
        let mut out = Vec::new();
        for it in items {
            if truthy(ctx.call_closure(id, vec![it.clone()])?) {
                out.push(it);
            } else {
                break;
            }
        }
        return Ok(array(out));
    }
    let n = count_arg(
        &args,
        "Can't take a prefix of negative length from a collection",
    )?
    .unwrap_or(0)
    .min(items.len());
    Ok(array(items[..n].to_vec()))
}

fn suffix(_c: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let n = count_arg(
        &args,
        "Can't take a suffix of negative length from a collection",
    )?
    .unwrap_or(0)
    .min(items.len());
    let start = items.len() - n;
    Ok(array(items[start..].to_vec()))
}

/// `drop(while:)` — the suffix beginning at the first element that fails the
/// predicate (the symmetric counterpart of `prefix(while:)`).
fn drop_while(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let id = closure(&args, "drop(while:)")?;
    let mut start = items.len();
    for (i, it) in items.iter().enumerate() {
        if !truthy(ctx.call_closure(id, vec![it.clone()])?) {
            start = i;
            break;
        }
    }
    Ok(array(items[start..].to_vec()))
}

fn drop_first(_c: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let n = count_arg(
        &args,
        "Can't drop a negative number of elements from a collection",
    )?
    .unwrap_or(1)
    .min(items.len());
    Ok(array(items[n..].to_vec()))
}

fn drop_last(_c: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let n = count_arg(
        &args,
        "Can't drop a negative number of elements from a collection",
    )?
    .unwrap_or(1)
    .min(items.len());
    Ok(array(items[..items.len() - n].to_vec()))
}

fn split(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let separator = labeled(&args, "separator").or_else(|| first_positional(&args));
    let predicate = closure(&args, "split").ok();
    if separator.is_none() && predicate.is_none() {
        return Err(type_err("split expects a separator or closure"));
    }
    let omit_empty = labeled(&args, "omittingEmptySubsequences")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let max_splits = labeled(&args, "maxSplits")
        .and_then(|v| match v {
            SwiftValue::Int(i) if i.raw >= 0 => Some(i.raw as usize),
            _ => None,
        })
        .unwrap_or(usize::MAX);
    let mut groups: Vec<Vec<SwiftValue>> = Vec::new();
    let mut cur: Vec<SwiftValue> = Vec::new();
    for it in items {
        let is_separator = match predicate {
            Some(id) => truthy(ctx.call_closure(id, vec![it.clone()])?),
            None => separator.as_ref().is_some_and(|sep| it == *sep),
        };
        if is_separator && groups.len() < max_splits {
            if !cur.is_empty() || !omit_empty {
                groups.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(it);
        }
    }
    if !cur.is_empty() || !omit_empty {
        groups.push(cur);
    }
    Ok(array(groups.into_iter().map(array).collect()))
}

fn joined(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let sep = labeled(&args, "separator");
    // String elements → a single joined string.
    if items.iter().all(|v| matches!(v, SwiftValue::Str(_))) {
        let sep = match sep {
            Some(SwiftValue::Str(s)) => s,
            _ => String::new(),
        };
        let parts: Vec<String> = items.iter().map(|v| v.to_string()).collect();
        return Ok(SwiftValue::Str(parts.join(&sep)));
    }
    // Array elements → a flattened array (separator inserted between groups).
    let sep_items = sep
        .as_ref()
        .and_then(|v| ctx.sequence_elements(v))
        .unwrap_or_default();
    let mut out = Vec::new();
    for (i, it) in items.into_iter().enumerate() {
        if i > 0 {
            out.extend(sep_items.clone());
        }
        if let Some(inner) = ctx.sequence_elements(&it) {
            out.extend(inner);
        } else {
            out.push(it);
        }
    }
    Ok(array(out))
}

// ---- comparison ------------------------------------------------------------

fn elements_equal(_c: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let other = first_positional(&args)
        .and_then(seq_of)
        .ok_or_else(|| type_err("elementsEqual expects a sequence"))?;
    Ok(SwiftValue::Bool(items == other))
}

fn starts_with(_c: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
    let prefix = labeled(&args, "with")
        .or_else(|| first_positional(&args))
        .and_then(seq_of)
        .ok_or_else(|| type_err("starts(with:) expects a sequence"))?;
    Ok(SwiftValue::Bool(
        items.len() >= prefix.len() && items[..prefix.len()] == prefix[..],
    ))
}

// ---- randomness ------------------------------------------------------------

fn random_element(_c: &mut dyn StdContext, items: Vec<SwiftValue>, _a: Vec<Arg>) -> StdResult {
    if items.is_empty() {
        return Ok(SwiftValue::Nil);
    }
    let i = pseudo_random(items.len() as u64) as usize % items.len();
    Ok(items[i].clone())
}

fn shuffled(_c: &mut dyn StdContext, mut items: Vec<SwiftValue>, _a: Vec<Arg>) -> StdResult {
    // Fisher–Yates with a tiny self-seeded PRNG (no external rand crate).
    let mut state = pseudo_random(items.len() as u64 + 0x9E37);
    for i in (1..items.len()).rev() {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let j = (state >> 33) as usize % (i + 1);
        items.swap(i, j);
    }
    Ok(array(items))
}

// ---- helpers ---------------------------------------------------------------

fn array(items: Vec<SwiftValue>) -> SwiftValue {
    SwiftValue::Array(Rc::new(items))
}

fn truthy(v: SwiftValue) -> bool {
    v.as_bool().unwrap_or(false)
}

fn closure(args: &[Arg], who: &str) -> Result<usize, StdError> {
    args.iter()
        .rev()
        .find_map(|a| match a.value {
            SwiftValue::Closure(id) => Some(id),
            _ => None,
        })
        .ok_or_else(|| type_err(&format!("{who} expects a closure")))
}

fn first_positional(args: &[Arg]) -> Option<SwiftValue> {
    args.iter()
        .find(|a| a.label.is_none())
        .map(|a| a.value.clone())
}

/// The element argument for search algorithms: the first non-closure value,
/// whatever its label (`contains(_:)`, `firstIndex(of:)`).
fn element_arg(args: &[Arg]) -> Option<SwiftValue> {
    args.iter()
        .find(|a| !matches!(a.value, SwiftValue::Closure(_)))
        .map(|a| a.value.clone())
}

fn labeled(args: &[Arg], label: &str) -> Option<SwiftValue> {
    args.iter()
        .find(|a| a.label.as_deref() == Some(label))
        .map(|a| a.value.clone())
}

/// The `Int` count argument for `prefix`/`suffix`/`dropFirst`/`dropLast`.
/// A missing argument is `Ok(None)` (the caller supplies its default); a
/// negative count *traps* with `negative_msg`, matching Swift's precondition
/// (Swift's `Array.dropFirst(-1)`/`prefix(-1)` do not clamp — they crash).
fn count_arg(args: &[Arg], negative_msg: &str) -> Result<Option<usize>, StdError> {
    match args.iter().find_map(|a| match &a.value {
        SwiftValue::Int(i) => Some(i.raw),
        _ => None,
    }) {
        Some(raw) if raw >= 0 => Ok(Some(raw as usize)),
        Some(_) => Err(StdError::Error(EvalError::Trap(negative_msg.to_string()))),
        None => Ok(None),
    }
}

fn type_err(msg: &str) -> StdError {
    StdError::Error(EvalError::Type(msg.to_string()))
}

/// Expand a value into a sequence of elements (array/range), else `None`.
fn seq_of(v: SwiftValue) -> Option<Vec<SwiftValue>> {
    match v {
        SwiftValue::Array(a) => Some(a.as_ref().clone()),
        SwiftValue::Range { lo, hi, inclusive } => {
            let end = if inclusive { hi + 1 } else { hi };
            Some((lo..end).map(SwiftValue::int).collect())
        }
        _ => None,
    }
}

/// Natural ordering over comparable scalar values (used in tests).
#[cfg(test)]
fn natural_cmp(a: &SwiftValue, b: &SwiftValue) -> Option<Ordering> {
    match (a, b) {
        (SwiftValue::Int(x), SwiftValue::Int(y)) => Some(x.raw.cmp(&y.raw)),
        (SwiftValue::Double(x), SwiftValue::Double(y)) => x.partial_cmp(y),
        (SwiftValue::Int(x), SwiftValue::Double(y)) => (x.raw as f64).partial_cmp(y),
        (SwiftValue::Double(x), SwiftValue::Int(y)) => x.partial_cmp(&(y.raw as f64)),
        (SwiftValue::Str(x), SwiftValue::Str(y)) => Some(x.cmp(y)),
        (SwiftValue::Bool(x), SwiftValue::Bool(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

/// A stable merge sort driven by a fallible comparator closure.
fn merge_sort_by(
    items: &mut [SwiftValue],
    cmp: &mut dyn FnMut(&SwiftValue, &SwiftValue) -> Ordering,
) {
    let n = items.len();
    if n <= 1 {
        return;
    }
    let mid = n / 2;
    let mut left = items[..mid].to_vec();
    let mut right = items[mid..].to_vec();
    merge_sort_by(&mut left, cmp);
    merge_sort_by(&mut right, cmp);
    let (mut i, mut j, mut k) = (0, 0, 0);
    while i < left.len() && j < right.len() {
        if cmp(&right[j], &left[i]) == Ordering::Less {
            items[k] = right[j].clone();
            j += 1;
        } else {
            items[k] = left[i].clone();
            i += 1;
        }
        k += 1;
    }
    while i < left.len() {
        items[k] = left[i].clone();
        i += 1;
        k += 1;
    }
    while j < right.len() {
        items[k] = right[j].clone();
        j += 1;
        k += 1;
    }
}

/// A tiny SplitMix64-style pseudo-random value (no external crate needed).
fn pseudo_random(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mock whose "closures" are selected by id: 0 = double, 1 = isEven,
    /// 2 = `<` comparator on Ints.
    struct Calc;
    impl StdContext for Calc {
        fn call_closure(&mut self, id: usize, args: Vec<SwiftValue>) -> StdResult {
            let n = |v: &SwiftValue| match v {
                SwiftValue::Int(i) => i.raw,
                _ => 0,
            };
            Ok(match id {
                0 => SwiftValue::int(n(&args[0]) * 2),
                1 => SwiftValue::Bool(n(&args[0]) % 2 == 0),
                2 => SwiftValue::Bool(n(&args[0]) < n(&args[1])),
                _ => SwiftValue::Nil,
            })
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            unreachable!()
        }

        fn call_closure_inout(
            &mut self,
            id: usize,
            accumulator: SwiftValue,
            element: SwiftValue,
        ) -> StdResult {
            assert_eq!(id, 3);
            let SwiftValue::Array(values) = accumulator else {
                panic!("test accumulator must be an array");
            };
            let SwiftValue::Int(value) = element else {
                panic!("test element must be an Int");
            };
            let mut updated = values.as_ref().clone();
            updated.push(SwiftValue::int(value.raw * 10));
            Ok(array(updated))
        }
    }

    fn ints(xs: &[i128]) -> Vec<SwiftValue> {
        xs.iter().map(|&x| SwiftValue::int(x)).collect()
    }
    fn clo(id: usize) -> Arg {
        Arg::positional(SwiftValue::Closure(id))
    }

    #[test]
    fn map_filter_reduce() {
        let mut c = Calc;
        assert_eq!(
            map(&mut c, ints(&[1, 2, 3]), vec![clo(0)]).unwrap(),
            array(ints(&[2, 4, 6]))
        );
        assert_eq!(
            filter(&mut c, ints(&[1, 2, 3, 4]), vec![clo(1)]).unwrap(),
            array(ints(&[2, 4]))
        );
        assert_eq!(
            reduce(
                &mut c,
                ints(&[1, 2, 3]),
                vec![
                    Arg {
                        label: Some("into".into()),
                        value: array(Vec::new()),
                        static_ty: None,
                    },
                    Arg::positional(SwiftValue::Closure(3)),
                ],
            )
            .unwrap(),
            array(ints(&[10, 20, 30]))
        );
    }

    #[test]
    fn sorted_natural_and_by() {
        let mut c = Calc;
        assert_eq!(
            sorted(&mut c, ints(&[3, 1, 2]), vec![]).unwrap(),
            array(ints(&[1, 2, 3]))
        );
        // sorted(by: <) keeps ascending; comparator id 2 is `<`.
        assert_eq!(
            sorted(&mut c, ints(&[3, 1, 2]), vec![clo(2)]).unwrap(),
            array(ints(&[1, 2, 3]))
        );
    }

    #[test]
    fn search_and_predicates() {
        let mut c = Calc;
        assert_eq!(
            contains(
                &mut c,
                ints(&[1, 2, 3]),
                vec![Arg::positional(SwiftValue::int(2))]
            )
            .unwrap(),
            SwiftValue::Bool(true)
        );
        assert_eq!(
            all_satisfy(&mut c, ints(&[2, 4]), vec![clo(1)]).unwrap(),
            SwiftValue::Bool(true)
        );
        assert_eq!(
            first_where(&mut c, ints(&[1, 3, 4]), vec![clo(1)]).unwrap(),
            SwiftValue::int(4)
        );
    }

    #[test]
    fn shape_and_minmax() {
        let mut c = Calc;
        assert_eq!(
            reversed(&mut c, ints(&[1, 2, 3]), vec![]).unwrap(),
            crate::reversedcollection::make_reversed_collection(ints(&[1, 2, 3]))
        );
        assert_eq!(
            prefix(
                &mut c,
                ints(&[1, 2, 3]),
                vec![Arg::positional(SwiftValue::int(2))]
            )
            .unwrap(),
            array(ints(&[1, 2]))
        );
        assert_eq!(
            min_by(&mut c, ints(&[3, 1, 2]), vec![]).unwrap(),
            SwiftValue::int(1)
        );
        assert_eq!(
            max_by(&mut c, ints(&[3, 1, 2]), vec![]).unwrap(),
            SwiftValue::int(3)
        );
    }

    #[test]
    fn negative_prefix_and_drop_counts_trap() {
        let mut c = Calc;
        let neg = || vec![Arg::positional(SwiftValue::int(-1))];
        for (name, result) in [
            ("prefix", prefix(&mut c, ints(&[1, 2, 3]), neg())),
            ("suffix", suffix(&mut c, ints(&[1, 2, 3]), neg())),
            ("dropFirst", drop_first(&mut c, ints(&[1, 2, 3]), neg())),
            ("dropLast", drop_last(&mut c, ints(&[1, 2, 3]), neg())),
        ] {
            assert!(
                matches!(result, Err(StdError::Error(EvalError::Trap(_)))),
                "{name}(-1) should trap, not clamp: {result:?}"
            );
        }
        // A non-negative count still works (dropFirst(1) drops one).
        assert_eq!(
            drop_first(
                &mut c,
                ints(&[1, 2, 3]),
                vec![Arg::positional(SwiftValue::int(1))]
            )
            .unwrap(),
            array(ints(&[2, 3]))
        );
    }

    #[test]
    fn shuffled_preserves_multiset() {
        let mut c = Calc;
        let out = shuffled(&mut c, ints(&[1, 2, 3, 4, 5]), vec![]).unwrap();
        let mut got = seq_of(out).unwrap();
        got.sort_by(|a, b| natural_cmp(a, b).unwrap());
        assert_eq!(got, ints(&[1, 2, 3, 4, 5]));
    }

    #[test]
    fn joined_strings() {
        let mut c = Calc;
        let items = vec![SwiftValue::Str("a".into()), SwiftValue::Str("b".into())];
        let out = joined(
            &mut c,
            items,
            vec![Arg {
                label: Some("separator".into()),
                value: SwiftValue::Str("-".into()),

                static_ty: None,
            }],
        )
        .unwrap();
        assert_eq!(out, SwiftValue::Str("a-b".into()));
    }
}
