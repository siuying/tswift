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
//! A `Character` is modelled as a single-grapheme `String`; `Substring` is a
//! distinct `SwiftValue::Substring { base, start, end }` carrying its own view
//! into a parent string (see `substring.rs` and `tswift_core::value`).

use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, Captures, EvalError, IntValue, IntWidth, Interpreter, LabeledMethodEntry,
    MethodEntry, Outcome, Regex, StdContext, StdError, StdResult, StructObj, SwiftValue,
};

/// Register the `String` intrinsics of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    let s = BuiltinReceiver::String;
    // --- Properties shared with Substring (text-extraction-based) ---
    install_shared_text_methods(interp, s);

    // --- String-only properties ---
    interp.register_property(s, "startIndex", start_index);
    interp.register_property(s, "endIndex", end_index);
    interp.register_property(s, "indices", indices);
    // Character-view and contiguity properties (parity with Substring). A
    // `String` is its own `Character` collection, so `characters` returns self;
    // the runtime always stores contiguous UTF-8.
    interp.register_property(s, "characters", characters);
    interp.register_property(s, "isContiguousUTF8", is_contiguous_utf8);

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

    // `index` is label-aware so we can distinguish index(after:), index(before:),
    // index(_:offsetBy:), and index(_:offsetBy:limitedBy:).
    interp.register_labeled_intrinsic(
        s,
        "index",
        LabeledMethodEntry {
            mutating: false,
            func: index_labeled,
        },
    );
    // `formIndex` is intercepted by the dispatcher (it needs the inout index
    // place); registering it here records coverage and provides a fallback.
    interp.register_labeled_intrinsic(
        s,
        "formIndex",
        LabeledMethodEntry {
            mutating: true,
            func: form_index_labeled,
        },
    );
    // `distance(from:to:)` and `insert`/`remove` family.
    interp.register_labeled_intrinsic(
        s,
        "distance",
        LabeledMethodEntry {
            mutating: false,
            func: distance_labeled,
        },
    );
    interp.register_labeled_intrinsic(
        s,
        "insert",
        LabeledMethodEntry {
            mutating: true,
            func: insert_labeled,
        },
    );

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
    pure("firstMatch", first_match);
    pure("wholeMatch", whole_match);
    pure("prefixMatch", prefix_match);
    pure("matches", matches);
    pure("replacing", replacing);
    pure("prefix", prefix);
    pure("suffix", suffix);
    pure("reversed", reversed);

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
    // `makeContiguousUTF8()` — no-op mutating (runtime is always contiguous).
    interp.register_intrinsic(
        s,
        "makeContiguousUTF8",
        MethodEntry {
            mutating: true,
            func: make_contiguous_utf8,
        },
    );
    interp.register_intrinsic(
        s,
        "remove",
        MethodEntry {
            mutating: true,
            func: remove_at,
        },
    );
    interp.register_intrinsic(
        s,
        "removeSubrange",
        MethodEntry {
            mutating: true,
            func: remove_subrange,
        },
    );
    interp.register_intrinsic(
        s,
        "replaceSubrange",
        MethodEntry {
            mutating: true,
            func: replace_subrange,
        },
    );
}

/// Register the subset of String methods whose implementations work correctly
/// on **both** `String` (`SwiftValue::Str`) and `Substring`
/// (`SwiftValue::Substring`) receivers via the [`str_of`] text-materialisation
/// helper.  These methods are registered under whichever `BuiltinReceiver` the
/// caller requests; the `substring` module calls this with
/// `BuiltinReceiver::Substring`.
///
/// **Not included**: `startIndex`/`endIndex` (base-relative for Substring),
/// `index`/`distance` (need base-relative bounds), `prefix`/`suffix` (need to
/// return Substring views), Character predicates, regex methods, mutating
/// remove/insert operations — all of which have Substring-specific impls.
pub(super) fn install_shared_text_methods(interp: &mut Interpreter<'_>, s: BuiltinReceiver) {
    // --- Properties ---
    interp.register_property(s, "count", count);
    interp.register_property(s, "isEmpty", is_empty);
    interp.register_property(s, "first", first);
    interp.register_property(s, "last", last);
    interp.register_property(s, "description", description);
    interp.register_property(s, "debugDescription", debug_description);
    interp.register_property(s, "hashValue", hash_value);
    interp.register_property(s, "utf8", utf8_view);
    interp.register_property(s, "utf16", utf16_view);
    interp.register_property(s, "unicodeScalars", unicode_scalars_view);

    // --- Non-mutating methods ---
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
    pure("split", split);
    pure("makeIterator", make_iterator);
    // A `String`/`Substring` has no contiguous storage over its `Character`
    // elements (its backing store is UTF-8 bytes), so this always returns nil
    // without invoking the closure — matching Swift's default `Collection`
    // behavior.
    pure(
        "withContiguousStorageIfAvailable",
        with_contiguous_storage_if_available,
    );

    // --- Mutating append ---
    interp.register_intrinsic(
        s,
        "append",
        MethodEntry {
            mutating: true,
            func: append,
        },
    );
}

/// `withContiguousStorageIfAvailable(_:)` — a `String`/`Substring` is not
/// contiguous over its `Character` elements, so this returns `nil` without
/// calling `body`, matching Swift's default `Collection` conformance.
fn with_contiguous_storage_if_available(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    str_of(&recv)?;
    Ok(Outcome {
        result: SwiftValue::Nil,
        receiver: recv,
    })
}

/// Segment a string into extended grapheme clusters (Swift `Character`s).
///
/// Re-exported from `tswift-core` so the interpreter's string iteration and
/// these `String` intrinsics segment identically.
pub use tswift_core::graphemes;

/// Extract the text from a `String` or `Substring` receiver.
///
/// For a `Substring`, this materialises the grapheme-cluster slice into an
/// owned `String`.  Shared by methods registered under both
/// `BuiltinReceiver::String` and `BuiltinReceiver::Substring` (e.g.
/// `lowercased`, `hasPrefix`, `split`, …) so they work on both types.
pub(super) fn str_of(recv: &SwiftValue) -> Result<String, StdError> {
    match recv {
        SwiftValue::Str(s) => Ok(s.clone()),
        SwiftValue::Substring { base, start, end } => Ok(graphemes(base)[*start..*end].concat()),
        other => Err(type_err(format!(
            "expected String or Substring, got {}",
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

/// `String.makeIterator()` / `Substring.makeIterator()` — for-in over a string
/// is driven by the interpreter's grapheme iteration, but an explicit
/// `.makeIterator()` returns the `Character`s as an iterable array of
/// single-grapheme strings.
fn make_iterator(_c: &mut dyn StdContext, recv: SwiftValue, _a: Vec<SwiftValue>) -> Outcomes {
    let items: Vec<SwiftValue> = graphemes(&str_of(&recv)?)
        .into_iter()
        .map(SwiftValue::Str)
        .collect();
    val(SwiftValue::Array(Rc::new(items)), recv)
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
    classify_lone(recv, |c| c.is_ascii_digit())
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
    Ok(SwiftValue::Bool(!s.is_empty() && s.is_ascii()))
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

/// `String.append(_:)` / `Substring.append(_:)` — append a character or string
/// in place.
///
/// When the receiver is a `Substring`, Swift's copy-on-write semantics detach
/// the slice from its base on the first mutation: the result is an independent
/// `Substring` with `start = 0` and a fresh backing string.  A `String`
/// receiver is updated in place as before.
fn append(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let was_substring = matches!(recv, SwiftValue::Substring { .. });
    let mut s = str_of(&recv)?;
    if let Some(extra) = arg_str(&args) {
        s.push_str(&extra);
    }
    let receiver = if was_substring {
        let n = graphemes(&s).len();
        SwiftValue::Substring {
            base: Rc::new(s),
            start: 0,
            end: n,
        }
    } else {
        SwiftValue::Str(s)
    };
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver,
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

/// `String.utf8` — the UTF-8 code units as an array of `UInt8` values. The
/// view's lazy semantics are not modelled; the materialised array supports
/// `count`, iteration, and `Array(_:)` conversion, which is the common surface.
fn utf8_view(recv: SwiftValue) -> StdResult {
    let bytes = str_of(&recv)?
        .bytes()
        .map(|b| SwiftValue::Int(IntValue::new(i128::from(b), IntWidth::U8)))
        .collect();
    Ok(SwiftValue::Array(Rc::new(bytes)))
}

/// `String.utf16` — the UTF-16 code units as an array of `UInt16` values.
fn utf16_view(recv: SwiftValue) -> StdResult {
    let units = str_of(&recv)?
        .encode_utf16()
        .map(|u| SwiftValue::Int(IntValue::new(i128::from(u), IntWidth::U16)))
        .collect();
    Ok(SwiftValue::Array(Rc::new(units)))
}

/// `String.unicodeScalars` — the Unicode scalar values as an array of `UInt32`
/// (each scalar modelled by its numeric `.value`).
fn unicode_scalars_view(recv: SwiftValue) -> StdResult {
    let scalars = str_of(&recv)?
        .chars()
        .map(|c| SwiftValue::Int(IntValue::new(i128::from(c as u32), IntWidth::U32)))
        .collect();
    Ok(SwiftValue::Array(Rc::new(scalars)))
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

/// `String.characters` — the `Character` view; a String IS its own character
/// collection, so this returns self (deprecated in Swift 4+ but still valid).
fn characters(recv: SwiftValue) -> StdResult {
    let _ = str_of(&recv)?; // validate receiver type
    Ok(recv)
}

/// `String.isContiguousUTF8` — always `true` (runtime stores contiguous UTF-8).
fn is_contiguous_utf8(recv: SwiftValue) -> StdResult {
    let _ = str_of(&recv)?;
    Ok(SwiftValue::Bool(true))
}

/// `String.makeContiguousUTF8()` — no-op mutating (runtime is always contiguous).
fn make_contiguous_utf8(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    _a: Vec<SwiftValue>,
) -> Outcomes {
    let s = str_of(&recv)?;
    val(SwiftValue::Void, SwiftValue::Str(s))
}

// ---- String.Index ----------------------------------------------------------

/// Construct an opaque `String.Index` value from a grapheme-cluster offset.
///
/// The index is stored as a `Struct` so it is not accidentally confused with an
/// integer and so that binary operators (`==`, `..<`, …) in ops.rs can
/// recognize it by type name and apply correct semantics.
pub(super) fn make_index(offset: usize) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "String.Index".into(),
        fields: vec![("_offset".into(), SwiftValue::int(offset as i128))],
    }))
}

/// Extract the grapheme-cluster offset from a `String.Index` value.
pub(super) fn index_offset(v: &SwiftValue) -> Option<usize> {
    match v {
        SwiftValue::Struct(obj) if obj.type_name == "String.Index" => {
            obj.get("_offset").and_then(|f| match f {
                SwiftValue::Int(i) if i.raw >= 0 => Some(i.raw as usize),
                _ => None,
            })
        }
        _ => None,
    }
}

/// `String.startIndex` — the `String.Index` of the first grapheme cluster.
fn start_index(_recv: SwiftValue) -> StdResult {
    Ok(make_index(0))
}

/// `String.endIndex` — the `String.Index` one past the last grapheme cluster.
fn end_index(recv: SwiftValue) -> StdResult {
    let count = graphemes(&str_of(&recv)?).len();
    Ok(make_index(count))
}

/// `String.indices` — the collection of valid subscript positions
/// (`startIndex..<endIndex`), materialised as an array of `String.Index`
/// values so `for i in s.indices` and `Array(s.indices)` both work.
fn indices(recv: SwiftValue) -> StdResult {
    let count = graphemes(&str_of(&recv)?).len();
    let items: Vec<SwiftValue> = (0..count).map(make_index).collect();
    Ok(SwiftValue::Array(Rc::new(items)))
}

/// `String.index` — label-aware dispatch over four overloads:
/// - `index(after:)` — advance by one grapheme cluster
/// - `index(before:)` — retreat by one grapheme cluster
/// - `index(_:offsetBy:)` — advance by `n` grapheme clusters (traps OOB)
/// - `index(_:offsetBy:limitedBy:)` — advance clamped to a limit, or `nil`
/// `String.formIndex(...)` — intercepted by the dispatcher for inout write-back;
/// delegates to `index` and serves only as a registered fallback.
fn form_index_labeled(
    c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    index_labeled(c, recv, args)
}

fn index_labeled(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let count = graphemes(&str_of(&recv)?).len();
    let trap_oob = |what: &str, offset: usize| -> StdError {
        StdError::Error(EvalError::Trap(format!(
            "String.{what}: index {offset} out of range [0, {count}]"
        )))
    };

    // index(after: i) — one arg labeled "after"
    if let Some(arg) = args.iter().find(|a| a.label.as_deref() == Some("after")) {
        let offset = index_offset(&arg.value)
            .ok_or_else(|| type_err("index(after:) expects a String.Index".into()))?;
        if offset >= count {
            return Err(trap_oob("index(after:)", offset));
        }
        return Ok(Some(Outcome {
            result: make_index(offset + 1),
            receiver: recv,
        }));
    }

    // index(before: i) — one arg labeled "before"
    if let Some(arg) = args.iter().find(|a| a.label.as_deref() == Some("before")) {
        let offset = index_offset(&arg.value)
            .ok_or_else(|| type_err("index(before:) expects a String.Index".into()))?;
        if offset > count {
            return Err(trap_oob("index(before:)", offset));
        }
        if offset == 0 {
            return Err(StdError::Error(EvalError::Trap(
                "String.index(before:): cannot retreat before startIndex".into(),
            )));
        }
        return Ok(Some(Outcome {
            result: make_index(offset - 1),
            receiver: recv,
        }));
    }

    // index(_:offsetBy:) or index(_:offsetBy:limitedBy:)
    let offset_arg = args.iter().find(|a| a.label.as_deref() == Some("offsetBy"));
    if let Some(off_arg) = offset_arg {
        // The base index is the first positional argument.
        let base_idx = args
            .iter()
            .find(|a| a.label.is_none())
            .ok_or_else(|| type_err("index(_:offsetBy:) expects a base String.Index".into()))?;
        let base = index_offset(&base_idx.value)
            .ok_or_else(|| type_err("index(_:offsetBy:) base must be a String.Index".into()))?;
        // Trap if the base index itself is past endIndex.
        if base > count {
            return Err(trap_oob("index(_:offsetBy:) base", base));
        }
        let n = match &off_arg.value {
            SwiftValue::Int(i) => i.raw,
            _ => return Err(type_err("index(_:offsetBy:) offset must be an Int".into())),
        };
        let new_offset = base as i128 + n;

        // index(_:offsetBy:limitedBy:)
        if let Some(limit_arg) = args
            .iter()
            .find(|a| a.label.as_deref() == Some("limitedBy"))
        {
            let limit = index_offset(&limit_arg.value).ok_or_else(|| {
                type_err("index(_:offsetBy:limitedBy:) limit must be a String.Index".into())
            })?;
            let limit = limit as i128;
            // Passed the limit? Return nil.
            // n == 0 means "don't move" so the limit never applies.
            let passed = (n > 0 && new_offset > limit) || (n < 0 && new_offset < limit);
            if passed {
                return Ok(Some(Outcome {
                    result: SwiftValue::Nil,
                    receiver: recv,
                }));
            }
            if new_offset < 0 || new_offset as usize > count {
                return Err(trap_oob(
                    "index(_:offsetBy:limitedBy:)",
                    new_offset as usize,
                ));
            }
            return Ok(Some(Outcome {
                result: make_index(new_offset as usize),
                receiver: recv,
            }));
        }

        // Plain index(_:offsetBy:)
        if new_offset < 0 || new_offset as usize > count {
            return Err(trap_oob(
                "index(_:offsetBy:)",
                new_offset.unsigned_abs() as usize,
            ));
        }
        return Ok(Some(Outcome {
            result: make_index(new_offset as usize),
            receiver: recv,
        }));
    }

    // No label matched — fall through to the plain positional intrinsic (none
    // registered for "index" on String, so this returns None and the call fails).
    Ok(None)
}

/// `String.distance(from:to:)` — number of grapheme clusters between two indices.
fn distance_labeled(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let from_arg = args.iter().find(|a| a.label.as_deref() == Some("from"));
    let to_arg = args.iter().find(|a| a.label.as_deref() == Some("to"));
    let (Some(from_arg), Some(to_arg)) = (from_arg, to_arg) else {
        return Ok(None); // Fall through if labels don't match.
    };
    let count = graphemes(&str_of(&recv)?).len();
    let from = index_offset(&from_arg.value)
        .ok_or_else(|| type_err("distance(from:to:) expects String.Index arguments".into()))?;
    let to = index_offset(&to_arg.value)
        .ok_or_else(|| type_err("distance(from:to:) expects String.Index arguments".into()))?;
    // Both indices must be within [0, count]; past-endIndex indices trap.
    if from > count {
        return Err(StdError::Error(EvalError::Trap(format!(
            "String.distance(from:to:): 'from' index {from} out of range [0, {count}]"
        ))));
    }
    if to > count {
        return Err(StdError::Error(EvalError::Trap(format!(
            "String.distance(from:to:): 'to' index {to} out of range [0, {count}]"
        ))));
    }
    Ok(Some(Outcome {
        result: SwiftValue::int(to as i128 - from as i128),
        receiver: recv,
    }))
}

/// `String.insert(_:at:)` and `String.insert(contentsOf:at:)` — label-aware.
fn insert_labeled(
    _c: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let at_arg = args.iter().find(|a| a.label.as_deref() == Some("at"));
    let Some(at_arg) = at_arg else {
        return Ok(None);
    };
    let offset = index_offset(&at_arg.value)
        .ok_or_else(|| type_err("insert(at:) expects a String.Index".into()))?;
    let mut grapheme_vec = graphemes(&str_of(&recv)?);
    let count = grapheme_vec.len();
    if offset > count {
        return Err(StdError::Error(EvalError::Trap(format!(
            "String.insert: index {offset} out of range [0, {count}]"
        ))));
    }

    // insert(contentsOf:at:) — the first arg is labeled "contentsOf"
    if let Some(contents_arg) = args
        .iter()
        .find(|a| a.label.as_deref() == Some("contentsOf"))
    {
        let extra = str_of(&contents_arg.value)
            .map_err(|_| type_err("insert(contentsOf:at:) expects a String".into()))?;
        let extra_graphemes = graphemes(&extra);
        for (i, g) in extra_graphemes.into_iter().enumerate() {
            grapheme_vec.insert(offset + i, g);
        }
        let new_str = SwiftValue::Str(grapheme_vec.concat());
        return Ok(Some(Outcome {
            result: SwiftValue::Void,
            receiver: new_str,
        }));
    }

    // insert(_:at:) — the first positional arg is the Character
    let char_arg = args
        .iter()
        .find(|a| a.label.is_none())
        .ok_or_else(|| type_err("insert(_:at:) expects a Character".into()))?;
    let ch = str_of(&char_arg.value)
        .map_err(|_| type_err("insert(_:at:) expects a Character".into()))?;
    // Swift's `Character` is exactly one grapheme cluster; trap otherwise.
    let ch_clusters = graphemes(&ch);
    if ch_clusters.len() != 1 {
        return Err(StdError::Error(EvalError::Trap(format!(
            "insert(_:at:): argument must be exactly one Character (grapheme cluster), \
             got {} clusters",
            ch_clusters.len()
        ))));
    }
    grapheme_vec.insert(offset, ch_clusters.into_iter().next().unwrap());
    let new_str = SwiftValue::Str(grapheme_vec.concat());
    Ok(Some(Outcome {
        result: SwiftValue::Void,
        receiver: new_str,
    }))
}

/// `String.remove(at:)` — remove and return the grapheme cluster at `index`.
fn remove_at(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let at = args
        .iter()
        .find_map(index_offset)
        .ok_or_else(|| type_err("remove(at:) expects a String.Index argument".into()))?;
    let mut grapheme_vec = graphemes(&str_of(&recv)?);
    let count = grapheme_vec.len();
    if at >= count {
        return Err(StdError::Error(EvalError::Trap(format!(
            "String.remove(at:): index {at} out of range [0, {count})"
        ))));
    }
    let removed = grapheme_vec.remove(at);
    Ok(Outcome {
        result: SwiftValue::Str(removed),
        receiver: SwiftValue::Str(grapheme_vec.concat()),
    })
}

/// `String.removeSubrange(_:)` — remove the grapheme clusters in a range.
fn remove_subrange(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let range = args
        .iter()
        .find(|a| matches!(a, SwiftValue::Range { .. }))
        .ok_or_else(|| type_err("removeSubrange expects a Range".into()))?;
    let mut grapheme_vec = graphemes(&str_of(&recv)?);
    let count = grapheme_vec.len();
    let (start, end) = tswift_core::collection_range_bounds(range, count, "removeSubrange")
        .map_err(StdError::Error)?;
    grapheme_vec.drain(start..end);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Str(grapheme_vec.concat()),
    })
}

/// `String.replaceSubrange(_:with:)` — replace a range with another string.
fn replace_subrange(_c: &mut dyn StdContext, recv: SwiftValue, args: Vec<SwiftValue>) -> Outcomes {
    let range = args
        .iter()
        .find(|a| matches!(a, SwiftValue::Range { .. }))
        .ok_or_else(|| type_err("replaceSubrange expects a Range".into()))?;
    let replacement = args
        .iter()
        .find_map(|a| match a {
            SwiftValue::Str(s) => Some(s.clone()),
            _ => None,
        })
        .ok_or_else(|| type_err("replaceSubrange(_:with:) expects a String replacement".into()))?;
    let mut grapheme_vec = graphemes(&str_of(&recv)?);
    let count = grapheme_vec.len();
    let (start, end) = tswift_core::collection_range_bounds(range, count, "replaceSubrange")
        .map_err(StdError::Error)?;
    let replacement_graphemes = graphemes(&replacement);
    grapheme_vec.splice(start..end, replacement_graphemes);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Str(grapheme_vec.concat()),
    })
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
    fn unicode_views() {
        // UTF-8 of "é" is two bytes (0xC3 0xA9); ASCII "AB" is [65, 66] as UInt8.
        let raws = |v: SwiftValue| match v {
            SwiftValue::Array(a) => a
                .iter()
                .map(|e| match e {
                    SwiftValue::Int(i) => i.raw,
                    _ => -1,
                })
                .collect::<Vec<_>>(),
            _ => panic!("expected an array view"),
        };
        assert_eq!(raws(utf8_view(s("AB")).unwrap()), vec![65, 66]);
        let accented = utf8_view(s("é")).unwrap();
        assert!(matches!(accented, SwiftValue::Array(a) if a.len() == 2));
        let units = utf16_view(s("AB")).unwrap();
        assert!(matches!(units, SwiftValue::Array(a) if a.len() == 2));
        let scalars = unicode_scalars_view(s("AB")).unwrap();
        assert!(matches!(scalars, SwiftValue::Array(a) if a.len() == 2));
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

    // ---- String.Index helpers -----------------------------------------------

    fn idx(offset: usize) -> SwiftValue {
        make_index(offset)
    }

    fn arg_labeled(label: &str, value: SwiftValue) -> Arg {
        Arg {
            label: Some(label.to_string()),
            value,

            static_ty: None,
        }
    }

    fn arg_pos(value: SwiftValue) -> Arg {
        Arg {
            label: None,
            value,
            static_ty: None,
        }
    }

    fn is_trap(e: &StdError) -> bool {
        matches!(e, StdError::Error(EvalError::Trap(_)))
    }

    #[test]
    fn string_index_properties() {
        assert_eq!(start_index(s("hello")).unwrap(), idx(0));
        assert_eq!(end_index(s("hello")).unwrap(), idx(5));
        assert_eq!(end_index(s("")).unwrap(), idx(0));
        // Multi-byte: "café" has 4 grapheme clusters.
        assert_eq!(end_index(s("cafe\u{301}")).unwrap(), idx(4));
    }

    #[test]
    fn index_after_steps_one_cluster() {
        let mut m = M;
        // index(after: startIndex) on "hello" → offset 1
        let result = index_labeled(&mut m, s("hello"), vec![arg_labeled("after", idx(0))])
            .unwrap()
            .unwrap()
            .result;
        assert_eq!(result, idx(1));
        // Family emoji counts as one cluster — after startIndex = endIndex.
        let fam = "👨\u{200D}👩\u{200D}👧\u{200D}👦";
        let result = index_labeled(&mut m, s(fam), vec![arg_labeled("after", idx(0))])
            .unwrap()
            .unwrap()
            .result;
        assert_eq!(result, idx(1)); // One cluster, so after(0) = 1 = endIndex.
    }

    #[test]
    fn index_after_past_end_traps() {
        let mut m = M;
        // "hello" has 5 clusters; index(after: endIndex=5) must trap.
        let err =
            index_labeled(&mut m, s("hello"), vec![arg_labeled("after", idx(5))]).unwrap_err();
        assert!(is_trap(&err), "expected trap, got {err:?}");
    }

    #[test]
    fn index_before_at_start_traps() {
        let mut m = M;
        let err =
            index_labeled(&mut m, s("hello"), vec![arg_labeled("before", idx(0))]).unwrap_err();
        assert!(is_trap(&err), "expected trap, got {err:?}");
    }

    #[test]
    fn index_before_past_end_traps() {
        let mut m = M;
        // index(before: 99) on a 5-cluster string must trap (base out of range).
        let err =
            index_labeled(&mut m, s("hello"), vec![arg_labeled("before", idx(99))]).unwrap_err();
        assert!(is_trap(&err), "expected trap, got {err:?}");
    }

    #[test]
    fn index_offset_by_base_oob_traps() {
        let mut m = M;
        // base index 99 is past endIndex of "hello" (count=5) → trap.
        let err = index_labeled(
            &mut m,
            s("hello"),
            vec![
                arg_pos(idx(99)),
                arg_labeled("offsetBy", SwiftValue::int(0)),
            ],
        )
        .unwrap_err();
        assert!(is_trap(&err), "expected trap, got {err:?}");
    }

    #[test]
    fn index_offset_by_result_oob_traps() {
        let mut m = M;
        // base=2 + offsetBy=10 → result 12 > 5 → trap.
        let err = index_labeled(
            &mut m,
            s("hello"),
            vec![
                arg_pos(idx(2)),
                arg_labeled("offsetBy", SwiftValue::int(10)),
            ],
        )
        .unwrap_err();
        assert!(is_trap(&err), "expected trap, got {err:?}");
    }

    #[test]
    fn index_offset_by_negative_oob_traps() {
        let mut m = M;
        // base=1 + offsetBy=-5 → result -4 → trap.
        let err = index_labeled(
            &mut m,
            s("hello"),
            vec![
                arg_pos(idx(1)),
                arg_labeled("offsetBy", SwiftValue::int(-5)),
            ],
        )
        .unwrap_err();
        assert!(is_trap(&err), "expected trap, got {err:?}");
    }

    #[test]
    fn distance_oob_traps() {
        let mut m = M;
        // 'from' index 99 is past endIndex of "hello" → trap.
        let err = distance_labeled(
            &mut m,
            s("hello"),
            vec![arg_labeled("from", idx(99)), arg_labeled("to", idx(0))],
        )
        .unwrap_err();
        assert!(is_trap(&err), "expected trap, got {err:?}");
        // 'to' index past end also traps.
        let err2 = distance_labeled(
            &mut m,
            s("hello"),
            vec![arg_labeled("from", idx(0)), arg_labeled("to", idx(99))],
        )
        .unwrap_err();
        assert!(is_trap(&err2), "expected trap, got {err2:?}");
    }

    #[test]
    fn insert_character_not_exactly_one_cluster_traps() {
        let mut m = M;
        // Two-grapheme string "ab" is not a valid Character.
        let err = insert_labeled(
            &mut m,
            s("hello"),
            vec![arg_pos(s("ab")), arg_labeled("at", idx(0))],
        )
        .unwrap_err();
        assert!(is_trap(&err), "expected trap, got {err:?}");
        // Empty string is also not a valid Character.
        let err2 = insert_labeled(
            &mut m,
            s("hello"),
            vec![arg_pos(s("")), arg_labeled("at", idx(0))],
        )
        .unwrap_err();
        assert!(is_trap(&err2), "expected trap, got {err2:?}");
    }

    #[test]
    fn replace_subrange_requires_string_replacement() {
        let mut m = M;
        // Passing an Int instead of a String replacement must error.
        let err = replace_subrange(
            &mut m,
            s("hello"),
            vec![
                SwiftValue::Range {
                    lo: 0,
                    hi: 2,
                    inclusive: false,
                },
                SwiftValue::int(42),
            ],
        )
        .unwrap_err();
        assert!(
            matches!(err, StdError::Error(EvalError::Type(_))),
            "expected type error, got {err:?}"
        );
    }
}
