//! Decoders from the parse AST's textual/keyword payloads into the
//! runtime-facing forms the interpreter reads: the modifier bitmask and the
//! numeric literal values.

/// Translate parser modifier keywords into the runtime-facing modifier bitmask
/// (the same bit layout `tswift-core` reads and `modifier_names` decodes).
pub(crate) fn modifier_bits(modifiers: &[String]) -> u32 {
    let mut bits = 0u32;
    for m in modifiers {
        bits |= match m.as_str() {
            "public" => 1 << 0,
            "private" => 1 << 1,
            "internal" => 1 << 2,
            "fileprivate" => 1 << 3,
            "open" => 1 << 4,
            // `static` and a type-level `class` member both mean "static".
            "static" | "class" => 1 << 5,
            "final" => 1 << 6,
            "override" => 1 << 7,
            "mutating" => 1 << 8,
            "nonmutating" => 1 << 9,
            "lazy" => 1 << 10,
            "weak" => 1 << 11,
            "unowned" => 1 << 12,
            "async" => 1 << 13,
            "throws" => 1 << 14,
            "rethrows" => 1 << 15,
            "indirect" => 1 << 16,
            "required" => 1 << 17,
            "convenience" => 1 << 18,
            "dynamic" => 1 << 19,
            // Parameter type attributes carried for the runtime.
            "escaping" => 1 << 26,
            "autoclosure" => 1 << 27,
            // Parameter flags: `T...` variadic and `inout`. The runtime reads
            // variadic via the same 1<<28 bit; inout uses a frontend-internal
            // bit surfaced through `param_info`.
            "variadic" => 1 << 28,
            "inout" => 1 << 30,
            _ => 0,
        };
    }
    bits
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
