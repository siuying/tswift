//! ChartContent mark modifiers + Chart-level View modifiers.
//!
//! Marks and the `Chart` container are ordinary view values with a `_modifiers`
//! list (same shape as SwiftUI). Each modifier appends a
//! `_Modifier { name, <args> }` record via the shared COW path in
//! `tswift_swiftui::{append_modifier, make_modifier}`.
//!
//! Shared mark names (`foregroundStyle`, `cornerRadius`, `opacity`, `offset`)
//! are re-registered here so Charts can attach typed parameter hints
//! (`by:` → `PlottableValue`, etc.) without changing the append semantics
//! SwiftUI uses. Charts-only mark names (`symbol`, `lineStyle`, …) and all
//! chart-level modifiers (`chartXAxis`, …) are registered only here.

use tswift_core::{
    Arg, BuiltinParam, Interpreter, StdContext, StdError, StdResult, StructMethodFn, SwiftValue,
};
use tswift_swiftui::{
    append_modifier, collect_children, container_value, make_modifier, view_value,
};

// ── Mark modifiers ──────────────────────────────────────────────────────────

/// Plain mark modifier: store every call arg on a `_Modifier` and append.
macro_rules! mark_modifier {
    ($fn_name:ident, $swift_name:literal) => {
        fn $fn_name(_ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
            append_modifier(recv, make_modifier($swift_name, args))
        }
    };
}

mark_modifier!(modifier_foreground_style, "foregroundStyle");
mark_modifier!(modifier_symbol, "symbol");
mark_modifier!(modifier_symbol_size, "symbolSize");
mark_modifier!(modifier_line_style, "lineStyle");
mark_modifier!(modifier_interpolation_method, "interpolationMethod");
mark_modifier!(modifier_corner_radius, "cornerRadius");
mark_modifier!(modifier_opacity, "opacity");
mark_modifier!(modifier_offset, "offset");
mark_modifier!(modifier_position, "position");

/// `.annotation(position:…, …) { content }` — like SwiftUI `overlay`/`background`:
/// evaluate the `@ViewBuilder` content into a child view value stored on the
/// modifier record (under `value` when unlabeled, or `content` when labeled).
fn modifier_annotation(ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
    let mut meta: Vec<Arg> = Vec::new();
    let mut content_args: Vec<Arg> = Vec::new();
    for arg in args {
        match arg.label.as_deref() {
            Some("position") | Some("alignment") | Some("spacing") | Some("overflowResolution") => {
                meta.push(arg)
            }
            Some("content") => content_args.push(Arg {
                label: None,
                value: arg.value,
                static_ty: None,
            }),
            // Trailing `@ViewBuilder` closure / direct view (unlabeled).
            None => content_args.push(arg),
            // Unknown labels (forward-compat) stay on the record as-is.
            _ => meta.push(arg),
        }
    }
    push_collected_content(ctx, &mut meta, content_args)?;
    append_modifier(recv, make_modifier("annotation", meta))
}

/// Mark modifiers registered as generic struct methods (any mark view value).
/// Coverage keys are `ChartContent.<name>` (inventory owning protocol).
pub(crate) const MARK_MODIFIER_FNS: &[(&str, StructMethodFn)] = &[
    ("foregroundStyle", modifier_foreground_style),
    ("symbol", modifier_symbol),
    ("symbolSize", modifier_symbol_size),
    ("lineStyle", modifier_line_style),
    ("interpolationMethod", modifier_interpolation_method),
    ("annotation", modifier_annotation),
    ("cornerRadius", modifier_corner_radius),
    ("opacity", modifier_opacity),
    ("offset", modifier_offset),
    ("position", modifier_position),
];

// ── Chart-level View modifiers ──────────────────────────────────────────────

/// Plain chart modifier: store every call arg on a `_Modifier` and append.
macro_rules! chart_modifier {
    ($fn_name:ident, $swift_name:literal) => {
        fn $fn_name(_ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
            append_modifier(recv, make_modifier($swift_name, args))
        }
    };
}

chart_modifier!(modifier_chart_x_scale, "chartXScale");
chart_modifier!(modifier_chart_y_scale, "chartYScale");
chart_modifier!(
    modifier_chart_foreground_style_scale,
    "chartForegroundStyleScale"
);
chart_modifier!(modifier_chart_x_selection, "chartXSelection");

/// Evaluate trailing builder content (closures / view values) into a single
/// child (or ZStack of several) and push it as an unlabeled `value` arg.
fn push_collected_content(
    ctx: &mut dyn StdContext,
    meta: &mut Vec<Arg>,
    content_args: Vec<Arg>,
) -> Result<(), StdError> {
    if content_args.is_empty() {
        return Ok(());
    }
    let views = collect_children(ctx, content_args)?;
    let content = match views.len() {
        0 => None,
        1 => Some(views.into_iter().next().expect("len checked")),
        _ => Some(container_value("ZStack", views)),
    };
    if let Some(content) = content {
        meta.push(Arg {
            label: None,
            value: content,
            static_ty: None,
        });
    }
    Ok(())
}

/// Collapse already-expanded view values into a single unlabeled `value` arg.
fn push_views_as_value(meta: &mut Vec<Arg>, views: Vec<SwiftValue>) {
    let content = match views.len() {
        0 => None,
        1 => Some(views.into_iter().next().expect("len checked")),
        _ => Some(container_value("ZStack", views)),
    };
    if let Some(content) = content {
        meta.push(Arg {
            label: None,
            value: content,
            static_ty: None,
        });
    }
}

/// `.chartXAxis { AxisMarks… }` / `.chartXAxis(.hidden)` — builder content is
/// collected into the modifier's `value`; visibility token is stored as-is.
fn modifier_chart_x_axis(ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
    chart_axis_modifier(ctx, recv, "chartXAxis", args)
}

/// `.chartYAxis { AxisMarks… }` / `.chartYAxis(.hidden)`.
fn modifier_chart_y_axis(ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
    chart_axis_modifier(ctx, recv, "chartYAxis", args)
}

fn chart_axis_modifier(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    name: &str,
    args: Vec<Arg>,
) -> StdResult {
    let mut meta: Vec<Arg> = Vec::new();
    let mut content_args: Vec<Arg> = Vec::new();
    for arg in args {
        match &arg.value {
            // Trailing `@AxisContentBuilder` closure.
            SwiftValue::Closure(_) => content_args.push(arg),
            // Visibility form `.chartXAxis(.hidden)` or direct axis-content view.
            other => {
                // A direct axis-content view value (rare) is still content.
                if arg.label.is_none() && is_axis_content_view(other) {
                    content_args.push(arg);
                } else {
                    meta.push(arg);
                }
            }
        }
    }
    push_collected_content(ctx, &mut meta, content_args)?;
    append_modifier(recv, make_modifier(name, meta))
}

fn is_axis_content_view(value: &SwiftValue) -> bool {
    matches!(
        value,
        SwiftValue::Struct(obj)
            if matches!(
                obj.type_name.as_str(),
                "AxisMarks" | "AxisGridLine" | "AxisTick" | "AxisValueLabel"
            )
    )
}

/// `.chartLegend(.hidden)` / `.chartLegend(position: .top)` /
/// `.chartLegend { Text(…) }` — visibility/position args stored as-is; builder
/// content is child-collected into the modifier `value` (never a raw Closure).
fn modifier_chart_legend(ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
    let mut meta: Vec<Arg> = Vec::new();
    let mut content_args: Vec<Arg> = Vec::new();
    for arg in args {
        match arg.label.as_deref() {
            Some("position") | Some("alignment") | Some("spacing") => meta.push(arg),
            Some("content") => content_args.push(Arg {
                label: None,
                value: arg.value,
                static_ty: None,
            }),
            None => match &arg.value {
                SwiftValue::Closure(_) => content_args.push(arg),
                // Visibility token form `.chartLegend(.hidden)`.
                _ => meta.push(arg),
            },
            _ => meta.push(arg),
        }
    }
    push_collected_content(ctx, &mut meta, content_args)?;
    append_modifier(recv, make_modifier("chartLegend", meta))
}

/// `.chartXAxisLabel("X")` / `.chartXAxisLabel { Text("X") }`.
fn modifier_chart_x_axis_label(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> StdResult {
    chart_axis_label_modifier(ctx, recv, "chartXAxisLabel", args)
}

/// `.chartYAxisLabel("Y")` / `.chartYAxisLabel { Text("Y") }`.
fn modifier_chart_y_axis_label(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> StdResult {
    chart_axis_label_modifier(ctx, recv, "chartYAxisLabel", args)
}

fn chart_axis_label_modifier(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    name: &str,
    args: Vec<Arg>,
) -> StdResult {
    let mut meta: Vec<Arg> = Vec::new();
    let mut content_args: Vec<Arg> = Vec::new();
    for arg in args {
        match arg.label.as_deref() {
            Some("position") | Some("alignment") | Some("spacing") => meta.push(arg),
            Some("content") => content_args.push(Arg {
                label: None,
                value: arg.value,
                static_ty: None,
            }),
            None => match &arg.value {
                // Trailing `@ViewBuilder` form.
                SwiftValue::Closure(_) => content_args.push(arg),
                // String (or other scalar) label form `.chartXAxisLabel("X")`.
                _ => meta.push(arg),
            },
            _ => meta.push(arg),
        }
    }
    push_collected_content(ctx, &mut meta, content_args)?;
    append_modifier(recv, make_modifier(name, meta))
}

/// Placeholder plot-area view passed into `.chartPlotStyle { plotArea in … }`.
/// Real Charts uses `ChartPlotContent`; v1 is an ordinary view value so the
/// closure can apply SwiftUI modifiers (`.background`, …) and return them.
fn chart_plot_content_placeholder() -> SwiftValue {
    view_value("ChartPlotContent", vec![])
}

/// Structured marker when a plot-style closure cannot be expanded to children
/// (keeps the `_Modifier` free of raw `Closure` / `(Function)` values).
fn chart_plot_style_marker() -> SwiftValue {
    view_value("ChartPlotStyleContent", vec![])
}

/// `.chartPlotStyle { plotArea in … }` — invoke the param closure with a
/// placeholder `ChartPlotContent` (AsyncImage/ForEach pattern via
/// `eval_block_values_with_args`), then expand the result into a child view
/// stored on the modifier. Falls back to a structured marker (never a raw
/// Closure) if invocation yields nothing.
fn modifier_chart_plot_style(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> StdResult {
    let mut meta: Vec<Arg> = Vec::new();
    let mut content_args: Vec<Arg> = Vec::new();
    for arg in args {
        match arg.label.as_deref() {
            Some("content") | None => content_args.push(Arg {
                label: None,
                value: arg.value,
                static_ty: None,
            }),
            _ => meta.push(arg),
        }
    }

    let mut views: Vec<SwiftValue> = Vec::new();
    for arg in content_args {
        match arg.value {
            SwiftValue::Closure(id) => {
                // Param form: `{ plotArea in plotArea.background(...) }`.
                // Mirror AsyncImage / ForEach — bind a placeholder view arg.
                match ctx.eval_block_values_with_args(id, vec![chart_plot_content_placeholder()]) {
                    Ok(produced) => {
                        if let Ok(kids) = collect_children(
                            ctx,
                            vec![Arg {
                                label: None,
                                value: produced,
                                static_ty: None,
                            }],
                        ) {
                            if !kids.is_empty() {
                                views.extend(kids);
                                continue;
                            }
                        }
                        // Invocation ran but produced no view children.
                        views.push(chart_plot_style_marker());
                    }
                    Err(_) => {
                        // Cannot bind / evaluate — structured marker, not Closure.
                        views.push(chart_plot_style_marker());
                    }
                }
            }
            other => {
                // Already a view value (or zero-arg builder result pre-expanded).
                if let Ok(kids) = collect_children(
                    ctx,
                    vec![Arg {
                        label: None,
                        value: other,
                        static_ty: None,
                    }],
                ) {
                    views.extend(kids);
                }
            }
        }
    }
    if views.is_empty() {
        views.push(chart_plot_style_marker());
    }
    push_views_as_value(&mut meta, views);
    append_modifier(recv, make_modifier("chartPlotStyle", meta))
}

/// Chart-level modifiers registered as generic struct methods (on `Chart` /
/// any view value). Coverage keys are `View.<name>` (inventory owning type).
pub(crate) const CHART_MODIFIER_FNS: &[(&str, StructMethodFn)] = &[
    ("chartXAxis", modifier_chart_x_axis),
    ("chartYAxis", modifier_chart_y_axis),
    ("chartXAxisLabel", modifier_chart_x_axis_label),
    ("chartYAxisLabel", modifier_chart_y_axis_label),
    ("chartXScale", modifier_chart_x_scale),
    ("chartYScale", modifier_chart_y_scale),
    (
        "chartForegroundStyleScale",
        modifier_chart_foreground_style_scale,
    ),
    ("chartLegend", modifier_chart_legend),
    ("chartPlotStyle", modifier_chart_plot_style),
    ("chartXSelection", modifier_chart_x_selection),
];

// ── Install ─────────────────────────────────────────────────────────────────

/// Register mark + chart modifiers and typed parameter hints for leading-dot.
pub(crate) fn install(interp: &mut Interpreter<'_>) {
    for (name, func) in MARK_MODIFIER_FNS {
        interp.register_struct_method(name, *func);
    }
    for (name, func) in CHART_MODIFIER_FNS {
        interp.register_struct_method(name, *func);
    }

    // `.foregroundStyle(by: .value(...))` and positional ShapeStyle/Color.
    interp.register_struct_method_typed(
        "foregroundStyle",
        modifier_foreground_style,
        vec![
            BuiltinParam::positional("Color"),
            BuiltinParam::labeled("by", "PlottableValue"),
        ],
    );
    // `.symbol(.circle)` / `.symbol(by: .value(...))`.
    interp.register_struct_method_typed(
        "symbol",
        modifier_symbol,
        vec![
            BuiltinParam::positional("ChartSymbolShape"),
            BuiltinParam::labeled("by", "PlottableValue"),
        ],
    );
    // `.interpolationMethod(.catmullRom)` — disambiguate from `Animation.linear`.
    interp.register_struct_method_typed(
        "interpolationMethod",
        modifier_interpolation_method,
        vec![BuiltinParam::positional("InterpolationMethod")],
    );
    // `.annotation(position: .top) { … }` — disambiguate from `Alignment.top`.
    interp.register_struct_method_typed(
        "annotation",
        modifier_annotation,
        vec![
            BuiltinParam::labeled("position", "AnnotationPosition"),
            BuiltinParam::labeled("alignment", "Alignment"),
            BuiltinParam::labeled("spacing", "CGFloat"),
        ],
    );
    // `.position(by: .value(...))` for grouped positioning.
    interp.register_struct_method_typed(
        "position",
        modifier_position,
        vec![BuiltinParam::labeled("by", "PlottableValue")],
    );

    // Chart-level: `.chartXAxis(.hidden)` / `.chartLegend(.visible)`.
    interp.register_struct_method_typed(
        "chartXAxis",
        modifier_chart_x_axis,
        vec![BuiltinParam::positional("Visibility")],
    );
    interp.register_struct_method_typed(
        "chartYAxis",
        modifier_chart_y_axis,
        vec![BuiltinParam::positional("Visibility")],
    );
    interp.register_struct_method_typed(
        "chartLegend",
        modifier_chart_legend,
        vec![
            BuiltinParam::positional("Visibility"),
            BuiltinParam::labeled("position", "AnnotationPosition"),
            BuiltinParam::labeled("alignment", "Alignment"),
            BuiltinParam::labeled("spacing", "CGFloat"),
        ],
    );
    // `.chartXAxisLabel("X")` / `.chartYAxisLabel("Y")` — string label form.
    // No special typed hints required.
    // `.chartXSelection(value: $binding)` — Binding is resolved by `$` sugar.
}
