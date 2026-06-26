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

use qswift_core::{
    BuiltinReceiver, EvalError, Interpreter, MethodEntry, Outcome, StdContext, StdError, StdResult,
    SwiftValue,
};

/// Register the `String` intrinsics of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    let s = BuiltinReceiver::String;
    interp.register_property(s, "count", count);
    interp.register_property(s, "isEmpty", is_empty);
    interp.register_property(s, "first", first);
    interp.register_property(s, "last", last);

    // `Character` predicate properties. A Character is a single-grapheme
    // String, so these classify its leading Unicode scalar.
    interp.register_property(s, "isLetter", is_letter);
    interp.register_property(s, "isNumber", is_number);
    interp.register_property(s, "isWholeNumber", is_number);
    interp.register_property(s, "isWhitespace", is_whitespace);
    interp.register_property(s, "isNewline", is_newline);
    interp.register_property(s, "isUppercase", is_uppercase);
    interp.register_property(s, "isLowercase", is_lowercase);
    interp.register_property(s, "isASCII", is_ascii);
    interp.register_property(s, "isHexDigit", is_hex_digit);

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

    interp.register_intrinsic(s, "append", MethodEntry { mutating: true, func: append });
}

/// Segment a string into extended grapheme clusters (pragmatic UAX #29 subset).
pub fn graphemes(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        i += 1;
        loop {
            if i >= chars.len() {
                break;
            }
            let prev = chars[i - 1];
            let cur = chars[i];
            // CRLF stays together.
            if prev == '\r' && cur == '\n' {
                i += 1;
                continue;
            }
            // Extend (combining marks / variation selectors) and ZWJ join.
            if is_extend(cur) || cur == ZWJ {
                i += 1;
                continue;
            }
            // A ZWJ glues whatever follows (emoji sequences).
            if prev == ZWJ {
                i += 1;
                continue;
            }
            // Pair regional indicators (flags): join only when the run from the
            // cluster start is currently odd in length.
            if is_regional(prev) && is_regional(cur) {
                let run = chars[start..i].iter().rev().take_while(|c| is_regional(**c)).count();
                if run % 2 == 1 {
                    i += 1;
                    continue;
                }
            }
            break;
        }
        out.push(chars[start..i].iter().collect());
    }
    out
}

const ZWJ: char = '\u{200D}';

fn is_regional(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

/// Combining marks and variation selectors that extend a grapheme cluster.
fn is_extend(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F   // combining diacritical marks
        | 0x0483..=0x0489
        | 0x0591..=0x05BD
        | 0x0610..=0x061A
        | 0x064B..=0x065F
        | 0x0670
        | 0x06D6..=0x06DC
        | 0x0E31 | 0x0E34..=0x0E3A
        | 0x1AB0..=0x1AFF // combining diacritical marks extended
        | 0x1DC0..=0x1DFF // combining diacritical marks supplement
        | 0x20D0..=0x20FF // combining diacritical marks for symbols
        | 0xFE00..=0xFE0F // variation selectors
        | 0xFE20..=0xFE2F // combining half marks
        | 0xE0100..=0xE01EF // variation selectors supplement
    )
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

// ---- Character predicates --------------------------------------------------

/// The leading Unicode scalar of a (Character) value, if any.
fn first_scalar(recv: &SwiftValue) -> Result<Option<char>, StdError> {
    Ok(str_of(recv)?.chars().next())
}

/// Classify the leading scalar with `pred`; an empty value is `false`.
fn classify(recv: SwiftValue, pred: impl Fn(char) -> bool) -> StdResult {
    Ok(SwiftValue::Bool(first_scalar(&recv)?.is_some_and(pred)))
}

fn is_letter(recv: SwiftValue) -> StdResult {
    classify(recv, |c| c.is_alphabetic())
}

fn is_number(recv: SwiftValue) -> StdResult {
    classify(recv, |c| c.is_numeric())
}

fn is_whitespace(recv: SwiftValue) -> StdResult {
    classify(recv, |c| c.is_whitespace())
}

fn is_newline(recv: SwiftValue) -> StdResult {
    classify(recv, |c| {
        matches!(
            c,
            '\n' | '\r'
                | '\u{0B}'
                | '\u{0C}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        )
    })
}

fn is_uppercase(recv: SwiftValue) -> StdResult {
    classify(recv, |c| c.is_uppercase())
}

fn is_lowercase(recv: SwiftValue) -> StdResult {
    classify(recv, |c| c.is_lowercase())
}

fn is_ascii(recv: SwiftValue) -> StdResult {
    classify(recv, |c| c.is_ascii())
}

fn is_hex_digit(recv: SwiftValue) -> StdResult {
    classify(recv, |c| c.is_ascii_hexdigit())
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
    fn split_yields_parts() {
        let mut m = M;
        match split(&mut m, s("a,b,c"), vec![s(",")]).unwrap().result {
            SwiftValue::Array(parts) => assert_eq!(parts.len(), 3),
            _ => panic!("split should yield an array"),
        }
    }
}
