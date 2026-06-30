//! `Decimal` — exact base-10 arithmetic over a `mantissa * 10^exponent`
//! representation.
//!
//! The value type lives in `tswift-foundation`, but its operator semantics must
//! be reachable from [`crate::ops`] (operator dispatch is a core concern, like
//! `Date`). This module owns the math; both crates bridge through the shared
//! [`SwiftValue::Struct`] representation: `type_name == "Decimal"` with fields
//! `_mantissa: Int`, `_exponent: Int`, and `_nan: Bool`.
//!
//! Representation is `i128` mantissa + `i32` exponent. This covers the common
//! daily range but is narrower than Darwin's 38-digit `Decimal`; the gap is
//! documented in `frameworks/foundation/scope.toml`.

use std::rc::Rc;

use crate::value::{StructObj, SwiftValue};

/// Maximum fractional guard digits produced by division.
const DIV_PRECISION: u32 = 30;

/// A decimal number: `mantissa * 10^exponent`, or NaN.
#[derive(Clone, Copy, Debug)]
pub struct Dec {
    pub nan: bool,
    pub mantissa: i128,
    pub exponent: i32,
}

impl Dec {
    pub const NAN: Dec = Dec {
        nan: true,
        mantissa: 0,
        exponent: 0,
    };

    pub fn new(mantissa: i128, exponent: i32) -> Dec {
        Dec {
            nan: false,
            mantissa,
            exponent,
        }
        .normalized()
    }

    pub fn zero() -> Dec {
        Dec {
            nan: false,
            mantissa: 0,
            exponent: 0,
        }
    }

    pub fn is_zero(&self) -> bool {
        !self.nan && self.mantissa == 0
    }

    /// Strip trailing decimal zeros (`1500 * 10^-2` → `15 * 10^0`), keeping a
    /// canonical form. Zero normalizes to exponent 0.
    fn normalized(mut self) -> Dec {
        if self.nan {
            return Dec::NAN;
        }
        if self.mantissa == 0 {
            self.exponent = 0;
            return self;
        }
        while self.mantissa % 10 == 0 {
            self.mantissa /= 10;
            self.exponent += 1;
        }
        self
    }

    pub fn magnitude(&self) -> Dec {
        if self.nan {
            return Dec::NAN;
        }
        Dec {
            nan: false,
            mantissa: self.mantissa.abs(),
            exponent: self.exponent,
        }
    }

    pub fn negated(&self) -> Dec {
        if self.nan {
            return Dec::NAN;
        }
        Dec {
            nan: false,
            mantissa: -self.mantissa,
            exponent: self.exponent,
        }
    }
}

/// Scale a mantissa up by `10^n`, returning `None` on `i128` overflow so callers
/// can surface NaN rather than panicking (debug) or wrapping (release).
fn scale_up(mantissa: i128, n: u32) -> Option<i128> {
    10_i128.checked_pow(n).and_then(|f| mantissa.checked_mul(f))
}

/// Re-express two decimals at a common (the lower) exponent. `None` on overflow.
fn align(a: Dec, b: Dec) -> Option<(i128, i128, i32)> {
    let exponent = a.exponent.min(b.exponent);
    let am = scale_up(a.mantissa, (a.exponent - exponent) as u32)?;
    let bm = scale_up(b.mantissa, (b.exponent - exponent) as u32)?;
    Some((am, bm, exponent))
}

pub fn add(a: Dec, b: Dec) -> Dec {
    if a.nan || b.nan {
        return Dec::NAN;
    }
    let Some((am, bm, exponent)) = align(a, b) else {
        return Dec::NAN;
    };
    match am.checked_add(bm) {
        Some(sum) => Dec::new(sum, exponent),
        None => Dec::NAN,
    }
}

pub fn sub(a: Dec, b: Dec) -> Dec {
    add(a, b.negated())
}

pub fn mul(a: Dec, b: Dec) -> Dec {
    if a.nan || b.nan {
        return Dec::NAN;
    }
    match (
        a.mantissa.checked_mul(b.mantissa),
        a.exponent.checked_add(b.exponent),
    ) {
        (Some(mantissa), Some(exponent)) => Dec::new(mantissa, exponent),
        _ => Dec::NAN,
    }
}

pub fn div(a: Dec, b: Dec) -> Dec {
    if a.nan || b.nan || b.mantissa == 0 {
        return Dec::NAN;
    }
    if a.mantissa == 0 {
        return Dec::zero();
    }
    let negative = (a.mantissa < 0) ^ (b.mantissa < 0);
    let num = a.mantissa.unsigned_abs();
    let den = b.mantissa.unsigned_abs();
    let Some(mut exponent) = a.exponent.checked_sub(b.exponent) else {
        return Dec::NAN;
    };

    let mut result: i128 = (num / den) as i128;
    let mut rem = num % den;
    let mut produced = 0;
    while rem != 0 && produced < DIV_PRECISION {
        let Some(scaled) = rem.checked_mul(10) else {
            break;
        };
        rem = scaled;
        let (Some(next), Some(next_exp)) = (
            result
                .checked_mul(10)
                .and_then(|r| r.checked_add((rem / den) as i128)),
            exponent.checked_sub(1),
        ) else {
            break;
        };
        exponent = next_exp;
        result = next;
        rem %= den;
        produced += 1;
    }
    // Round half-up on the first dropped digit, if any remains.
    if rem != 0 && (rem * 2) >= den {
        match result.checked_add(1) {
            Some(rounded) => result = rounded,
            None => return Dec::NAN,
        }
    }
    let signed = if negative { -result } else { result };
    Dec::new(signed, exponent)
}

/// Total ordering for comparison operators (NaN handling is the caller's
/// responsibility). Falls back to an `f64` comparison if exponent alignment
/// would overflow `i128`.
pub fn compare(a: Dec, b: Dec) -> std::cmp::Ordering {
    if let Some((am, bm, _)) = align(a, b) {
        return am.cmp(&bm);
    }
    let af = a.mantissa as f64 * 10_f64.powi(a.exponent);
    let bf = b.mantissa as f64 * 10_f64.powi(b.exponent);
    af.partial_cmp(&bf).unwrap_or(std::cmp::Ordering::Equal)
}

/// Rounding modes mirroring `Decimal.RoundingMode`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RoundingMode {
    Plain,
    Down,
    Up,
    Bankers,
}

impl RoundingMode {
    pub fn from_name(name: &str) -> Option<RoundingMode> {
        Some(match name {
            "plain" => RoundingMode::Plain,
            "down" => RoundingMode::Down,
            "up" => RoundingMode::Up,
            "bankers" => RoundingMode::Bankers,
            _ => return None,
        })
    }
}

/// Round to `scale` fractional digits (target exponent `-scale`).
pub fn rounded(value: Dec, scale: i32, mode: RoundingMode) -> Dec {
    if value.nan {
        return Dec::NAN;
    }
    let target = -scale;
    if value.exponent >= target {
        // Already coarser than the target; no rounding needed.
        return value;
    }
    let drop = (target - value.exponent) as u32;
    let Some(divisor) = 10_i128.checked_pow(drop) else {
        // Dropping more digits than the value has: it rounds to zero.
        return Dec::zero();
    };
    let negative = value.mantissa < 0;
    let abs = value.mantissa.unsigned_abs();
    let divisor_u = divisor as u128;
    let mut quotient = (abs / divisor_u) as i128;
    let remainder = abs % divisor_u;
    if remainder != 0 {
        let twice = remainder * 2;
        let round_up = match mode {
            RoundingMode::Down => false,
            RoundingMode::Up => true,
            RoundingMode::Plain => twice >= divisor_u,
            RoundingMode::Bankers => twice > divisor_u || (twice == divisor_u && quotient % 2 == 1),
        };
        if round_up {
            quotient += 1;
        }
    }
    let signed = if negative { -quotient } else { quotient };
    Dec::new(signed, target)
}

/// Canonical decimal string (`"15"`, `"0.3"`, `"-12.5"`).
pub fn to_string(value: Dec) -> String {
    if value.nan {
        return "NaN".to_string();
    }
    if value.mantissa == 0 {
        return "0".to_string();
    }
    let negative = value.mantissa < 0;
    let digits = value.mantissa.unsigned_abs().to_string();
    let mut body = if value.exponent >= 0 {
        // Append trailing zeros.
        let mut s = digits;
        s.push_str(&"0".repeat(value.exponent as usize));
        s
    } else {
        let frac = (-value.exponent) as usize;
        if digits.len() > frac {
            let point = digits.len() - frac;
            format!("{}.{}", &digits[..point], &digits[point..])
        } else {
            let zeros = "0".repeat(frac - digits.len());
            format!("0.{zeros}{digits}")
        }
    };
    if negative {
        body.insert(0, '-');
    }
    body
}

/// Parse a decimal string (`"-12.50"`, `"1e3"` is *not* supported). Returns
/// `None` on malformed input.
pub fn parse(input: &str) -> Option<Dec> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (negative, rest) = match trimmed.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let mut mantissa: i128 = 0;
    let mut exponent: i32 = 0;
    let mut seen_digit = false;
    let mut seen_point = false;
    for ch in rest.chars() {
        match ch {
            '0'..='9' => {
                mantissa = mantissa
                    .checked_mul(10)?
                    .checked_add((ch as u8 - b'0') as i128)?;
                seen_digit = true;
                if seen_point {
                    exponent -= 1;
                }
            }
            '.' if !seen_point => seen_point = true,
            _ => return None,
        }
    }
    if !seen_digit {
        return None;
    }
    if negative {
        mantissa = -mantissa;
    }
    Some(Dec::new(mantissa, exponent))
}

// ---------------------------------------------------------------------------
// SwiftValue bridging
// ---------------------------------------------------------------------------

pub fn to_value(value: Dec) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "Decimal".into(),
        fields: vec![
            ("_mantissa".into(), SwiftValue::int(value.mantissa)),
            ("_exponent".into(), SwiftValue::int(value.exponent as i128)),
            ("_nan".into(), SwiftValue::Bool(value.nan)),
        ],
    }))
}

/// Read a `Dec` out of a `Decimal` struct value, if it is one.
pub fn from_value(value: &SwiftValue) -> Option<Dec> {
    let SwiftValue::Struct(obj) = value else {
        return None;
    };
    if obj.type_name != "Decimal" {
        return None;
    }
    let mantissa = match obj.get("_mantissa") {
        Some(SwiftValue::Int(i)) => i.raw,
        _ => return None,
    };
    let exponent = match obj.get("_exponent") {
        Some(SwiftValue::Int(i)) => i.raw as i32,
        _ => return None,
    };
    let nan = matches!(obj.get("_nan"), Some(SwiftValue::Bool(true)));
    Some(Dec {
        nan,
        mantissa,
        exponent,
    })
}

/// Coerce an operand into a `Dec` for mixed Decimal arithmetic (`d + 1`).
pub fn coerce(value: &SwiftValue) -> Option<Dec> {
    match value {
        SwiftValue::Struct(_) => from_value(value),
        SwiftValue::Int(i) => Some(Dec::new(i.raw, 0)),
        // Use the shortest round-tripping decimal string of the double.
        SwiftValue::Double(d) => parse(&format!("{d}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(s: &str) -> Dec {
        parse(s).unwrap()
    }

    #[test]
    fn exact_addition_avoids_binary_error() {
        let sum = add(dec("0.1"), dec("0.2"));
        assert_eq!(to_string(sum), "0.3");
        assert_eq!(compare(sum, dec("0.3")), std::cmp::Ordering::Equal);
    }

    #[test]
    fn multiplication_and_division() {
        assert_eq!(to_string(mul(dec("1.5"), dec("2"))), "3");
        assert_eq!(to_string(div(dec("1"), dec("8"))), "0.125");
        assert_eq!(to_string(div(dec("10"), dec("4"))), "2.5");
    }

    #[test]
    fn rounding_modes() {
        assert_eq!(to_string(rounded(dec("2.5"), 0, RoundingMode::Plain)), "3");
        assert_eq!(to_string(rounded(dec("2.5"), 0, RoundingMode::Down)), "2");
        assert_eq!(to_string(rounded(dec("2.4"), 0, RoundingMode::Up)), "3");
        assert_eq!(
            to_string(rounded(dec("2.5"), 0, RoundingMode::Bankers)),
            "2"
        );
        assert_eq!(
            to_string(rounded(dec("3.5"), 0, RoundingMode::Bankers)),
            "4"
        );
        assert_eq!(
            to_string(rounded(dec("1.2345"), 2, RoundingMode::Plain)),
            "1.23"
        );
    }

    #[test]
    fn overflow_and_nan_are_contained() {
        // Aligning exponents beyond i128 yields NaN, not a panic/wrap.
        let huge = add(
            dec("1"),
            parse("0.000000000000000000000000000000000000001").unwrap(),
        );
        assert!(huge.nan);
        // Division by zero is NaN, and NaN compares unequal to itself.
        let nan = div(dec("1"), dec("0"));
        assert!(nan.nan);
        assert_ne!(compare(nan, nan), std::cmp::Ordering::Greater); // no panic
    }

    #[test]
    fn string_round_trip_and_negatives() {
        assert_eq!(to_string(dec("-12.50")), "-12.5");
        assert_eq!(to_string(dec("0.000")), "0");
        assert!(parse("1.2.3").is_none());
        assert!(parse("abc").is_none());
    }
}
