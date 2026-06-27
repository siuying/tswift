//! Free-function intrinsics (no receiver).
//!
//! Output (`print`/`debugPrint`/`dump`), comparison (`min`/`max`/`abs`),
//! sequence builders (`zip`/`stride`/`repeatElement`/`sequence`), the
//! diagnostic family (`assert`/`precondition`/`fatalError`/…), and `readLine`.
//!
//! `swap` and `isKnownUniquelyReferenced` need caller `Place`s / reference
//! identity and are served directly by the interpreter, not through this seam.

use std::io::BufRead;
use std::rc::Rc;

use tswift_core::{Arg, EvalError, Interpreter, StdContext, StdError, StdResult, SwiftValue};

/// Register the free functions of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_free_fn("print", print);
    interp.register_free_fn("debugPrint", debug_print);
    interp.register_free_fn("dump", dump);
    interp.register_free_fn("min", min);
    interp.register_free_fn("max", max);
    interp.register_free_fn("abs", abs);
    interp.register_free_fn("zip", zip);
    interp.register_free_fn("stride", stride);
    interp.register_free_fn("repeatElement", repeat_element);
    interp.register_free_fn("sequence", sequence);
    interp.register_free_fn("readLine", read_line);
    interp.register_free_fn("assert", assert);
    interp.register_free_fn("assertionFailure", assertion_failure);
    interp.register_free_fn("precondition", precondition);
    interp.register_free_fn("preconditionFailure", precondition_failure);
    interp.register_free_fn("fatalError", fatal_error);
}

// ---- output ----------------------------------------------------------------

/// `print(_:separator:terminator:)` — display each item, default `" "`/`"\n"`.
fn print(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let (items, sep, term) = output_parts(args, " ", "\n");
    let rendered: Vec<String> = items.iter().map(|v| ctx.display(v)).collect();
    let line = rendered.join(&sep);
    let _ = write!(ctx.out(), "{line}{term}");
    Ok(SwiftValue::Void)
}

/// `debugPrint(_:separator:terminator:)` — like `print` but each item uses its
/// debug representation (strings are quoted, nesting is preserved).
fn debug_print(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let (items, sep, term) = output_parts(args, " ", "\n");
    let line = items
        .iter()
        .map(debug_format)
        .collect::<Vec<_>>()
        .join(&sep);
    let _ = write!(ctx.out(), "{line}{term}");
    Ok(SwiftValue::Void)
}

/// `dump(_:)` — print a reflection tree and return the value unchanged.
///
/// A pragmatic subset: scalars render as `- <debug>`; sequences render a
/// `▿ N elements` header followed by one `  - <debug>` line per element.
fn dump(ctx: &mut dyn StdContext, mut args: Vec<Arg>) -> StdResult {
    if args.is_empty() {
        return Ok(SwiftValue::Void);
    }
    let value = args.remove(0).value;
    match as_sequence(&value) {
        Some(items) => {
            let _ = writeln!(
                ctx.out(),
                "▿ {} element{}",
                items.len(),
                plural(items.len())
            );
            for item in &items {
                let _ = writeln!(ctx.out(), "  - {}", debug_format(item));
            }
        }
        None => {
            let _ = writeln!(ctx.out(), "- {}", debug_format(&value));
        }
    }
    Ok(value)
}

/// Split output args into `(items, separator, terminator)`, honouring the
/// `separator:`/`terminator:` labels.
fn output_parts(
    args: Vec<Arg>,
    def_sep: &str,
    def_term: &str,
) -> (Vec<SwiftValue>, String, String) {
    let mut items = Vec::new();
    let mut sep = def_sep.to_string();
    let mut term = def_term.to_string();
    for arg in args {
        match arg.label.as_deref() {
            Some("separator") => sep = arg.value.to_string(),
            Some("terminator") => term = arg.value.to_string(),
            _ => items.push(arg.value),
        }
    }
    (items, sep, term)
}

// ---- comparison ------------------------------------------------------------

/// `min(_:_:...)` — the least of two or more comparable values.
fn min(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    fold_extreme(ctx, args, false)
}

/// `max(_:_:...)` — the greatest of two or more comparable values.
fn max(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    fold_extreme(ctx, args, true)
}

/// `abs(_:)` — magnitude of an integer or floating-point value.
fn abs(_ctx: &mut dyn StdContext, mut args: Vec<Arg>) -> StdResult {
    let v = take_one(&mut args, "abs")?;
    match v {
        SwiftValue::Int(i) => {
            let mag = i.raw.checked_abs().ok_or_else(|| {
                StdError::Error(EvalError::Trap("arithmetic overflow in abs".into()))
            })?;
            Ok(SwiftValue::Int(tswift_core::IntValue::new(mag, i.width)))
        }
        SwiftValue::Double(d) => Ok(SwiftValue::Double(d.abs())),
        other => Err(type_err(format!(
            "abs expects a number, got {}",
            other.type_name()
        ))),
    }
}

/// Reduce 2+ comparable arguments to an extreme. `want_greater` selects `max`;
/// otherwise `min`. Ordering goes through `ctx.value_less_than`, so user types
/// conforming to `Comparable` (a `static func <`) work, not just scalars. Ties
/// mirror Swift: `min` keeps the earlier value, `max` the later.
fn fold_extreme(ctx: &mut dyn StdContext, args: Vec<Arg>, want_greater: bool) -> StdResult {
    let mut values = args.into_iter().map(|a| a.value);
    let mut best = values
        .next()
        .ok_or_else(|| type_err("min/max requires at least two arguments".into()))?;
    for v in values {
        let less = ctx
            .value_less_than(&v, &best)
            .ok_or_else(|| type_err("min/max arguments are not comparable".into()))?;
        // min replaces when `v < best`; max replaces when `v >= best`.
        if if want_greater { !less } else { less } {
            best = v;
        }
    }
    Ok(best)
}

// ---- sequence builders -----------------------------------------------------

/// `zip(_:_:)` — pair elements of two sequences into a tuple array (eager).
fn zip(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut it = args.into_iter();
    let a = it
        .next()
        .ok_or_else(|| type_err("zip expects two sequences".into()))?;
    let b = it
        .next()
        .ok_or_else(|| type_err("zip expects two sequences".into()))?;
    let xs =
        as_sequence(&a.value).ok_or_else(|| type_err("zip argument is not a sequence".into()))?;
    let ys =
        as_sequence(&b.value).ok_or_else(|| type_err("zip argument is not a sequence".into()))?;
    let pairs = xs
        .into_iter()
        .zip(ys)
        .map(|(x, y)| SwiftValue::tuple(vec![x, y]))
        .collect();
    Ok(SwiftValue::Array(Rc::new(pairs)))
}

/// `stride(from:to:by:)` (exclusive) / `stride(from:through:by:)` (inclusive).
fn stride(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let from = labeled(&args, "from").ok_or_else(|| type_err("stride needs from:".into()))?;
    let by = labeled(&args, "by").ok_or_else(|| type_err("stride needs by:".into()))?;
    let (limit, inclusive) = match (labeled(&args, "to"), labeled(&args, "through")) {
        (Some(t), _) => (t, false),
        (_, Some(t)) => (t, true),
        _ => return Err(type_err("stride needs to: or through:".into())),
    };

    // Integer stride when all bounds are integers; otherwise floating-point.
    if let (SwiftValue::Int(f), SwiftValue::Int(s), SwiftValue::Int(l)) = (&from, &by, &limit) {
        let (mut cur, step, lim) = (f.raw, s.raw, l.raw);
        if step == 0 {
            return Err(type_err("stride by: must be non-zero".into()));
        }
        let mut out = Vec::new();
        while stride_continues(cur, lim, step, inclusive) {
            out.push(SwiftValue::int(cur));
            cur += step;
        }
        return Ok(SwiftValue::Array(Rc::new(out)));
    }

    let (mut cur, step, lim) = (as_f64(&from)?, as_f64(&by)?, as_f64(&limit)?);
    if step == 0.0 {
        return Err(type_err("stride by: must be non-zero".into()));
    }
    let mut out = Vec::new();
    while stride_continues_f(cur, lim, step, inclusive) {
        out.push(SwiftValue::Double(cur));
        cur += step;
    }
    Ok(SwiftValue::Array(Rc::new(out)))
}

fn stride_continues(cur: i128, lim: i128, step: i128, inclusive: bool) -> bool {
    if step > 0 {
        if inclusive {
            cur <= lim
        } else {
            cur < lim
        }
    } else if inclusive {
        cur >= lim
    } else {
        cur > lim
    }
}

fn stride_continues_f(cur: f64, lim: f64, step: f64, inclusive: bool) -> bool {
    if step > 0.0 {
        if inclusive {
            cur <= lim
        } else {
            cur < lim
        }
    } else if inclusive {
        cur >= lim
    } else {
        cur > lim
    }
}

/// `repeatElement(_:count:)` — an array of `count` copies of the element.
fn repeat_element(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let element = args
        .iter()
        .find(|a| a.label.is_none())
        .map(|a| a.value.clone())
        .ok_or_else(|| type_err("repeatElement expects an element".into()))?;
    let count = labeled(&args, "count")
        .and_then(|v| as_index(&v))
        .ok_or_else(|| type_err("repeatElement needs count:".into()))?;
    Ok(SwiftValue::Array(Rc::new(vec![element; count])))
}

/// `sequence(first:next:)` — unfold from `first`, applying `next` until it
/// returns `nil`, materialized eagerly into an array.
fn sequence(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let first = labeled(&args, "first")
        .ok_or_else(|| type_err("sequence(first:next:) needs first:".into()))?;
    let next = args
        .iter()
        .rev()
        .find_map(|a| match a.value {
            SwiftValue::Closure(id) => Some(id),
            _ => None,
        })
        .ok_or_else(|| type_err("sequence(first:next:) needs a next closure".into()))?;

    let mut out = vec![first.clone()];
    let mut cur = first;
    loop {
        match ctx.call_closure(next, vec![cur])? {
            SwiftValue::Nil => break,
            v => {
                out.push(v.clone());
                cur = v;
            }
        }
    }
    Ok(SwiftValue::Array(Rc::new(out)))
}

// ---- input -----------------------------------------------------------------

/// `readLine(strippingNewline:)` — a line from stdin, or `nil` at end of input.
fn read_line(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let strip = labeled(&args, "strippingNewline")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut line = String::new();
    let n = std::io::stdin().lock().read_line(&mut line).unwrap_or(0);
    if n == 0 {
        return Ok(SwiftValue::Nil);
    }
    if strip {
        while line.ends_with('\n') || line.ends_with('\r') {
            line.pop();
        }
    }
    Ok(SwiftValue::Str(line))
}

// ---- diagnostics -----------------------------------------------------------

/// `assert(_:_:)` — evaluate the condition; trap with the message if false.
fn assert(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    check_condition(&args, "assertion failed")
}

/// `assertionFailure(_:)` — always trap with the message.
fn assertion_failure(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    Err(fatal(&args, "fatal error"))
}

/// `precondition(_:_:)` — evaluate the condition; trap with the message if false.
fn precondition(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    check_condition(&args, "precondition failed")
}

/// `preconditionFailure(_:)` — always trap with the message.
fn precondition_failure(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    Err(fatal(&args, "fatal error"))
}

/// `fatalError(_:)` — always trap with the message.
fn fatal_error(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    Err(fatal(&args, "fatal error"))
}

/// Evaluate the leading boolean condition; if false, trap with the message arg.
fn check_condition(args: &[Arg], kind: &str) -> StdResult {
    let cond = args
        .iter()
        .find(|a| a.label.is_none())
        .and_then(|a| a.value.as_bool())
        .unwrap_or(false);
    if cond {
        Ok(SwiftValue::Void)
    } else {
        Err(fatal_with_prefix(args, kind))
    }
}

fn fatal(args: &[Arg], default: &str) -> StdError {
    let msg = args
        .iter()
        .filter(|a| a.label.is_none())
        .find_map(|a| match &a.value {
            SwiftValue::Str(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let text = if msg.is_empty() {
        default.to_string()
    } else {
        msg
    };
    StdError::Error(EvalError::Trap(text))
}

fn fatal_with_prefix(args: &[Arg], kind: &str) -> StdError {
    // The message is the second positional argument (after the condition).
    let msg = args
        .iter()
        .filter(|a| a.label.is_none())
        .nth(1)
        .and_then(|a| match &a.value {
            SwiftValue::Str(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let text = if msg.is_empty() {
        kind.to_string()
    } else {
        format!("{kind}: {msg}")
    };
    StdError::Error(EvalError::Trap(text))
}

// ---- shared helpers --------------------------------------------------------

/// Find the first argument carrying `label`.
fn labeled(args: &[Arg], label: &str) -> Option<SwiftValue> {
    args.iter()
        .find(|a| a.label.as_deref() == Some(label))
        .map(|a| a.value.clone())
}

/// Pop the single expected argument from `args`.
fn take_one(args: &mut Vec<Arg>, who: &str) -> Result<SwiftValue, StdError> {
    if args.is_empty() {
        return Err(type_err(format!("{who} expects one argument")));
    }
    Ok(args.remove(0).value)
}

fn type_err(msg: String) -> StdError {
    StdError::Error(EvalError::Type(msg))
}

fn as_f64(v: &SwiftValue) -> Result<f64, StdError> {
    match v {
        SwiftValue::Int(i) => Ok(i.raw as f64),
        SwiftValue::Double(d) => Ok(*d),
        other => Err(type_err(format!(
            "expected a number, got {}",
            other.type_name()
        ))),
    }
}

fn as_index(v: &SwiftValue) -> Option<usize> {
    match v {
        SwiftValue::Int(i) if i.raw >= 0 => Some(i.raw as usize),
        _ => None,
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Eagerly expand a builtin sequence value into its elements.
fn as_sequence(value: &SwiftValue) -> Option<Vec<SwiftValue>> {
    match value {
        SwiftValue::Array(items) => Some(items.as_ref().clone()),
        SwiftValue::Range { lo, hi, inclusive } => {
            let end = if *inclusive { *hi + 1 } else { *hi };
            Some((*lo..end).map(SwiftValue::int).collect())
        }
        _ => None,
    }
}

/// The debug (`String(reflecting:)`) rendering: strings quoted, nesting kept.
fn debug_format(v: &SwiftValue) -> String {
    match v {
        SwiftValue::Str(s) => format!("{s:?}"),
        SwiftValue::Array(items) => {
            let inner = items
                .iter()
                .map(debug_format)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{inner}]")
        }
        SwiftValue::Tuple(items, labels) => {
            let inner = items
                .iter()
                .enumerate()
                .map(|(i, v)| match labels.get(i) {
                    Some(Some(label)) => format!("{label}: {}", debug_format(v)),
                    _ => debug_format(v),
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("({inner})")
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockCtx {
        sink: Vec<u8>,
    }
    impl StdContext for MockCtx {
        fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> StdResult {
            Ok(SwiftValue::Nil)
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            &mut self.sink
        }
    }

    fn pos(v: SwiftValue) -> Arg {
        Arg::positional(v)
    }
    fn lab(l: &str, v: SwiftValue) -> Arg {
        Arg {
            label: Some(l.into()),
            value: v,
        }
    }

    #[test]
    fn min_max_pick_extremes() {
        let mut c = MockCtx { sink: vec![] };
        assert_eq!(
            min(
                &mut c,
                vec![pos(SwiftValue::int(3)), pos(SwiftValue::int(7))]
            )
            .unwrap(),
            SwiftValue::int(3)
        );
        assert_eq!(
            max(
                &mut c,
                vec![pos(SwiftValue::int(3)), pos(SwiftValue::int(7))]
            )
            .unwrap(),
            SwiftValue::int(7)
        );
    }

    #[test]
    fn abs_traps_on_int_min_and_handles_doubles() {
        let mut c = MockCtx { sink: vec![] };
        assert_eq!(
            abs(&mut c, vec![pos(SwiftValue::int(-5))]).unwrap(),
            SwiftValue::int(5)
        );
        assert_eq!(
            abs(&mut c, vec![pos(SwiftValue::Double(-2.5))]).unwrap(),
            SwiftValue::Double(2.5)
        );
    }

    #[test]
    fn stride_exclusive_and_inclusive() {
        let mut c = MockCtx { sink: vec![] };
        let ex = stride(
            &mut c,
            vec![
                lab("from", SwiftValue::int(0)),
                lab("to", SwiftValue::int(10)),
                lab("by", SwiftValue::int(3)),
            ],
        )
        .unwrap();
        assert_eq!(as_sequence(&ex).unwrap().len(), 4); // 0,3,6,9

        let inc = stride(
            &mut c,
            vec![
                lab("from", SwiftValue::int(1)),
                lab("through", SwiftValue::int(5)),
                lab("by", SwiftValue::int(2)),
            ],
        )
        .unwrap();
        assert_eq!(
            as_sequence(&inc).unwrap(),
            vec![SwiftValue::int(1), SwiftValue::int(3), SwiftValue::int(5)]
        );
    }

    #[test]
    fn fatal_error_traps() {
        let mut c = MockCtx { sink: vec![] };
        let err = fatal_error(&mut c, vec![pos(SwiftValue::Str("boom".into()))]).unwrap_err();
        assert!(matches!(err, StdError::Error(EvalError::Trap(m)) if m == "boom"));
    }

    #[test]
    fn assert_passes_when_true_traps_when_false() {
        let mut c = MockCtx { sink: vec![] };
        assert!(assert(&mut c, vec![pos(SwiftValue::Bool(true))]).is_ok());
        assert!(assert(&mut c, vec![pos(SwiftValue::Bool(false))]).is_err());
    }

    #[test]
    fn debug_format_quotes_strings() {
        assert_eq!(debug_format(&SwiftValue::Str("hi".into())), "\"hi\"");
        assert_eq!(debug_format(&SwiftValue::int(42)), "42");
    }
}
