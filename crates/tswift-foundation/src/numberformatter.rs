//! `NumberFormatter` — number ⇄ string formatting (decimal, currency, percent).
//!
//! Fixed en_US conventions: `,` grouping, `.` decimal, `$` currency. Locale is
//! ignored; the gap is documented in `frameworks/foundation/scope.toml`. Like
//! `DateFormatter`, this ObjC class is absent from the generated
//! `.swiftinterface` inventory, so it is implemented and tested but does not
//! move the coverage roll-up.

use std::{cell::RefCell, rc::Rc};

use tswift_core::{
    Arg, BuiltinReceiver, ClassObj, Interpreter, IntrinsicFn, MethodEntry, Outcome, StdContext,
    StdError, StdResult, SwiftValue,
};

use crate::type_error;

pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_builtin_enum_with_raw(
        "NumberFormatter.Style",
        &[
            ("none", 0),
            ("decimal", 1),
            ("currency", 2),
            ("percent", 3),
            ("scientific", 4),
        ],
    );

    interp.register_free_fn("NumberFormatter", number_formatter_init);
    for prop in [
        "numberStyle",
        "minimumFractionDigits",
        "maximumFractionDigits",
        "groupingSeparator",
        "decimalSeparator",
        "currencySymbol",
    ] {
        interp.register_property(BuiltinReceiver::NumberFormatter, prop, prop_getter(prop));
    }
    // Effective getter: an unset (`Nil`) value resolves to the per-style default.
    interp.register_property(
        BuiltinReceiver::NumberFormatter,
        "usesGroupingSeparator",
        get_uses_grouping,
    );

    for (name, func) in [
        ("string", number_formatter_string as IntrinsicFn),
        ("number", number_formatter_number),
    ] {
        interp.register_intrinsic(
            BuiltinReceiver::NumberFormatter,
            name,
            MethodEntry {
                mutating: false,
                func,
            },
        );
    }
}

macro_rules! prop_getters {
    ($($field:literal => $getter:ident),+ $(,)?) => {
        $(
            fn $getter(recv: SwiftValue) -> StdResult {
                check_nf_recv(&recv)?;
                Ok(read_nf_field(&recv, $field).unwrap_or(SwiftValue::Nil))
            }
        )+
        fn prop_getter(name: &str) -> tswift_core::PropertyFn {
            match name {
                $($field => $getter,)+
                _ => unreachable!("unregistered NumberFormatter property {name}"),
            }
        }
    };
}

prop_getters! {
    "numberStyle" => get_number_style,
    "minimumFractionDigits" => get_min_fraction,
    "maximumFractionDigits" => get_max_fraction,
    "groupingSeparator" => get_grouping_separator,
    "decimalSeparator" => get_decimal_separator,
    "currencySymbol" => get_currency_symbol,
}

fn get_uses_grouping(recv: SwiftValue) -> StdResult {
    let value = match read_nf_field(&recv, "usesGroupingSeparator") {
        Some(SwiftValue::Bool(b)) => b,
        _ => {
            let (_, _, def_group) = style_defaults(number_style(&recv));
            def_group
        }
    };
    Ok(SwiftValue::Bool(value))
}

/// Construct a `NumberFormatter` Object (reference semantics — class in real Swift).
fn number_formatter_object() -> SwiftValue {
    SwiftValue::Object(Rc::new(RefCell::new(ClassObj {
        class_name: "NumberFormatter".into(),
        fields: vec![
            ("numberStyle".into(), SwiftValue::int(0)),
            // -1 sentinels: "unset", so per-style defaults apply.
            ("minimumFractionDigits".into(), SwiftValue::int(-1)),
            ("maximumFractionDigits".into(), SwiftValue::int(-1)),
            // `usesGroupingSeparator` is intentionally not stored here: leaving
            // it absent lets the effective getter (and formatting) fall back to
            // the per-style default until the user assigns a Bool.
            ("groupingSeparator".into(), SwiftValue::Nil),
            ("decimalSeparator".into(), SwiftValue::Nil),
            ("currencySymbol".into(), SwiftValue::Nil),
        ],
    })))
}

fn number_formatter_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if !args.is_empty() {
        return Err(type_error("NumberFormatter() takes no arguments"));
    }
    Ok(number_formatter_object())
}

/// Read a named field from either a `SwiftValue::Struct` or `SwiftValue::Object`
/// `NumberFormatter` receiver.
///
/// Returns `None` when the receiver is the wrong type or the field is absent.
fn read_nf_field(recv: &SwiftValue, field: &str) -> Option<SwiftValue> {
    match recv {
        SwiftValue::Struct(obj) if obj.type_name == "NumberFormatter" => obj.get(field).cloned(),
        SwiftValue::Object(o) if o.borrow().class_name == "NumberFormatter" => {
            o.borrow().get(field).cloned()
        }
        _ => None,
    }
}

/// Return `Err` if `recv` is not a `NumberFormatter` Struct or Object receiver.
fn check_nf_recv(recv: &SwiftValue) -> Result<(), StdError> {
    match recv {
        SwiftValue::Struct(obj) if obj.type_name == "NumberFormatter" => Ok(()),
        SwiftValue::Object(o) if o.borrow().class_name == "NumberFormatter" => Ok(()),
        other => Err(type_error(format!(
            "expected NumberFormatter, got {}",
            other.type_name()
        ))),
    }
}

/// Resolve `numberStyle` (stored as Int ordinal or `.style` enum) to a [`Style`].
fn number_style(recv: &SwiftValue) -> Style {
    match read_nf_field(recv, "numberStyle") {
        Some(SwiftValue::Int(i)) => Style::from_ordinal(i.raw),
        Some(SwiftValue::Enum(e)) => Style::from_name(&e.case),
        _ => Style::None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    None,
    Decimal,
    Currency,
    Percent,
    Scientific,
}

impl Style {
    fn from_ordinal(value: i128) -> Style {
        match value {
            1 => Style::Decimal,
            2 => Style::Currency,
            3 => Style::Percent,
            4 => Style::Scientific,
            _ => Style::None,
        }
    }

    fn from_name(name: &str) -> Style {
        match name {
            "decimal" => Style::Decimal,
            "currency" => Style::Currency,
            "percent" => Style::Percent,
            "scientific" => Style::Scientific,
            _ => Style::None,
        }
    }
}

fn int_field(recv: &SwiftValue, field: &str) -> i64 {
    match read_nf_field(recv, field) {
        Some(SwiftValue::Int(i)) => i.raw as i64,
        _ => -1,
    }
}

fn str_field(recv: &SwiftValue, field: &str, default: &str) -> String {
    match read_nf_field(recv, field) {
        Some(SwiftValue::Str(s)) => s.to_string(),
        _ => default.to_string(),
    }
}

fn number_as_f64(value: &SwiftValue) -> Option<f64> {
    match value {
        SwiftValue::Int(i) => Some(i.raw as f64),
        SwiftValue::Double(d) => Some(*d),
        _ => None,
    }
}

/// Per-style default (min, max) fraction digits and whether grouping is on.
fn style_defaults(style: Style) -> (i64, i64, bool) {
    match style {
        Style::None => (0, 0, false),
        Style::Decimal => (0, 3, true),
        Style::Currency => (2, 2, true),
        Style::Percent => (0, 0, true),
        Style::Scientific => (0, 6, false),
    }
}

#[allow(clippy::too_many_arguments)]
fn format_decimal(
    value: f64,
    min_frac: i64,
    max_frac: i64,
    grouping: bool,
    group_sep: &str,
    decimal_sep: &str,
) -> String {
    let negative = value < 0.0;
    let max = max_frac.max(0) as usize;
    // Fixed-precision render, then trim trailing zeros down to min_frac.
    let fixed = format!("{:.*}", max, value.abs());
    let (int_part, frac_part) = match fixed.split_once('.') {
        Some((i, f)) => (i.to_string(), f.to_string()),
        None => (fixed, String::new()),
    };
    let mut frac: Vec<char> = frac_part.chars().collect();
    while frac.len() > min_frac.max(0) as usize && frac.last() == Some(&'0') {
        frac.pop();
    }

    let grouped = if grouping {
        group_integer(&int_part, group_sep)
    } else {
        int_part
    };
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(&grouped);
    if !frac.is_empty() {
        out.push_str(decimal_sep);
        out.extend(frac);
    }
    out
}

fn group_integer(digits: &str, separator: &str) -> String {
    let chars: Vec<char> = digits.chars().collect();
    let mut out = String::new();
    let len = chars.len();
    for (idx, ch) in chars.iter().enumerate() {
        if idx > 0 && (len - idx).is_multiple_of(3) {
            out.push_str(separator);
        }
        out.push(*ch);
    }
    out
}

fn number_formatter_string(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let [number] = args.as_slice() else {
        return Err(type_error(
            "NumberFormatter.string(from:) expects one argument",
        ));
    };
    let Some(value) = number_as_f64(number) else {
        return Err(type_error(format!(
            "NumberFormatter.string(from:) expects a number, got {}",
            number.type_name()
        )));
    };
    check_nf_recv(&recv)?;
    let style = number_style(&recv);
    let (def_min, def_max, def_group) = style_defaults(style);
    let min_frac = {
        let explicit = int_field(&recv, "minimumFractionDigits");
        if explicit < 0 {
            def_min
        } else {
            explicit
        }
    };
    let max_frac = {
        let explicit = int_field(&recv, "maximumFractionDigits");
        if explicit < 0 {
            def_max.max(min_frac)
        } else {
            explicit.max(min_frac)
        }
    };
    let grouping = match read_nf_field(&recv, "usesGroupingSeparator") {
        Some(SwiftValue::Bool(b)) => b,
        // Unset: fall back to the per-style default.
        _ => def_group,
    };
    let group_sep = str_field(&recv, "groupingSeparator", ",");
    let decimal_sep = str_field(&recv, "decimalSeparator", ".");

    let body = match style {
        Style::Percent => {
            let formatted = format_decimal(
                value * 100.0,
                min_frac,
                max_frac,
                grouping,
                &group_sep,
                &decimal_sep,
            );
            format!("{formatted}%")
        }
        Style::Currency => {
            let symbol = str_field(&recv, "currencySymbol", "$");
            let formatted = format_decimal(
                value.abs(),
                min_frac,
                max_frac,
                grouping,
                &group_sep,
                &decimal_sep,
            );
            if value < 0.0 {
                format!("-{symbol}{formatted}")
            } else {
                format!("{symbol}{formatted}")
            }
        }
        _ => format_decimal(
            value,
            min_frac,
            max_frac,
            grouping,
            &group_sep,
            &decimal_sep,
        ),
    };

    Ok(Outcome {
        result: SwiftValue::Str(body),
        receiver: recv,
    })
}

fn number_formatter_number(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let [SwiftValue::Str(input)] = args.as_slice() else {
        return Err(type_error(
            "NumberFormatter.number(from:) expects a String argument",
        ));
    };
    check_nf_recv(&recv)?;
    let style = number_style(&recv);
    let group_sep = str_field(&recv, "groupingSeparator", ",");
    let decimal_sep = str_field(&recv, "decimalSeparator", ".");
    let currency_symbol = str_field(&recv, "currencySymbol", "$");

    let mut cleaned = input.replace(&group_sep, "");
    cleaned = cleaned.replace(&currency_symbol, "");
    let is_percent = cleaned.contains('%');
    cleaned = cleaned.replace('%', "");
    if decimal_sep != "." {
        cleaned = cleaned.replace(&decimal_sep, ".");
    }
    let cleaned = cleaned.trim();

    let result = match cleaned.parse::<f64>() {
        Ok(mut number) => {
            if is_percent || style == Style::Percent {
                number /= 100.0;
            }
            if number.fract() == 0.0 && number.abs() < i64::MAX as f64 {
                SwiftValue::int(number as i128)
            } else {
                SwiftValue::Double(number)
            }
        }
        Err(_) => SwiftValue::Nil,
    };
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal no-op context for intrinsics that never call closures or write
    // output (mirrors the `PanicCtx` pattern in `formatter.rs`).
    struct PanicCtx;
    impl StdContext for PanicCtx {
        fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> StdResult {
            unreachable!("number formatter helpers never call closures")
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            unreachable!("number formatter helpers never write output")
        }
    }

    #[test]
    fn decimal_grouping_and_fraction() {
        assert_eq!(
            format_decimal(1234567.5, 0, 2, true, ",", "."),
            "1,234,567.5"
        );
        assert_eq!(format_decimal(1234.0, 2, 2, true, ",", "."), "1,234.00");
        assert_eq!(format_decimal(-12.5, 0, 3, false, ",", "."), "-12.5");
    }

    #[test]
    fn grouping_boundaries() {
        assert_eq!(group_integer("1", ","), "1");
        assert_eq!(group_integer("100", ","), "100");
        assert_eq!(group_integer("1000", ","), "1,000");
        assert_eq!(group_integer("1234567", ","), "1,234,567");
    }

    // ----- Phase 2b reference-semantics tests --------------------------------

    #[test]
    fn number_formatter_init_returns_object() {
        let result = number_formatter_init(&mut PanicCtx, vec![]).unwrap();
        assert!(
            matches!(&result, SwiftValue::Object(o)
                if o.borrow().class_name == "NumberFormatter"),
            "expected Object with class_name NumberFormatter, got {result:?}"
        );
    }

    /// An alias of a `NumberFormatter` Object observes property mutations
    /// written through the alias (reference semantics — Swift class behaviour).
    #[test]
    fn number_formatter_alias_observes_property_change() {
        let nf = number_formatter_object();
        let alias = nf.clone(); // shallow Rc clone — same ClassObj
                                // Write numberStyle = 1 (decimal) through the alias.
        if let SwiftValue::Object(o) = &alias {
            o.borrow_mut().set("numberStyle", SwiftValue::int(1));
        } else {
            panic!("alias was not Object");
        }
        // The original binding must see the mutation.
        let field = read_nf_field(&nf, "numberStyle");
        assert_eq!(
            field,
            Some(SwiftValue::int(1)),
            "original did not observe alias mutation"
        );
    }

    /// `string(from:)` uses the `numberStyle` stored in the Object, so a
    /// property set before the call is reflected in the output.
    #[test]
    fn number_formatter_string_reflects_object_number_style() {
        let nf = number_formatter_object();
        // Set decimal style (ordinal 1) directly on the Object.
        if let SwiftValue::Object(o) = &nf {
            o.borrow_mut().set("numberStyle", SwiftValue::int(1));
        }
        let out = number_formatter_string(&mut PanicCtx, nf, vec![SwiftValue::int(1234)]).unwrap();
        assert_eq!(out.result, SwiftValue::Str("1,234".into()));
    }

    /// `let nf = NumberFormatter()` — verifies that `mutating: false` is
    /// correct: the receiver is returned unchanged (no write-back needed).
    #[test]
    fn string_intrinsic_returns_unchanged_receiver() {
        let nf = number_formatter_object();
        let before = if let SwiftValue::Object(o) = &nf {
            Rc::as_ptr(o)
        } else {
            panic!("not Object")
        };
        let out =
            number_formatter_string(&mut PanicCtx, nf, vec![SwiftValue::Double(3.14)]).unwrap();
        let after = if let SwiftValue::Object(o) = &out.receiver {
            Rc::as_ptr(o)
        } else {
            panic!("receiver not Object")
        };
        assert_eq!(before, after, "receiver Rc pointer should be identical");
    }
}
