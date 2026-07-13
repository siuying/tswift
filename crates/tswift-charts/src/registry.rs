//! Registry wiring: [`install`] and coverage-key derivation via [`registered_keys`].

use tswift_core::{BuiltinParam, BuiltinReceiver, Interpreter};

use crate::axis::{axis_grid_line_init, axis_marks_init, axis_tick_init, axis_value_label_init};
use crate::marks::{
    area_mark_init, bar_mark_init, chart_init, line_mark_init, plottable_value_static,
    point_mark_init, rectangle_mark_init, rule_mark_init, sector_mark_init, xy_plottable_params,
};
use crate::modifiers;

/// Register every currently-supported Charts constructor into `interp`.
pub fn install(interp: &mut Interpreter<'_>) {
    // `Chart { … }` — trailing content-builder closure becomes `_children`
    // (same shape as SwiftUI containers).
    interp.register_free_fn("Chart", chart_init);
    // Mark constructors — typed so leading-dot `.value(...)` resolves against
    // `PlottableValue` where the arg is plottable (see PRELUDE). Non-plottable
    // args (width/height/innerRadius/angularInset) accept CGFloat/Any generically.
    interp.register_free_fn_typed("BarMark", bar_mark_init, xy_plottable_params());
    interp.register_free_fn_typed("LineMark", line_mark_init, xy_plottable_params());
    interp.register_free_fn_typed("PointMark", point_mark_init, xy_plottable_params());
    interp.register_free_fn_typed("AreaMark", area_mark_init, xy_plottable_params());
    // RuleMark has several labeled forms (x: / y: / xStart:xEnd:y: / yStart:yEnd:x:).
    // Register every common label as PlottableValue; the init stores whatever arrives.
    interp.register_free_fn_typed(
        "RuleMark",
        rule_mark_init,
        vec![
            BuiltinParam::labeled("x", "PlottableValue"),
            BuiltinParam::labeled("y", "PlottableValue"),
            BuiltinParam::labeled("xStart", "PlottableValue"),
            BuiltinParam::labeled("xEnd", "PlottableValue"),
            BuiltinParam::labeled("yStart", "PlottableValue"),
            BuiltinParam::labeled("yEnd", "PlottableValue"),
        ],
    );
    interp.register_free_fn_typed(
        "RectangleMark",
        rectangle_mark_init,
        vec![
            BuiltinParam::labeled("x", "PlottableValue"),
            BuiltinParam::labeled("y", "PlottableValue"),
            BuiltinParam::labeled("width", "CGFloat"),
            BuiltinParam::labeled("height", "CGFloat"),
        ],
    );
    interp.register_free_fn_typed(
        "SectorMark",
        sector_mark_init,
        vec![
            BuiltinParam::labeled("angle", "PlottableValue"),
            BuiltinParam::labeled("innerRadius", "CGFloat"),
            BuiltinParam::labeled("angularInset", "CGFloat"),
        ],
    );
    // Axis-content leaves used inside `.chartXAxis { … }` / `.chartYAxis { … }`.
    // Produce view_value nodes so `collect_children` captures them as children
    // of the chart-axis modifier (and of AxisMarks when nested in its builder).
    interp.register_free_fn("AxisMarks", axis_marks_init);
    interp.register_free_fn("AxisGridLine", axis_grid_line_init);
    interp.register_free_fn("AxisTick", axis_tick_init);
    interp.register_free_fn("AxisValueLabel", axis_value_label_init);
    // Also expose `PlottableValue.value` as a Rust static intrinsic so
    // `PlottableValue.value(...)` (qualified) works without the prelude and
    // so coverage sees a live registry key. Leading-dot still needs PRELUDE
    // (resolve_implicit_static_method only sees user-declared statics).
    let plottable = BuiltinReceiver::register_extension("PlottableValue");
    interp.register_static(plottable, "value", plottable_value_static);
    // ChartContent mark modifiers + Chart-level View modifiers.
    modifiers::install(interp);
}

/// Free-fn mark / container / axis-content names that become `Type.init`
/// coverage keys.
const INIT_NAMES: &[&str] = &[
    "AreaMark",
    "AxisGridLine",
    "AxisMarks",
    "AxisTick",
    "AxisValueLabel",
    "BarMark",
    "Chart",
    "LineMark",
    "PointMark",
    "RectangleMark",
    "RuleMark",
    "SectorMark",
];

/// Every Charts entry registered by [`install`], as coverage keys
/// (`Type.member`, matching `tools/framework-inventory/coverage.py`).
pub fn registered_keys() -> Vec<String> {
    let mut sink = std::io::sink();
    let mut interp = Interpreter::new(&mut sink);
    install(&mut interp);
    let mut keys: Vec<String> = interp
        .registered_keys()
        .into_iter()
        .filter_map(|key| {
            if INIT_NAMES.contains(&key.as_str()) {
                Some(format!("{key}.init"))
            } else if key == "PlottableValue.value" {
                Some(key)
            } else {
                None
            }
        })
        .collect();
    // PRELUDE `PlottableValue.value` is the call path for leading-dot; the
    // Rust static is registered for qualified calls + coverage. Ensure the
    // coverage key is present even if registry filtering changes.
    if !keys.iter().any(|k| k == "PlottableValue.value") {
        keys.push("PlottableValue.value".into());
    }
    // Mark modifiers are members of the `ChartContent` protocol in the SDK
    // inventory (not `View.*` — that key belongs to SwiftUI coverage).
    keys.extend(
        modifiers::MARK_MODIFIER_FNS
            .iter()
            .map(|(m, _)| format!("ChartContent.{m}")),
    );
    // Chart-level modifiers are members of `View` in the Charts inventory
    // (Charts extends SwiftUICore.View).
    keys.extend(
        modifiers::CHART_MODIFIER_FNS
            .iter()
            .map(|(m, _)| format!("View.{m}")),
    );
    keys.sort();
    keys.dedup();
    keys
}
