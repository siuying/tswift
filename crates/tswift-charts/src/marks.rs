//! Chart container, mark leaf constructors, and PlottableValue construction.

use std::rc::Rc;

use tswift_core::{Arg, BuiltinParam, StdContext, StdResult, StructObj, SwiftValue};
use tswift_swiftui::{collect_children, container_value, view_value};

/// PlottableValue-typed x/y params shared by Bar/Line/Point/Area marks.
pub(crate) fn xy_plottable_params() -> Vec<BuiltinParam> {
    vec![
        BuiltinParam::labeled("x", "PlottableValue"),
        BuiltinParam::labeled("y", "PlottableValue"),
    ]
}

/// `Chart { marks… }` — container view collecting content-builder children.
pub(crate) fn chart_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    Ok(container_value("Chart", collect_children(ctx, args)?))
}

/// Build a mark leaf view from the subset of `wanted` labels present in `args`
/// (order follows `wanted`, matching BarMark's x-then-y field construction).
pub(crate) fn mark_leaf(kind: &str, args: Vec<Arg>, wanted: &[&str]) -> StdResult {
    let mut found: Vec<(String, Option<SwiftValue>)> =
        wanted.iter().map(|l| ((*l).into(), None)).collect();
    for arg in args {
        if let Some(label) = arg.label.as_deref() {
            if let Some(slot) = found.iter_mut().find(|(l, _)| l == label) {
                slot.1 = Some(arg.value);
            }
        }
    }
    let fields = found
        .into_iter()
        .filter_map(|(label, value)| value.map(|v| (label, v)))
        .collect();
    Ok(view_value(kind, fields))
}

/// Store every labeled arg as a field (RuleMark's variable forms).
pub(crate) fn mark_leaf_all_labeled(kind: &str, args: Vec<Arg>) -> StdResult {
    let mut fields = Vec::new();
    for arg in args {
        if let Some(label) = arg.label {
            fields.push((label, arg.value));
        }
    }
    Ok(view_value(kind, fields))
}

/// `BarMark(x:y:)` — mark leaf carrying its plottable x/y args.
pub(crate) fn bar_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf("BarMark", args, &["x", "y"])
}

/// `LineMark(x:y:)` — line series mark.
pub(crate) fn line_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf("LineMark", args, &["x", "y"])
}

/// `PointMark(x:y:)` — scatter/point mark.
pub(crate) fn point_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf("PointMark", args, &["x", "y"])
}

/// `AreaMark(x:y:)` — filled area under a series.
pub(crate) fn area_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf("AreaMark", args, &["x", "y"])
}

/// `RuleMark(...)` — store whatever labeled PlottableValue args are passed
/// (`x:`, `y:`, `xStart:xEnd:y:`, `yStart:yEnd:x:`, …).
pub(crate) fn rule_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf_all_labeled("RuleMark", args)
}

/// `RectangleMark(x:y:width:height:)` — store provided args.
pub(crate) fn rectangle_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf("RectangleMark", args, &["x", "y", "width", "height"])
}

/// `SectorMark(angle:innerRadius:angularInset:)` — pie/donut sector.
pub(crate) fn sector_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf(
        "SectorMark",
        args,
        &["angle", "innerRadius", "angularInset"],
    )
}

/// `PlottableValue.value(_ label:, _ value:)` — Rust intrinsic fallback used
/// for qualified `PlottableValue.value(...)` calls and coverage registration.
pub(crate) fn plottable_value_static(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut label = String::new();
    let mut value = SwiftValue::Void;
    let mut pos = 0usize;
    for arg in args {
        match arg.label.as_deref() {
            Some("label") => label = arg.value.to_string(),
            Some("value") => value = arg.value,
            // Two unlabeled positionals: label then value (the Swift signature).
            None => {
                if pos == 0 {
                    label = arg.value.to_string();
                    pos = 1;
                } else {
                    value = arg.value;
                    pos = 2;
                }
            }
            _ => {}
        }
    }
    Ok(SwiftValue::Struct(Rc::new(StructObj {
        type_name: "PlottableValue".into(),
        fields: vec![
            ("label".into(), SwiftValue::Str(label)),
            ("value".into(), value),
        ],
    })))
}
