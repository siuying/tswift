//! Swift Charts marks (ADR-0020).
//!
//! A `Chart { … }` is a container view whose children are *marks*
//! (`BarMark`/`LineMark`/`PointMark`/`AreaMark`/`RuleMark`/`RectangleMark`/
//! `SectorMark`). Each mark is a leaf UIIR node whose args are the plotted
//! channels it was given (`x`/`y`/`xStart`/`yEnd`/… as `PlottableValue`s, plus
//! `width`/`height` `MarkDimension`s and a `stacking` token). The channels are
//! recorded verbatim and serialized by `uiir.rs`; no scale resolution or data
//! aggregation happens in the runtime (the host draws from the recorded
//! channels — see ADR-0020 for the honest fidelity tier).

use tswift_core::{Arg, BuiltinParam, Interpreter, StdContext, StdResult, SwiftValue};

use crate::values::{container_value, view_value};
use crate::views::{collect_children, keyed_rows};

/// The plotted channels a mark records verbatim when supplied. Anything else
/// (an unrecognized label) is dropped rather than mis-serialized.
const MARK_CHANNELS: &[&str] = &[
    "x",
    "y",
    "xStart",
    "xEnd",
    "yStart",
    "yEnd",
    "series",
    "angle",
    "width",
    "height",
    "innerRadius",
    "outerRadius",
    "angularInset",
    "stacking",
];

/// Build a leaf mark node of kind `type_name`, recording each supplied plotted
/// channel as an arg field.
fn mark_init(type_name: &str, args: Vec<Arg>) -> StdResult {
    let mut fields: Vec<(String, SwiftValue)> = Vec::new();
    for arg in args {
        if let Some(label) = arg.label.as_deref() {
            if MARK_CHANNELS.contains(&label) {
                fields.push((label.to_string(), arg.value));
            }
        }
    }
    Ok(view_value(type_name, fields))
}

fn bar_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_init("BarMark", args)
}

fn line_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_init("LineMark", args)
}

fn point_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_init("PointMark", args)
}

fn area_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_init("AreaMark", args)
}

fn rule_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_init("RuleMark", args)
}

fn rectangle_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_init("RectangleMark", args)
}

fn sector_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_init("SectorMark", args)
}

/// `Chart { … }` (static marks) or `Chart(data, id:) { d in … }` (data-driven,
/// sugar for a keyed `ForEach` of marks — one mark subtree per element).
fn chart_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let data_driven = args
        .iter()
        .any(|a| a.label.is_none() && !matches!(a.value, SwiftValue::Closure(_)));
    let children = if data_driven {
        keyed_rows(ctx, args, "Chart")?
    } else {
        collect_children(ctx, args)?
    };
    Ok(container_value("Chart", children))
}

/// Typed params shared by every mark init so leading-dot channel values resolve
/// against the right namespace: `x: .value(…)` → `PlottableValue.value`,
/// `width: .fixed(20)` → `MarkDimension.fixed`, `stacking: .center` →
/// `MarkStackingMethod.center` (the issue #203 typed-param pattern).
fn mark_params() -> Vec<BuiltinParam> {
    let mut params = Vec::new();
    for channel in [
        "x", "y", "xStart", "xEnd", "yStart", "yEnd", "series", "angle",
    ] {
        params.push(BuiltinParam::labeled(channel, "PlottableValue"));
    }
    for channel in ["width", "height", "innerRadius", "outerRadius"] {
        params.push(BuiltinParam::labeled(channel, "MarkDimension"));
    }
    params.push(BuiltinParam::labeled("stacking", "MarkStackingMethod"));
    params
}

/// Register the Charts view constructors into `interp`. Called by
/// [`crate::install`] so charts marks render through the same session/UIIR
/// pipeline as native SwiftUI views.
pub(crate) fn install(interp: &mut Interpreter<'_>) {
    interp.register_free_fn("Chart", chart_init);
    for (name, f) in MARK_FNS {
        interp.register_free_fn_typed(name, *f, mark_params());
    }
}

/// The mark constructors, paired for both registration and the coverage dump.
type MarkFn = fn(&mut dyn StdContext, Vec<Arg>) -> StdResult;
const MARK_FNS: &[(&str, MarkFn)] = &[
    ("BarMark", bar_mark_init),
    ("LineMark", line_mark_init),
    ("PointMark", point_mark_init),
    ("AreaMark", area_mark_init),
    ("RuleMark", rule_mark_init),
    ("RectangleMark", rectangle_mark_init),
    ("SectorMark", sector_mark_init),
];

/// Coverage keys for the `charts` framework registry dump (`Chart.init`,
/// `BarMark.init`, …). Mirrors [`crate::registered_keys`] but scoped to Charts.
pub fn registered_keys() -> Vec<String> {
    let mut keys = vec!["Chart.init".to_string()];
    keys.extend(MARK_FNS.iter().map(|(name, _)| format!("{name}.init")));
    keys.sort();
    keys
}
