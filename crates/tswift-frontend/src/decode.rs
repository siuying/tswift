//! Decoders from the parse AST's textual/keyword payloads into the
//! runtime-facing forms the interpreter reads: the modifier bitmask and the
//! numeric literal values.

// --- Modifier bit layout: the single source of truth ---
//
// Each Swift modifier keyword occupies one bit of the `u32` mask that crosses
// the frontend→runtime seam. These named constants are the *only* place the bit
// positions are written: `modifier_bits` (encode) and `flag_names` (decode)
// derive from them, and `Node`'s modifier predicates test them by concept so
// the runtime never sees a raw bit.
pub(crate) const PUBLIC: u32 = 1 << 0;
pub(crate) const PRIVATE: u32 = 1 << 1;
pub(crate) const INTERNAL: u32 = 1 << 2;
pub(crate) const FILEPRIVATE: u32 = 1 << 3;
pub(crate) const OPEN: u32 = 1 << 4;
pub(crate) const STATIC: u32 = 1 << 5;
pub(crate) const FINAL: u32 = 1 << 6;
pub(crate) const OVERRIDE: u32 = 1 << 7;
pub(crate) const MUTATING: u32 = 1 << 8;
pub(crate) const NONMUTATING: u32 = 1 << 9;
pub(crate) const LAZY: u32 = 1 << 10;
pub(crate) const WEAK: u32 = 1 << 11;
pub(crate) const UNOWNED: u32 = 1 << 12;
pub(crate) const ASYNC: u32 = 1 << 13;
pub(crate) const THROWS: u32 = 1 << 14;
pub(crate) const RETHROWS: u32 = 1 << 15;
pub(crate) const INDIRECT: u32 = 1 << 16;
pub(crate) const REQUIRED: u32 = 1 << 17;
pub(crate) const CONVENIENCE: u32 = 1 << 18;
pub(crate) const DYNAMIC: u32 = 1 << 19;
/// `@objc optional` protocol requirement.
pub(crate) const OPTIONAL_REQ: u32 = 1 << 20;
/// A `MemberExpr` reached through optional chaining (`base?.member`), as
/// opposed to a plain `base.member` access. Lets the runtime distinguish the
/// two so an `Optional`-owned member override applies only to plain access.
pub(crate) const OPTIONAL_CHAIN: u32 = 1 << 21;
pub(crate) const ESCAPING: u32 = 1 << 26;
pub(crate) const AUTOCLOSURE: u32 = 1 << 27;
pub(crate) const VARIADIC: u32 = 1 << 28;
pub(crate) const FAILABLE: u32 = 1 << 29;
pub(crate) const INOUT: u32 = 1 << 30;
/// `as?` optional cast. Deliberately reuses the `weak` bit: the two never
/// co-occur — `weak` appears only on a property decl, this only on a `CastExpr`
/// — so one bit serves both across disjoint node kinds.
pub(crate) const OPTIONAL_CAST: u32 = WEAK;

/// The decl-modifier bits paired with their Swift keyword, for decoding a mask
/// back into names (`Node::modifier_names`, the `tswift dump` format).
const FLAG_NAMES: &[(u32, &str)] = &[
    (PUBLIC, "public"),
    (PRIVATE, "private"),
    (INTERNAL, "internal"),
    (FILEPRIVATE, "fileprivate"),
    (OPEN, "open"),
    (STATIC, "static"),
    (FINAL, "final"),
    (OVERRIDE, "override"),
    (MUTATING, "mutating"),
    (NONMUTATING, "nonmutating"),
    (LAZY, "lazy"),
    (WEAK, "weak"),
    (UNOWNED, "unowned"),
    (ASYNC, "async"),
    (THROWS, "throws"),
    (RETHROWS, "rethrows"),
    (INDIRECT, "indirect"),
    (REQUIRED, "required"),
    (CONVENIENCE, "convenience"),
    (DYNAMIC, "dynamic"),
    (OPTIONAL_REQ, "optional"),
    (ESCAPING, "escaping"),
    (AUTOCLOSURE, "autoclosure"),
    (VARIADIC, "variadic"),
    (FAILABLE, "failable"),
];

/// Translate parser modifier keywords into the runtime-facing modifier bitmask.
pub(crate) fn modifier_bits(modifiers: &[String]) -> u32 {
    let mut bits = 0u32;
    for m in modifiers {
        bits |= match m.as_str() {
            "public" => PUBLIC,
            "private" => PRIVATE,
            "internal" => INTERNAL,
            "fileprivate" => FILEPRIVATE,
            "open" => OPEN,
            // `static` and a type-level `class` member both mean "static".
            "static" | "class" => STATIC,
            "final" => FINAL,
            "override" => OVERRIDE,
            "mutating" => MUTATING,
            "nonmutating" => NONMUTATING,
            "lazy" => LAZY,
            "weak" => WEAK,
            "unowned" => UNOWNED,
            "async" => ASYNC,
            "throws" => THROWS,
            "rethrows" => RETHROWS,
            "indirect" => INDIRECT,
            "required" => REQUIRED,
            "convenience" => CONVENIENCE,
            "dynamic" => DYNAMIC,
            "optional" => OPTIONAL_REQ,
            "?." => OPTIONAL_CHAIN,
            // Parameter type attributes carried for the runtime.
            "escaping" => ESCAPING,
            "autoclosure" => AUTOCLOSURE,
            // Parameter flags: `T...` variadic and `inout`.
            "variadic" => VARIADIC,
            "inout" => INOUT,
            _ => 0,
        };
    }
    bits
}

/// Decode a modifier bitmask into its set Swift keyword names, in bit order.
pub(crate) fn flag_names(bits: u32) -> Vec<&'static str> {
    FLAG_NAMES
        .iter()
        .filter(|(bit, _)| bits & bit != 0)
        .map(|(_, name)| *name)
        .collect()
}

/// Parse a Swift integer literal in any radix, honouring `_` digit separators
/// and a leading `-`.
pub(crate) fn parse_int_literal(text: &str) -> Option<i64> {
    let mut s = text.replace('_', "");
    let negative = s.starts_with('-');
    if negative {
        s.remove(0);
    }
    let (digits, radix) = if let Some(rest) = s.strip_prefix("0x") {
        (rest, 16)
    } else if let Some(rest) = s.strip_prefix("0b") {
        (rest, 2)
    } else if let Some(rest) = s.strip_prefix("0o") {
        (rest, 8)
    } else {
        (s.as_str(), 10)
    };
    let value = i64::from_str_radix(digits, radix).ok()?;
    Some(if negative { -value } else { value })
}

/// Parse a Swift floating-point literal, including the hexadecimal form
/// (`0x1.8p1`) that Rust's `str::parse::<f64>` rejects.
pub(crate) fn parse_float_literal(text: &str) -> Option<f64> {
    let s = text.replace('_', "");
    let body = s.strip_prefix('-').unwrap_or(&s);
    let value = if let Some(rest) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        parse_hex_float(rest)?
    } else {
        body.parse().ok()?
    };
    Some(if s.starts_with('-') { -value } else { value })
}

/// Parse the mantissa/exponent of a hex float (the part after `0x`): hexadecimal
/// `int[.frac]` scaled by a binary exponent `p[±]dec`, e.g. `1.8p1` → 3.0.
fn parse_hex_float(rest: &str) -> Option<f64> {
    let p = rest.find(['p', 'P'])?;
    let mantissa = &rest[..p];
    let exponent: i32 = rest[p + 1..].parse().ok()?;
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));

    let mut value = 0.0f64;
    for c in int_part.chars() {
        value = value * 16.0 + f64::from(c.to_digit(16)?);
    }
    let mut scale = 1.0 / 16.0;
    for c in frac_part.chars() {
        value += f64::from(c.to_digit(16)?) * scale;
        scale /= 16.0;
    }
    Some(value * 2f64.powi(exponent))
}

#[cfg(test)]
mod tests {
    use super::{parse_float_literal, parse_int_literal};

    #[test]
    fn integer_literals_in_every_radix() {
        assert_eq!(parse_int_literal("1_000"), Some(1000));
        assert_eq!(parse_int_literal("0xFF"), Some(255));
        assert_eq!(parse_int_literal("0o755"), Some(493));
        assert_eq!(parse_int_literal("0b1010"), Some(10));
        assert_eq!(parse_int_literal("-42"), Some(-42));
    }

    #[test]
    fn decimal_floats_parse() {
        assert_eq!(parse_float_literal("3.5"), Some(3.5));
        assert_eq!(parse_float_literal("1.5e3"), Some(1500.0));
        assert_eq!(parse_float_literal("1_000.5"), Some(1000.5));
    }

    #[test]
    fn hex_floats_parse() {
        assert_eq!(parse_float_literal("0x1.8p1"), Some(3.0));
        assert_eq!(parse_float_literal("0x1p4"), Some(16.0));
        assert_eq!(parse_float_literal("0xA.8p0"), Some(10.5));
        assert_eq!(parse_float_literal("-0x1.0p2"), Some(-4.0));
    }
}
