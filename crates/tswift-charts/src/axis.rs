//! Axis-content builtins (AxisMarks / AxisGridLine / AxisTick / AxisValueLabel).

use std::rc::Rc;

use tswift_core::{Arg, StdContext, StdResult, SwiftValue};
use tswift_swiftui::{collect_children, view_value};

use crate::marks::mark_leaf_all_labeled;

/// `AxisMarks(...)` / `AxisMarks { … }` / `AxisMarks(values:) { … }` —
/// axis-content view. Labeled args (`values`, `preset`, `position`, …) become
/// fields; trailing `@AxisMarkBuilder` content becomes `_children`.
pub(crate) fn axis_marks_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut fields: Vec<(String, SwiftValue)> = Vec::new();
    let mut content_args: Vec<Arg> = Vec::new();
    for arg in args {
        match arg.label.as_deref() {
            Some("values") | Some("preset") | Some("position") | Some("stroke")
            | Some("format") => {
                fields.push((arg.label.expect("label checked"), arg.value));
            }
            Some("content") => content_args.push(Arg {
                label: None,
                value: arg.value,
                static_ty: None,
            }),
            None => content_args.push(arg),
            Some(other) => fields.push((other.into(), arg.value)),
        }
    }
    if !content_args.is_empty() {
        let children = collect_children(ctx, content_args)?;
        if !children.is_empty() {
            fields.push((
                tswift_swiftui::CHILDREN_FIELD.into(),
                SwiftValue::Array(Rc::new(children)),
            ));
        }
    }
    Ok(view_value("AxisMarks", fields))
}

/// `AxisGridLine()` — axis-content leaf (grid lines).
pub(crate) fn axis_grid_line_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf_all_labeled("AxisGridLine", args)
}

/// `AxisTick()` — axis-content leaf (tick marks).
pub(crate) fn axis_tick_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf_all_labeled("AxisTick", args)
}

/// `AxisValueLabel()` / `AxisValueLabel("title")` / `AxisValueLabel { Text… }` —
/// axis-content leaf. String title stays a field; trailing `@ViewBuilder`
/// content is child-collected into `_children` (never a raw Closure).
pub(crate) fn axis_value_label_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut fields = Vec::new();
    let mut content_args: Vec<Arg> = Vec::new();
    let mut positional = 0usize;
    for arg in args {
        match arg.label.as_deref() {
            Some("content") => content_args.push(Arg {
                label: None,
                value: arg.value,
                static_ty: None,
            }),
            Some(other) => fields.push((other.into(), arg.value)),
            None => match &arg.value {
                // Trailing `@ViewBuilder` form: `AxisValueLabel { Text("x") }`.
                SwiftValue::Closure(_) => content_args.push(arg),
                // String (or other scalar) title form: `AxisValueLabel("title")`.
                _ => {
                    let key = if positional == 0 {
                        "title".into()
                    } else {
                        format!("value{positional}")
                    };
                    positional += 1;
                    fields.push((key, arg.value));
                }
            },
        }
    }
    if !content_args.is_empty() {
        let children = collect_children(ctx, content_args)?;
        if !children.is_empty() {
            fields.push((
                tswift_swiftui::CHILDREN_FIELD.into(),
                SwiftValue::Array(Rc::new(children)),
            ));
        }
    }
    Ok(view_value("AxisValueLabel", fields))
}
