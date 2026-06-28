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

use tswift_core::{
    BuiltinReceiver, Captures, EvalError, Interpreter, MethodEntry, Outcome, Regex, StdContext,
    StdError, StdResult, SwiftValue,
};

/// Register the `String` intrinsics of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    let s = BuiltinReceiver::String;
    interp.register_property(s, "count", count);
    interp.register_property(s, "isEmpty", is_empty);
    interp.register_property(s, "first", first);
    interp.register_property(s, "last", last);

    // `Character` predicate properties. A Character is a single-grapheme
    // String, so these classify the whole cluster: `isASCII` requires every
    // scalar to be ASCII, and digit-like predicates require a single scalar so
    // an enclosed digit (e.g. a keycap `1\u{20E3}`) is not misread as a digit.
    interp.register_property(s, "isLetter", is_letter);
    interp.register_property(s, "isNumber", is_number);
    interp.register_property(s, "isWholeNumber", is_whole_number);
    interp.register_property(s, "isWhitespace", is_whitespace);
    interp.register_property(s, "isNewline", is_newline);
    interp.register_property(s, "isUppercase", is_uppercase);
    interp.register_property(s, "isLowercase", is_lowercase);
    interp.register_property(s, "isASCII", is_ascii);
    interp.register_property(s, "isHexDigit", is_hex_digit);
    interp.register_property(s, "description", description);
    interp.register_property(s, "debugDescription", debug_description);
    interp.register_property(s, "hashValue", hash_value);

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
    pure("uppercased", uppercased);
    pure("lowercased", lowercased);
    pure("hasPrefix", has_prefix);
    pure("hasSuffix", has_suffix);
    pure("contains", contains);
    pure("firstMatch", first_match);
    pure("wholeMatch", whole_match);
    pure("prefixMatch", prefix_match);
    pure("matches", matches);
    pure("replacing", replacing);
    pure("prefix", prefix);
    pure("suffix", suffix);
    pure("split", split);
    pure("reversed", reversed);

    interp.register_intrinsic(
        s,
        "append",
        MethodEntry {
            mutating: true,
            func: append,
        },
    );
    interp.register_intrinsic(
        s,
        "removeAll",
        MethodEntry {
            mutating: true,
            func: remove_all,
        },
    );
    interp.register_intrinsic(
        s,
        "reserveCapacity",
        MethodEntry {
            mutating: true,
            func: reserve_capacity,
        },
    );
}

/// Segment a string into extended grapheme clusters (Swift `Character`s).
///
/// Re-exported from `tswift-core` so the interpreter's string iteration and
/// these `String` intrinsics segment identically.
pub use tswift_core::graphemes;

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

// ---- Character predicates --------------------------------------------------

/// The leading Unicode scalar of a (Character) value, if any.
fn first_scalar(recv: &SwiftValue) -> Result<Option<char>, StdError> {
    Ok(str_of(recv)?.chars().next())
}

/// The single Unicode scalar of a value, or `None` if it is empty or a
/// multi-scalar grapheme cluster (combining marks, enclosing keycaps, ZWJ
/// sequences, …). Used by predicates whose concept applies only to a lone
/// scalar, so an adorned digit/letter is not misclassified.
fn lone_scalar(recv: &SwiftValue) -> Result<Option<char>, StdError> {
    let s = str_of(recv)?;
    let mut it = s.chars();
    Ok(match (it.next(), it.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    })
}

/// Classify the leading scalar with `pred`; an empty value is `false`.
fn classify(recv: SwiftValue, pred: impl Fn(char) -> bool) -> StdResult {
    Ok(SwiftValue::Bool(first_scalar(&recv)?.is_some_and(pred)))
}

/// Classify a value that must be a single scalar; multi-scalar clusters and
/// empty values are `false`.
fn classify_lone(recv: SwiftValue, pred: impl Fn(char) -> bool) -> StdResult {
    Ok(SwiftValue::Bool(lone_scalar(&recv)?.is_some_and(pred)))
}

fn is_letter(recv: SwiftValue) -> StdResult {
    classify(recv, |c| c.is_alphabetic())
}

fn is_number(recv: SwiftValue) -> StdResult {
    classify_lone(recv, |c| c.is_numeric())
}

/// `Character.isWholeNumber` — a digit with an integer value (e.g. `7`), unlike
/// `isNumber` which also accepts fractions like `½`.
fn is_whole_number(recv: SwiftValue) -> StdResult {
    classify_lone(recv, |c| c.to_digit(10).is_some())
}

fn is_whitespace(recv: SwiftValue) -> StdResult {
    classify(recv, |c| c.is_whitespace())
}

fn is_newline(recv: SwiftValue) -> StdResult {
    classify(recv, |c| {
        matches!(
            c,
            '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
        )
    })
}

fn is_uppercase(recv: SwiftValue) -> StdResult {
    classify(recv, |c| c.is_uppercase())
}

fn is_lowercase(recv: SwiftValue) -> StdResult {
    classify(recv, |c| c.is_lowercase())
}

/// `Character.isASCII` — true only when every scalar of the cluster is ASCII,
/// so `e\u{301}` (a combining accent) is not ASCII.
fn is_ascii(recv: SwiftValue) -> StdResult {
    let s = str_of(&recv)?;
    Ok(SwiftValue::Bool(
        !s.is_empty() && s.chars().all(|c| c.is_ascii()),
    ))
}

fn is_hex_digit(recv: SwiftValue) -> StdResult {
    classify_lone(recv, |c| c.is_ascii_hexdigit())
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

/// `String.contains(_:)` — substring containment, or, when passed a `Regex`,
/// whether the pattern matches anywhere in the string.
fn contains(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    if let Some(re) = arg_regex(&args) {
        let chars = chars_of(&recv)?;
        return val(SwiftValue::Bool(re.find(&chars).is_some()), recv);
    }
    let needle = arg_str(&args).ok_or_else(|| type_err("contains expects a string".into()))?;
    val(SwiftValue::Bool(str_of(&recv)?.contains(&needle)), recv)
}

/// `String.firstMatch(of:)` — the leftmost match of `regex`, or `nil`.
fn first_match(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let re = arg_regex(&args).ok_or_else(|| type_err("firstMatch expects a Regex".into()))?;
    let chars = chars_of(&recv)?;
    val(optional_match(re.find(&chars), &chars), recv)
}

/// `String.wholeMatch(of:)` — a match spanning the entire string, or `nil`.
fn whole_match(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let re = arg_regex(&args).ok_or_else(|| type_err("wholeMatch expects a Regex".into()))?;
    let chars = chars_of(&recv)?;
    val(optional_match(re.whole_match(&chars), &chars), recv)
}

/// `String.prefixMatch(of:)` — a match anchored at the start, or `nil`.
fn prefix_match(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let re = arg_regex(&args).ok_or_else(|| type_err("prefixMatch expects a Regex".into()))?;
    let chars = chars_of(&recv)?;
    val(optional_match(re.prefix_match(&chars), &chars), recv)
}

/// `String.matches(of:)` — every non-overlapping match, left to right.
fn matches(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let re = arg_regex(&args).ok_or_else(|| type_err("matches expects a Regex".into()))?;
    let chars = chars_of(&recv)?;
    let out: Vec<SwiftValue> = re
        .find_all(&chars)
        .iter()
        .map(|m| match_tuple(m, &chars))
        .collect();
    val(SwiftValue::Array(Rc::new(out)), recv)
}

/// `String.replacing(_:with:)` — replace every match of `regex` with a string.
fn replacing(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let re = arg_regex(&args).ok_or_else(|| type_err("replacing expects a Regex".into()))?;
    let with = args
        .iter()
        .find_map(|a| match a {
            SwiftValue::Str(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let chars = chars_of(&recv)?;
    let mut out = String::new();
    let mut cursor = 0;
    for m in re.find_all(&chars) {
        let (s, e) = m.whole();
        out.extend(&chars[cursor..s]);
        out.push_str(&with);
        cursor = e;
    }
    out.extend(&chars[cursor..]);
    val(SwiftValue::Str(out), recv)
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
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Str(s),
    })
}

/// `String.description` — the string itself.
fn description(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Str(str_of(&recv)?))
}

/// `String.debugDescription` — the string wrapped in quotes with the common
/// escapes applied, matching Swift's debug rendering for simple text.
fn debug_description(recv: SwiftValue) -> StdResult {
    let s = str_of(&recv)?;
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out.push('"');
    Ok(SwiftValue::Str(out))
}

/// `String.hashValue` — a deterministic per-run hash. Swift seeds its hasher
/// per process, so only equal hashes for equal strings are observable; an
/// FNV-1a digest of the bytes models that with a stable witness.
fn hash_value(recv: SwiftValue) -> StdResult {
    let s = str_of(&recv)?;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Ok(SwiftValue::int(i128::from(h as i64)))
}

/// `String.removeAll(keepingCapacity:)` — empty the string in place.
fn remove_all(_c: &mut dyn StdContext, _recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    val(SwiftValue::Void, SwiftValue::Str(String::new()))
}

/// `String.reserveCapacity(_:)` — a no-op here; storage growth is implicit.
fn reserve_capacity(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let s = str_of(&recv)?;
    val(SwiftValue::Void, SwiftValue::Str(s))
}

// ---- helpers ---------------------------------------------------------------

type Outcomes = Result<Outcome, StdError>;

fn val(result: SwiftValue, receiver: SwiftValue) -> Outcomes {
    Ok(Outcome { result, receiver })
}

fn type_err(msg: String) -> StdError {
    StdError::Error(EvalError::Type(msg))
}

/// The first `Regex` argument, if any.
fn arg_regex(args: &[SwiftValue]) -> Option<Rc<Regex>> {
    args.iter().find_map(|a| match a {
        SwiftValue::Regex(r) => Some(r.clone()),
        _ => None,
    })
}

/// The receiver string as a `char` vector (regex matching is scalar-indexed).
fn chars_of(recv: &SwiftValue) -> Result<Vec<char>, StdError> {
    Ok(str_of(recv)?.chars().collect())
}

/// Render a capture as a tuple of substrings: index 0 is the whole match, then
/// one element per capture group (`nil` for a group that did not participate).
/// `match.0`, `match.1`, … then read like Swift's `Regex.Match` output tuple.
fn match_tuple(caps: &Captures, chars: &[char]) -> SwiftValue {
    let parts = caps
        .groups
        .iter()
        .map(|g| match g {
            Some((s, e)) => SwiftValue::Str(chars[*s..*e].iter().collect()),
            None => SwiftValue::Nil,
        })
        .collect();
    SwiftValue::tuple(parts)
}

/// Wrap an optional capture as a Swift `Optional`: the match tuple, or `nil`.
fn optional_match(caps: Option<Captures>, chars: &[char]) -> SwiftValue {
    match caps {
        Some(c) => match_tuple(&c, chars),
        None => SwiftValue::Nil,
    }
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
    fn describe_and_hash() {
        assert_eq!(description(s("hi")).unwrap(), s("hi"));
        assert_eq!(
            debug_description(s("a\tb\"c")).unwrap(),
            s("\"a\\tb\\\"c\"")
        );
        assert_eq!(hash_value(s("abc")).unwrap(), hash_value(s("abc")).unwrap());
        assert_ne!(hash_value(s("abc")).unwrap(), hash_value(s("abd")).unwrap());
    }

    #[test]
    fn remove_all_and_reserve() {
        let mut m = M;
        assert_eq!(
            remove_all(&mut m, s("keep"), vec![]).unwrap().receiver,
            s("")
        );
        let r = reserve_capacity(&mut m, s("grow"), vec![SwiftValue::int(99)]).unwrap();
        assert_eq!(r.receiver, s("grow"));
        assert_eq!(r.result, SwiftValue::Void);
    }

    #[test]
    fn transforms() {
        let mut m = M;
        assert_eq!(uppercased(&mut m, s("hi"), vec![]).unwrap().result, s("HI"));
        assert_eq!(lowercased(&mut m, s("HI"), vec![]).unwrap().result, s("hi"));
        assert_eq!(
            has_prefix(&mut m, s("swift"), vec![s("sw")])
                .unwrap()
                .result,
            SwiftValue::Bool(true)
        );
        assert_eq!(
            contains(&mut m, s("hello world"), vec![s("o w")])
                .unwrap()
                .result,
            SwiftValue::Bool(true)
        );
        assert_eq!(
            prefix(&mut m, s("hello"), vec![SwiftValue::int(3)])
                .unwrap()
                .result,
            s("hel")
        );
        assert_eq!(
            suffix(&mut m, s("hello"), vec![SwiftValue::int(2)])
                .unwrap()
                .result,
            s("lo")
        );
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
    fn character_predicates_classify_leading_scalar() {
        let t = SwiftValue::Bool(true);
        let f = SwiftValue::Bool(false);
        assert_eq!(is_letter(s("A")).unwrap(), t);
        assert_eq!(is_letter(s("7")).unwrap(), f);
        assert_eq!(is_number(s("7")).unwrap(), t);
        assert_eq!(is_number(s("A")).unwrap(), f);
        assert_eq!(is_whitespace(s(" ")).unwrap(), t);
        assert_eq!(is_uppercase(s("A")).unwrap(), t);
        assert_eq!(is_lowercase(s("a")).unwrap(), t);
        assert_eq!(is_uppercase(s("a")).unwrap(), f);
        assert_eq!(is_newline(s("\n")).unwrap(), t);
        assert_eq!(is_ascii(s("z")).unwrap(), t);
        assert_eq!(is_ascii(s("\u{00E9}")).unwrap(), f);
        assert_eq!(is_hex_digit(s("F")).unwrap(), t);
        assert_eq!(is_hex_digit(s("G")).unwrap(), f);
        // An empty value classifies as false rather than trapping.
        assert_eq!(is_letter(s("")).unwrap(), f);
    }

    #[test]
    fn character_predicates_consider_whole_cluster() {
        let t = SwiftValue::Bool(true);
        let f = SwiftValue::Bool(false);
        // isASCII looks at every scalar: a combining accent is not ASCII.
        assert_eq!(is_ascii(s("e\u{301}")).unwrap(), f);
        // A digit with an enclosing keycap is not a number/whole-number/hex.
        let keycap = s("1\u{20E3}");
        assert_eq!(is_number(keycap.clone()).unwrap(), f);
        assert_eq!(is_whole_number(keycap.clone()).unwrap(), f);
        assert_eq!(is_hex_digit(keycap).unwrap(), f);
        // isWholeNumber rejects fractions that isNumber accepts.
        assert_eq!(is_number(s("\u{00BD}")).unwrap(), t); // ½ is a number
        assert_eq!(is_whole_number(s("\u{00BD}")).unwrap(), f);
        assert_eq!(is_whole_number(s("7")).unwrap(), t);
    }

    #[test]
    fn split_yields_parts() {
        let mut m = M;
        match split(&mut m, s("a,b,c"), vec![s(",")]).unwrap().result {
            SwiftValue::Array(parts) => assert_eq!(parts.len(), 3),
            _ => panic!("split should yield an array"),
        }
    }

    fn re(p: &str) -> SwiftValue {
        SwiftValue::Regex(Rc::new(Regex::compile(p).unwrap()))
    }

    #[test]
    fn contains_matches_regex_or_substring() {
        let mut m = M;
        assert_eq!(
            contains(&mut m, s("abc123"), vec![re(r"\d+")])
                .unwrap()
                .result,
            SwiftValue::Bool(true)
        );
        assert_eq!(
            contains(&mut m, s("abcdef"), vec![re(r"\d+")])
                .unwrap()
                .result,
            SwiftValue::Bool(false)
        );
    }

    #[test]
    fn first_match_returns_capture_tuple() {
        let mut m = M;
        let out = first_match(&mut m, s("order-42"), vec![re(r"(\w+)-(\d+)")])
            .unwrap()
            .result;
        assert_eq!(
            out,
            SwiftValue::tuple(vec![s("order-42"), s("order"), s("42")])
        );
    }

    #[test]
    fn first_match_misses_yield_nil() {
        let mut m = M;
        let out = first_match(&mut m, s("abc"), vec![re(r"\d+")])
            .unwrap()
            .result;
        assert_eq!(out, SwiftValue::Nil);
    }

    #[test]
    fn whole_match_requires_full_string() {
        let mut m = M;
        assert!(matches!(
            whole_match(&mut m, s("abc"), vec![re(r"[a-z]+")])
                .unwrap()
                .result,
            SwiftValue::Tuple(..)
        ));
        assert_eq!(
            whole_match(&mut m, s("abc1"), vec![re(r"[a-z]+")])
                .unwrap()
                .result,
            SwiftValue::Nil
        );
    }

    #[test]
    fn matches_collects_every_match() {
        let mut m = M;
        match matches(&mut m, s("a1 b22 c333"), vec![re(r"\d+")])
            .unwrap()
            .result
        {
            SwiftValue::Array(items) => {
                let firsts: Vec<SwiftValue> = items
                    .iter()
                    .map(|t| match t {
                        SwiftValue::Tuple(v, _) => v[0].clone(),
                        _ => panic!("match should be a tuple"),
                    })
                    .collect();
                assert_eq!(firsts, vec![s("1"), s("22"), s("333")]);
            }
            _ => panic!("matches should yield an array"),
        }
    }

    #[test]
    fn replacing_substitutes_every_match() {
        let mut m = M;
        assert_eq!(
            replacing(&mut m, s("2024-01-02"), vec![re(r"\d+"), s("#")])
                .unwrap()
                .result,
            s("#-#-#")
        );
    }
}
