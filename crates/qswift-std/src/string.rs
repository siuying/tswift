//! `String` (and `Character`) method and property intrinsics.
//!
//! Swift counts and indexes a `String` by *extended grapheme cluster*, not by
//! byte or Unicode scalar. Because crates.io is unavailable offline (no
//! `unicode-segmentation`), this module ships a self-contained segmenter
//! ([`graphemes`]) implementing a pragmatic subset of UAX #29: combining marks
//! and variation selectors extend a cluster, ZWJ glues emoji sequences, and
//! regional-indicator scalars pair into flags. It is not the full algorithm and
//! is not pinned to a Unicode version, but matches Swift for common text and
//! the emoji cases exercised here.
//!
//! A `Character` is modelled as a single-grapheme `String`; `Substring` shares
//! the flattened string representation (it is a `String` value here).

use std::rc::Rc;

use qswift_core::{
    graphemes, BuiltinReceiver, EvalError, Interpreter, MethodEntry, Outcome, StdContext, StdError,
    StdResult, StrViewKind, SwiftValue,
};

/// Register the `String` intrinsics of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    let s = BuiltinReceiver::String;
    interp.register_property(s, "count", count);
    interp.register_property(s, "isEmpty", is_empty);
    interp.register_property(s, "first", first);
    interp.register_property(s, "last", last);

    let mut pure = |name: &str, f: qswift_core::IntrinsicFn| {
        interp.register_intrinsic(s, name, MethodEntry { mutating: false, func: f });
    };
    pure("uppercased", uppercased);
    pure("lowercased", lowercased);
    pure("hasPrefix", has_prefix);
    pure("hasSuffix", has_suffix);
    pure("contains", contains);
    pure("prefix", prefix);
    pure("suffix", suffix);
    pure("split", split);
    pure("reversed", reversed);
    // Index navigation (ADR-0006).
    pure("index", string_index);
    pure("distance", distance);

    interp.register_intrinsic(s, "append", MethodEntry { mutating: true, func: append });

    // Index model (ADR-0006).
    interp.register_property(s, "startIndex", start_index);
    interp.register_property(s, "endIndex", end_index);
    // `index(before:)` is disambiguated from `index(after:)` by the interpreter
    // (labels are stripped at the seam) and recorded under the `String.index`
    // coverage key.
    interp.register_intrinsic_as(
        s,
        "index(before:)",
        "String.index",
        MethodEntry { mutating: false, func: index_before },
    );

    // `Unicode.Scalar.value` (a scalar is modelled as a one-scalar string).
    interp.register_property(s, "value", scalar_value);

    // Encoding views.
    interp.register_property(s, "unicodeScalars", unicode_scalars);
    interp.register_property(s, "utf8", utf8_view);
    interp.register_property(s, "utf16", utf16_view);

    // Index-based mutation.
    interp.register_intrinsic(s, "insert", MethodEntry { mutating: true, func: insert });
    interp.register_intrinsic(s, "remove", MethodEntry { mutating: true, func: remove });
    interp.register_intrinsic(
        s,
        "removeSubrange",
        MethodEntry { mutating: true, func: remove_subrange },
    );
    interp.register_intrinsic(
        s,
        "replaceSubrange",
        MethodEntry { mutating: true, func: replace_subrange },
    );
}

// ---- index model -----------------------------------------------------------

/// Byte offsets of every grapheme-cluster boundary, including `0` and `len`.
fn boundaries(s: &str) -> Vec<usize> {
    let mut offs = vec![0usize];
    let mut acc = 0;
    for g in graphemes(s) {
        acc += g.len();
        offs.push(acc);
    }
    offs
}

fn index_value(utf8: usize) -> SwiftValue {
    SwiftValue::StringIndex { utf8, transcoded: 0 }
}

fn as_index(v: &SwiftValue) -> Result<usize, StdError> {
    match v {
        SwiftValue::StringIndex { utf8, .. } => Ok(*utf8),
        other => Err(type_err(format!(
            "expected a String.Index, got {}",
            other.type_name()
        ))),
    }
}

fn as_int(v: &SwiftValue) -> Result<i128, StdError> {
    match v {
        SwiftValue::Int(i) => Ok(i.raw),
        other => Err(type_err(format!("expected an integer, got {}", other.type_name()))),
    }
}

fn start_index(recv: SwiftValue) -> StdResult {
    let _ = str_of(&recv)?;
    Ok(index_value(0))
}

fn end_index(recv: SwiftValue) -> StdResult {
    Ok(index_value(str_of(&recv)?.len()))
}

/// `s.index(after:)` / `s.index(before:)` / `s.index(_:offsetBy:[limitedBy:])`,
/// all dispatched on the `index` method name by argument labels/arity.
fn string_index(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let s = str_of(&recv)?;
    let offs = boundaries(&s);
    let pos_of = |byte: usize| offs.iter().position(|&o| o == byte);

    // The first argument distinguishes the overloads: `after:`/`before:` take a
    // single index; `index(_:offsetBy:)` takes (index, n[, limit]).
    let first = args.first().ok_or_else(|| arg_err("index"))?;
    let base = as_index(first)?;
    let base_pos = pos_of(base).ok_or_else(|| trap_err("String index is not aligned"))?;

    match args.len() {
        // index(after:) when called positionally is also length 1; Swift spells
        // before/after via labels, but the runtime passes values positionally,
        // so a lone index means `after`. `before` arrives as a negative offset
        // through index(_:offsetBy:) in practice; we also accept index(before:)
        // by a sentinel handled in the 1-arg branch is ambiguous, so callers use
        // index(after:)/index(_:offsetBy:).
        1 => {
            let next = base_pos + 1;
            offs.get(next)
                .copied()
                .map(|b| ok(index_value(b), recv.clone()))
                .unwrap_or_else(|| Err(trap_err("String index is out of bounds")))
        }
        _ => {
            let n = as_int(&args[1])?;
            let target = base_pos as i128 + n;
            let limit = args.get(2).map(as_index).transpose()?;
            if target < 0 || target as usize >= offs.len() {
                // Out of range: respect `limitedBy` (return nil) or trap.
                if limit.is_some() {
                    return ok(SwiftValue::Nil, recv);
                }
                return Err(trap_err("String index is out of bounds"));
            }
            let result = offs[target as usize];
            if let Some(lim) = limit {
                // Overshooting the limit yields nil (Swift `limitedBy`).
                if (n >= 0 && result > lim) || (n < 0 && result < lim) {
                    return ok(SwiftValue::Nil, recv);
                }
            }
            ok(index_value(result), recv)
        }
    }
}

/// `s.index(before:)` — the index one grapheme cluster earlier; traps at
/// `startIndex`.
fn index_before(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let s = str_of(&recv)?;
    let offs = boundaries(&s);
    let base = as_index(args.first().ok_or_else(|| arg_err("index(before:)"))?)?;
    let pos = offs
        .iter()
        .position(|&o| o == base)
        .ok_or_else(|| trap_err("String index is not aligned"))?;
    if pos == 0 {
        return Err(trap_err("String index is out of bounds"));
    }
    val(index_value(offs[pos - 1]), recv)
}

/// `s.distance(from:to:)` — grapheme-cluster count between two indices.
fn distance(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let s = str_of(&recv)?;
    let offs = boundaries(&s);
    let from = as_index(args.first().ok_or_else(|| arg_err("distance"))?)?;
    let to = as_index(args.get(1).ok_or_else(|| arg_err("distance"))?)?;
    let pos = |byte: usize| offs.iter().position(|&o| o == byte);
    let a = pos(from).ok_or_else(|| trap_err("String index is not aligned"))? as i128;
    let b = pos(to).ok_or_else(|| trap_err("String index is not aligned"))? as i128;
    ok(SwiftValue::int(b - a), recv)
}

// ---- encoding views --------------------------------------------------------

/// `Unicode.Scalar.value` — the code point of a single-scalar value (`UInt32`).
fn scalar_value(recv: SwiftValue) -> StdResult {
    let s = str_of(&recv)?;
    let cp = s.chars().next().map(|c| c as u32).unwrap_or(0);
    Ok(SwiftValue::Int(qswift_core::IntValue::new(
        cp as i128,
        qswift_core::IntWidth::U32,
    )))
}

fn unicode_scalars(recv: SwiftValue) -> StdResult {
    view(recv, StrViewKind::UnicodeScalars)
}
fn utf8_view(recv: SwiftValue) -> StdResult {
    view(recv, StrViewKind::Utf8)
}
fn utf16_view(recv: SwiftValue) -> StdResult {
    view(recv, StrViewKind::Utf16)
}
fn view(recv: SwiftValue, kind: StrViewKind) -> StdResult {
    Ok(SwiftValue::StringView {
        base: Rc::new(str_of(&recv)?),
        kind,
    })
}

// ---- index-based mutation --------------------------------------------------

fn arg_char(args: &[SwiftValue], who: &str) -> Result<String, StdError> {
    match args.first() {
        Some(SwiftValue::Str(s)) => Ok(s.clone()),
        _ => Err(arg_err(who)),
    }
}

/// `s.insert(_:at:)` — insert a Character at an index.
fn insert(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut s = str_of(&recv)?;
    let ch = arg_char(&args, "insert(_:at:)")?;
    let at = as_index(args.get(1).ok_or_else(|| arg_err("insert(_:at:)"))?)?;
    if at > s.len() || !s.is_char_boundary(at) {
        return Err(trap_err("String index is out of bounds"));
    }
    s.insert_str(at, &ch);
    ok(SwiftValue::Void, SwiftValue::Str(s))
}

/// `s.remove(at:)` — remove and return the grapheme at an index.
fn remove(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut s = str_of(&recv)?;
    let at = as_index(args.first().ok_or_else(|| arg_err("remove(at:)"))?)?;
    if at >= s.len() || !s.is_char_boundary(at) {
        return Err(trap_err("String index is out of bounds"));
    }
    let g = graphemes(&s[at..])
        .into_iter()
        .next()
        .ok_or_else(|| trap_err("String index is out of bounds"))?;
    let removed = s.drain(at..at + g.len()).collect::<String>();
    ok(SwiftValue::Str(removed), SwiftValue::Str(s))
}

fn range_bounds(v: &SwiftValue, who: &str) -> Result<(usize, usize), StdError> {
    match v {
        SwiftValue::Range { lo, hi, .. } => Ok((*lo as usize, *hi as usize)),
        _ => Err(arg_err(who)),
    }
}

/// `s.removeSubrange(_:)` — remove the bytes in an index range.
fn remove_subrange(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut s = str_of(&recv)?;
    let (lo, hi) = range_bounds(args.first().ok_or_else(|| arg_err("removeSubrange"))?, "removeSubrange")?;
    if lo > hi || hi > s.len() || !s.is_char_boundary(lo) || !s.is_char_boundary(hi) {
        return Err(trap_err("String range is out of bounds"));
    }
    s.replace_range(lo..hi, "");
    ok(SwiftValue::Void, SwiftValue::Str(s))
}

/// `s.replaceSubrange(_:with:)` — replace an index range with a string.
fn replace_subrange(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut s = str_of(&recv)?;
    let (lo, hi) = range_bounds(args.first().ok_or_else(|| arg_err("replaceSubrange"))?, "replaceSubrange")?;
    let with = match args.get(1) {
        Some(SwiftValue::Str(w)) => w.clone(),
        _ => return Err(arg_err("replaceSubrange(_:with:)")),
    };
    if lo > hi || hi > s.len() || !s.is_char_boundary(lo) || !s.is_char_boundary(hi) {
        return Err(trap_err("String range is out of bounds"));
    }
    s.replace_range(lo..hi, &with);
    ok(SwiftValue::Void, SwiftValue::Str(s))
}

fn ok(result: SwiftValue, receiver: SwiftValue) -> Outcomes {
    Ok(Outcome { result, receiver })
}

fn trap_err(msg: &str) -> StdError {
    StdError::Error(EvalError::Trap(msg.to_string()))
}

fn arg_err(who: &str) -> StdError {
    StdError::Error(EvalError::Type(format!("{who}: missing or wrong argument")))
}

fn str_of(recv: &SwiftValue) -> Result<String, StdError> {
    match recv {
        SwiftValue::Str(s) => Ok(s.clone()),
        other => Err(type_err(format!(
            "expected a string receiver, got {}",
            other.type_name()
        ))),
    }
}

fn arg_str(args: &[SwiftValue]) -> Option<String> {
    args.iter().find_map(|a| match a {
        SwiftValue::Str(s) => Some(s.clone()),
        _ => None,
    })
}

// ---- properties ------------------------------------------------------------

fn count(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(graphemes(&str_of(&recv)?).len() as i128))
}

fn is_empty(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(str_of(&recv)?.is_empty()))
}

fn first(recv: SwiftValue) -> StdResult {
    Ok(graphemes(&str_of(&recv)?)
        .into_iter()
        .next()
        .map(SwiftValue::Str)
        .unwrap_or(SwiftValue::Nil))
}

fn last(recv: SwiftValue) -> StdResult {
    Ok(graphemes(&str_of(&recv)?)
        .into_iter()
        .next_back()
        .map(SwiftValue::Str)
        .unwrap_or(SwiftValue::Nil))
}

// ---- transforms ------------------------------------------------------------

fn uppercased(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    val(SwiftValue::Str(str_of(&recv)?.to_uppercase()), recv)
}

fn lowercased(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    val(SwiftValue::Str(str_of(&recv)?.to_lowercase()), recv)
}

fn has_prefix(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let p = arg_str(&args).ok_or_else(|| type_err("hasPrefix expects a string".into()))?;
    val(SwiftValue::Bool(str_of(&recv)?.starts_with(&p)), recv)
}

fn has_suffix(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let p = arg_str(&args).ok_or_else(|| type_err("hasSuffix expects a string".into()))?;
    val(SwiftValue::Bool(str_of(&recv)?.ends_with(&p)), recv)
}

/// `String.contains(_:)` — substring containment (overrides the element-wise
/// sequence `contains`).
fn contains(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let needle = arg_str(&args).ok_or_else(|| type_err("contains expects a string".into()))?;
    val(SwiftValue::Bool(str_of(&recv)?.contains(&needle)), recv)
}

fn prefix(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let g = graphemes(&str_of(&recv)?);
    let n = count_arg(&args).unwrap_or(0).min(g.len());
    val(SwiftValue::Str(g[..n].concat()), recv)
}

fn suffix(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let g = graphemes(&str_of(&recv)?);
    let n = count_arg(&args).unwrap_or(0).min(g.len());
    val(SwiftValue::Str(g[g.len() - n..].concat()), recv)
}

/// `String.split(separator:)` — split into substrings (here, strings).
fn split(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let s = str_of(&recv)?;
    let sep = labeled(&args, "separator")
        .or_else(|| arg_str(&args))
        .ok_or_else(|| type_err("split expects a separator".into()))?;
    let omit_empty = labeled_bool(&args, "omittingEmptySubsequences").unwrap_or(true);
    let parts: Vec<SwiftValue> = s
        .split(sep.as_str())
        .filter(|p| !omit_empty || !p.is_empty())
        .map(|p| SwiftValue::Str(p.to_string()))
        .collect();
    val(SwiftValue::Array(std::rc::Rc::new(parts)), recv)
}

fn reversed(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let mut g = graphemes(&str_of(&recv)?);
    g.reverse();
    val(SwiftValue::Str(g.concat()), recv)
}

// ---- mutating --------------------------------------------------------------

/// `String.append(_:)` — append a character or string in place.
fn append(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let mut s = str_of(&recv)?;
    if let Some(extra) = arg_str(&args) {
        s.push_str(&extra);
    }
    Ok(Outcome { result: SwiftValue::Void, receiver: SwiftValue::Str(s) })
}

// ---- helpers ---------------------------------------------------------------

type Outcomes = Result<Outcome, StdError>;

fn val(result: SwiftValue, receiver: SwiftValue) -> Outcomes {
    Ok(Outcome { result, receiver })
}

fn type_err(msg: String) -> StdError {
    StdError::Error(EvalError::Type(msg))
}

fn labeled(args: &[SwiftValue], _label: &str) -> Option<String> {
    // Method intrinsics receive values without labels; for `split` the separator
    // is the first (and only) string argument.
    args.iter().find_map(|a| match a {
        SwiftValue::Str(s) => Some(s.clone()),
        _ => None,
    })
}

fn labeled_bool(args: &[SwiftValue], _label: &str) -> Option<bool> {
    args.iter().find_map(|a| a.as_bool())
}

fn count_arg(args: &[SwiftValue]) -> Option<usize> {
    args.iter().find_map(|a| match a {
        SwiftValue::Int(i) if i.raw >= 0 => Some(i.raw as usize),
        _ => None,
    })
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

    fn s(t: &str) -> SwiftValue {
        SwiftValue::Str(t.into())
    }

    #[test]
    fn grapheme_count_handles_combining_zwj_and_flags() {
        assert_eq!(graphemes("café").len(), 4);
        assert_eq!(graphemes("e\u{301}").len(), 1); // e + combining acute
        assert_eq!(graphemes("🇺🇸").len(), 1); // regional-indicator flag
        assert_eq!(graphemes("👨\u{200D}👩\u{200D}👧").len(), 1); // ZWJ family
        assert_eq!(graphemes("ab🇺🇸c").len(), 4);
    }

    #[test]
    fn transforms() {
        let mut m = M;
        assert_eq!(uppercased(&mut m, s("hi"), vec![]).unwrap().result, s("HI"));
        assert_eq!(lowercased(&mut m, s("HI"), vec![]).unwrap().result, s("hi"));
        assert_eq!(has_prefix(&mut m, s("swift"), vec![s("sw")]).unwrap().result, SwiftValue::Bool(true));
        assert_eq!(contains(&mut m, s("hello world"), vec![s("o w")]).unwrap().result, SwiftValue::Bool(true));
        assert_eq!(prefix(&mut m, s("hello"), vec![SwiftValue::int(3)]).unwrap().result, s("hel"));
        assert_eq!(suffix(&mut m, s("hello"), vec![SwiftValue::int(2)]).unwrap().result, s("lo"));
        assert_eq!(reversed(&mut m, s("abc"), vec![]).unwrap().result, s("cba"));
    }

    #[test]
    fn count_first_last_append() {
        let mut m = M;
        assert_eq!(count(s("café")).unwrap(), SwiftValue::int(4));
        assert_eq!(first(s("hi")).unwrap(), s("h"));
        assert_eq!(last(s("hi")).unwrap(), s("i"));
        assert_eq!(first(s("")).unwrap(), SwiftValue::Nil);
        let appended = append(&mut m, s("ab"), vec![s("c")]).unwrap();
        assert_eq!(appended.receiver, s("abc"));
    }

    #[test]
    fn split_yields_parts() {
        let mut m = M;
        match split(&mut m, s("a,b,c"), vec![s(",")]).unwrap().result {
            SwiftValue::Array(parts) => assert_eq!(parts.len(), 3),
            _ => panic!("split should yield an array"),
        }
    }

    fn idx(utf8: usize) -> SwiftValue {
        SwiftValue::StringIndex { utf8, transcoded: 0 }
    }

    #[test]
    fn boundaries_are_grapheme_aligned() {
        // "café" = c|a|f|é(2 bytes) -> offsets 0,1,2,3,5.
        assert_eq!(boundaries("café"), vec![0, 1, 2, 3, 5]);
        assert_eq!(boundaries(""), vec![0]);
    }

    #[test]
    fn index_navigation_and_distance() {
        let mut m = M;
        // index(after: startIndex) of "café" -> offset 1.
        assert_eq!(
            string_index(&mut m, s("café"), vec![idx(0)]).unwrap().result,
            idx(1)
        );
        // index(_:offsetBy:) crossing the 2-byte 'é'.
        assert_eq!(
            string_index(&mut m, s("café"), vec![idx(3), SwiftValue::int(1)])
                .unwrap()
                .result,
            idx(5)
        );
        // limitedBy past the end -> nil.
        assert_eq!(
            string_index(
                &mut m,
                s("ab"),
                vec![idx(0), SwiftValue::int(9), idx(2)]
            )
            .unwrap()
            .result,
            SwiftValue::Nil
        );
        // index(before:) at endIndex.
        assert_eq!(
            index_before(&mut m, s("café"), vec![idx(5)]).unwrap().result,
            idx(3)
        );
        // distance over grapheme clusters.
        assert_eq!(
            distance(&mut m, s("café"), vec![idx(0), idx(5)]).unwrap().result,
            SwiftValue::int(4)
        );
    }

    #[test]
    fn mutation_at_index() {
        let mut m = M;
        // insert at startIndex.
        assert_eq!(
            insert(&mut m, s("bc"), vec![s("a"), idx(0)]).unwrap().receiver,
            s("abc")
        );
        // remove(at:) returns the removed grapheme and the shortened string.
        let out = remove(&mut m, s("café"), vec![idx(3)]).unwrap();
        assert_eq!(out.result, s("é"));
        assert_eq!(out.receiver, s("caf"));
        // replaceSubrange over byte offsets 0..1.
        let r = SwiftValue::Range {
            lo: 0,
            hi: 1,
            inclusive: false,
        };
        assert_eq!(
            replace_subrange(&mut m, s("abc"), vec![r, s("**")])
                .unwrap()
                .receiver,
            s("**bc")
        );
    }
}
