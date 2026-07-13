//! tswift-charts — Swift Charts view primitives as runtime builtins.
//!
//! Charts is a **render-host framework** like SwiftUI: `Chart { … }` is a
//! container view and marks (`BarMark`, …) are content-builder children that
//! become view values in the **same** UIIR tree SwiftUI produces. Hosts that
//! already render SwiftUI UIIR can later special-case `kind: "Chart"` /
//! `"BarMark"` without a separate IR. See `notes.md` (Charts autoloop).
//!
//! This crate mirrors the `tswift-swiftui` registry seam: [`install`] wires
//! constructors into an interpreter, and [`registered_keys`] exposes the live
//! registry to the framework-inventory coverage tooling.

mod modifiers;

use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinParam, BuiltinReceiver, Interpreter, StdContext, StdResult, StructObj, SwiftValue,
};
use tswift_swiftui::{collect_children, container_value, view_value};

/// Swift prelude for Charts value types that need leading-dot static methods.
///
/// `PlottableValue.value(_:_:)` is declared in Swift (like SwiftUI's
/// `GridItem.flexible`) so `.value("Label", datum)` resolves via the
/// interpreter's implicit-static-method path when `BarMark(x:y:)` pushes a
/// `PlottableValue` contextual type. Prepend this ahead of user source the
/// same way hosts prepend `tswift_swiftui::PRELUDE`.
///
/// Token namespaces (`InterpolationMethod`, `ChartSymbolShape`,
/// `AnnotationPosition`) mirror SwiftUI's `Color`/`Font` pattern so leading-dot
/// forms resolve under typed mark-modifier parameter hints.
pub const PRELUDE: &str = r#"
// `PlottableValue` — label + plottable datum pair used as mark x/y args
// (`BarMark(x: .value("Name", "A"), y: .value("Count", 3))`). Real Charts is
// generic over `Plottable`; v1 stores the datum as `Any`.
struct PlottableValue {
    let label: String
    let value: Any
    static func value(_ label: String, _ value: Any) -> PlottableValue {
        PlottableValue(label: label, value: value)
    }
}
// Line/area interpolation token (`.interpolationMethod(.catmullRom)`).
struct InterpolationMethod {
    let token: String
    static let linear = InterpolationMethod(token: "linear")
    static let catmullRom = InterpolationMethod(token: "catmullRom")
    static let monotone = InterpolationMethod(token: "monotone")
    static let cardinal = InterpolationMethod(token: "cardinal")
    static let stepStart = InterpolationMethod(token: "stepStart")
    static let stepCenter = InterpolationMethod(token: "stepCenter")
    static let stepEnd = InterpolationMethod(token: "stepEnd")
}
// Point symbol shape token (`.symbol(.circle)`).
struct ChartSymbolShape {
    let token: String
    static let circle = ChartSymbolShape(token: "circle")
    static let square = ChartSymbolShape(token: "square")
    static let diamond = ChartSymbolShape(token: "diamond")
    static let triangle = ChartSymbolShape(token: "triangle")
    static let cross = ChartSymbolShape(token: "cross")
    static let plus = ChartSymbolShape(token: "plus")
    static let asterisk = ChartSymbolShape(token: "asterisk")
    static let pentagon = ChartSymbolShape(token: "pentagon")
}
// Annotation placement token (`.annotation(position: .top) { … }`).
struct AnnotationPosition {
    let token: String
    static let automatic = AnnotationPosition(token: "automatic")
    static let top = AnnotationPosition(token: "top")
    static let bottom = AnnotationPosition(token: "bottom")
    static let leading = AnnotationPosition(token: "leading")
    static let trailing = AnnotationPosition(token: "trailing")
    static let overlay = AnnotationPosition(token: "overlay")
    static let topLeading = AnnotationPosition(token: "topLeading")
    static let topTrailing = AnnotationPosition(token: "topTrailing")
    static let bottomLeading = AnnotationPosition(token: "bottomLeading")
    static let bottomTrailing = AnnotationPosition(token: "bottomTrailing")
}
// Minimal `StrokeStyle` so `.lineStyle(StrokeStyle(lineWidth: 2))` stores args.
struct StrokeStyle {
    let lineWidth: Double
    init(lineWidth: Double = 1.0) { self.lineWidth = lineWidth }
}
// Axis / legend visibility token (`.chartXAxis(.hidden)`, `.chartLegend(.visible)`).
// Real SwiftUI `Visibility` lives in SwiftUICore; Charts reuses it. v1 is a
// Charts-local token so leading-dot resolves under chart-modifier type hints.
struct Visibility {
    let token: String
    static let automatic = Visibility(token: "automatic")
    static let visible = Visibility(token: "visible")
    static let hidden = Visibility(token: "hidden")
}
"#;

/// PlottableValue-typed x/y params shared by Bar/Line/Point/Area marks.
fn xy_plottable_params() -> Vec<BuiltinParam> {
    vec![
        BuiltinParam::labeled("x", "PlottableValue"),
        BuiltinParam::labeled("y", "PlottableValue"),
    ]
}

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

/// `Chart { marks… }` — container view collecting content-builder children.
fn chart_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    Ok(container_value("Chart", collect_children(ctx, args)?))
}

/// Build a mark leaf view from the subset of `wanted` labels present in `args`
/// (order follows `wanted`, matching BarMark's x-then-y field construction).
fn mark_leaf(kind: &str, args: Vec<Arg>, wanted: &[&str]) -> StdResult {
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
fn mark_leaf_all_labeled(kind: &str, args: Vec<Arg>) -> StdResult {
    let mut fields = Vec::new();
    for arg in args {
        if let Some(label) = arg.label {
            fields.push((label, arg.value));
        }
    }
    Ok(view_value(kind, fields))
}

/// `BarMark(x:y:)` — mark leaf carrying its plottable x/y args.
fn bar_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf("BarMark", args, &["x", "y"])
}

/// `LineMark(x:y:)` — line series mark.
fn line_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf("LineMark", args, &["x", "y"])
}

/// `PointMark(x:y:)` — scatter/point mark.
fn point_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf("PointMark", args, &["x", "y"])
}

/// `AreaMark(x:y:)` — filled area under a series.
fn area_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf("AreaMark", args, &["x", "y"])
}

/// `RuleMark(...)` — store whatever labeled PlottableValue args are passed
/// (`x:`, `y:`, `xStart:xEnd:y:`, `yStart:yEnd:x:`, …).
fn rule_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf_all_labeled("RuleMark", args)
}

/// `RectangleMark(x:y:width:height:)` — store provided args.
fn rectangle_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf("RectangleMark", args, &["x", "y", "width", "height"])
}

/// `SectorMark(angle:innerRadius:angularInset:)` — pie/donut sector.
fn sector_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf(
        "SectorMark",
        args,
        &["angle", "innerRadius", "angularInset"],
    )
}

// ── Axis content ────────────────────────────────────────────────────────────

/// `AxisMarks(...)` / `AxisMarks { … }` / `AxisMarks(values:) { … }` —
/// axis-content view. Labeled args (`values`, `preset`, `position`, …) become
/// fields; trailing `@AxisMarkBuilder` content becomes `_children`.
fn axis_marks_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
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
fn axis_grid_line_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf_all_labeled("AxisGridLine", args)
}

/// `AxisTick()` — axis-content leaf (tick marks).
fn axis_tick_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    mark_leaf_all_labeled("AxisTick", args)
}

/// `AxisValueLabel()` / `AxisValueLabel("title")` / `AxisValueLabel { Text… }` —
/// axis-content leaf. String title stays a field; trailing `@ViewBuilder`
/// content is child-collected into `_children` (never a raw Closure).
fn axis_value_label_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
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

/// `PlottableValue.value(_ label:, _ value:)` — Rust intrinsic fallback used
/// for qualified `PlottableValue.value(...)` calls and coverage registration.
fn plottable_value_static(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
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

#[cfg(test)]
mod coverage_dump {
    #[test]
    fn dump_registered_keys() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join("frameworks/charts/registered_keys.txt");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let body = super::registered_keys().join("\n") + "\n";
        std::fs::write(&path, body).expect("write registered_keys.txt");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;
    use tswift_core::{Interpreter, StructObj, SwiftValue};
    use tswift_swiftui::{
        render_root, uiir, view_type_name, CHILDREN_FIELD, MODIFIERS_FIELD,
        PRELUDE as SWIFTUI_PRELUDE,
    };

    /// Same prelude stack hosts use (`tswift-cli` / `tswift-wasm` prepare):
    /// SwiftUI PRELUDE + SwiftData QUERY_PRELUDE + Charts PRELUDE + user source.
    /// Do not invent a different composition here — hosts must stay in lockstep.
    fn host_program(user: &str) -> String {
        format!(
            "{SWIFTUI_PRELUDE}\n{}\n{PRELUDE}\n{user}\n",
            tswift_swiftdata::QUERY_PRELUDE,
        )
    }

    /// Analyze + run `src` with the host prelude stack and SwiftUI + Charts installed.
    fn with_interp<R>(src: &str, f: impl FnOnce(&mut Interpreter) -> R) -> R {
        let program = host_program(src);
        let analysis =
            tswift_frontend::Analysis::analyze(&program, "charts_test.swift").expect("analyze");
        let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        tswift_std::install(&mut interp);
        tswift_swiftui::install(&mut interp);
        install(&mut interp);
        interp.run(analysis).expect("run");
        f(&mut interp)
    }

    #[test]
    fn registered_keys_cover_core_marks_and_modifiers() {
        let keys = registered_keys();
        for expected in [
            "AreaMark.init",
            "AxisGridLine.init",
            "AxisMarks.init",
            "AxisTick.init",
            "AxisValueLabel.init",
            "BarMark.init",
            "Chart.init",
            "ChartContent.accessibilityHidden",
            "ChartContent.accessibilityIdentifier",
            "ChartContent.accessibilityLabel",
            "ChartContent.accessibilityValue",
            "ChartContent.alignsMarkStylesWithPlotArea",
            "ChartContent.annotation",
            "ChartContent.blur",
            "ChartContent.clipShape",
            "ChartContent.compositingLayer",
            "ChartContent.cornerRadius",
            "ChartContent.foregroundStyle",
            "ChartContent.interpolationMethod",
            "ChartContent.lineStyle",
            "ChartContent.mask",
            "ChartContent.offset",
            "ChartContent.opacity",
            "ChartContent.position",
            "ChartContent.shadow",
            "ChartContent.symbol",
            "ChartContent.symbolSize",
            "ChartContent.zIndex",
            "LineMark.init",
            "PlottableValue.value",
            "PointMark.init",
            "RectangleMark.init",
            "RuleMark.init",
            "SectorMark.init",
            "View.chartAngleSelection",
            "View.chartBackground",
            "View.chartForegroundStyleScale",
            "View.chartLegend",
            "View.chartLineStyleScale",
            "View.chartOverlay",
            "View.chartPlotStyle",
            "View.chartScrollPosition",
            "View.chartScrollTargetBehavior",
            "View.chartScrollableAxes",
            "View.chartSymbolScale",
            "View.chartSymbolSizeScale",
            "View.chartXAxis",
            "View.chartXAxisLabel",
            "View.chartXAxisStyle",
            "View.chartXScale",
            "View.chartXSelection",
            "View.chartXVisibleDomain",
            "View.chartYAxis",
            "View.chartYAxisLabel",
            "View.chartYAxisStyle",
            "View.chartYScale",
            "View.chartYSelection",
            "View.chartYVisibleDomain",
        ] {
            assert!(
                keys.iter().any(|k| k == expected),
                "missing coverage key {expected}; keys={keys:?}"
            );
        }
    }

    /// First `_Modifier` on a mark with the given `name`, or panic.
    fn mark_modifier<'a>(mark: &'a StructObj, name: &str) -> &'a StructObj {
        let Some(SwiftValue::Array(mods)) = mark.get(MODIFIERS_FIELD) else {
            panic!("expected _modifiers on mark");
        };
        for m in mods.iter() {
            let SwiftValue::Struct(obj) = m else {
                continue;
            };
            if obj.get("name") == Some(&SwiftValue::Str(name.into())) {
                return obj;
            }
        }
        panic!("modifier `{name}` not found in {:?}", mods);
    }

    fn assert_has_modifier(mark: &StructObj, name: &str) {
        let _ = mark_modifier(mark, name);
    }

    /// Assert `Chart { mark }` has one child of `kind` and returns that child struct.
    fn chart_single_mark(interp: &mut Interpreter, root: &str, kind: &str) -> Rc<StructObj> {
        let view = render_root(interp, root).expect("render");
        assert_eq!(view_type_name(&view), Some("Chart"));
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected Chart struct");
        };
        let Some(SwiftValue::Array(children)) = obj.get(CHILDREN_FIELD) else {
            panic!("expected Chart _children");
        };
        assert_eq!(children.len(), 1, "Chart should have one {kind} child");
        assert_eq!(view_type_name(&children[0]), Some(kind));
        let SwiftValue::Struct(mark) = &children[0] else {
            panic!("expected {kind} struct");
        };
        Rc::clone(mark)
    }

    fn assert_plottable(mark: &StructObj, field: &str, label: &str, value: SwiftValue) {
        let Some(SwiftValue::Struct(pv)) = mark.get(field) else {
            panic!("expected {field} PlottableValue, got {:?}", mark.get(field));
        };
        assert_eq!(pv.type_name, "PlottableValue");
        assert_eq!(pv.get("label"), Some(&SwiftValue::Str(label.into())));
        assert_eq!(pv.get("value"), Some(&value));
    }

    fn assert_uiir_kinds(view: &SwiftValue, kinds: &[&str]) {
        let json = uiir::to_json(view);
        for kind in kinds {
            let needle = format!(r#""kind":"{kind}""#);
            assert!(json.contains(&needle), "UIIR missing {kind}: {json}");
        }
    }

    #[test]
    fn chart_bar_mark_renders_into_uiir() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("Name", "A"), y: .value("Count", 3))
        }
    }
}
"#;
        with_interp(src, |interp| {
            let mark = chart_single_mark(interp, "Demo", "BarMark");
            assert_plottable(&mark, "x", "Name", SwiftValue::Str("A".into()));
            assert_plottable(&mark, "y", "Count", SwiftValue::int(3));
            let view = render_root(interp, "Demo").expect("render");
            assert_uiir_kinds(&view, &["Chart", "BarMark"]);
            let json = uiir::to_json(&view);
            assert!(
                json.contains("Name") && json.contains("Count"),
                "UIIR missing labels: {json}"
            );
        });
    }

    #[test]
    fn chart_line_mark_renders_into_uiir() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            LineMark(x: .value("Day", "Mon"), y: .value("Sales", 10))
        }
    }
}
"#;
        with_interp(src, |interp| {
            let mark = chart_single_mark(interp, "Demo", "LineMark");
            assert_plottable(&mark, "x", "Day", SwiftValue::Str("Mon".into()));
            assert_plottable(&mark, "y", "Sales", SwiftValue::int(10));
            let view = render_root(interp, "Demo").expect("render");
            assert_uiir_kinds(&view, &["Chart", "LineMark"]);
        });
    }

    #[test]
    fn chart_point_mark_renders_into_uiir() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            PointMark(x: .value("X", 1), y: .value("Y", 2))
        }
    }
}
"#;
        with_interp(src, |interp| {
            let mark = chart_single_mark(interp, "Demo", "PointMark");
            assert_plottable(&mark, "x", "X", SwiftValue::int(1));
            assert_plottable(&mark, "y", "Y", SwiftValue::int(2));
            let view = render_root(interp, "Demo").expect("render");
            assert_uiir_kinds(&view, &["Chart", "PointMark"]);
        });
    }

    #[test]
    fn chart_area_mark_renders_into_uiir() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            AreaMark(x: .value("T", "a"), y: .value("V", 5))
        }
    }
}
"#;
        with_interp(src, |interp| {
            let mark = chart_single_mark(interp, "Demo", "AreaMark");
            assert_plottable(&mark, "x", "T", SwiftValue::Str("a".into()));
            assert_plottable(&mark, "y", "V", SwiftValue::int(5));
            let view = render_root(interp, "Demo").expect("render");
            assert_uiir_kinds(&view, &["Chart", "AreaMark"]);
        });
    }

    #[test]
    fn chart_rule_mark_x_and_range_forms() {
        // RuleMark(x:) vertical rule
        let src_x = r#"
struct DemoX: View {
    var body: some View {
        Chart {
            RuleMark(x: .value("Threshold", 50))
        }
    }
}
"#;
        with_interp(src_x, |interp| {
            let mark = chart_single_mark(interp, "DemoX", "RuleMark");
            assert_plottable(&mark, "x", "Threshold", SwiftValue::int(50));
            assert!(mark.get("y").is_none());
            let view = render_root(interp, "DemoX").expect("render");
            assert_uiir_kinds(&view, &["Chart", "RuleMark"]);
        });

        // RuleMark(y:) horizontal rule
        let src_y = r#"
struct DemoY: View {
    var body: some View {
        Chart {
            RuleMark(y: .value("Baseline", 0))
        }
    }
}
"#;
        with_interp(src_y, |interp| {
            let mark = chart_single_mark(interp, "DemoY", "RuleMark");
            assert_plottable(&mark, "y", "Baseline", SwiftValue::int(0));
        });

        // RuleMark(xStart:xEnd:y:) horizontal segment
        let src_seg = r#"
struct DemoSeg: View {
    var body: some View {
        Chart {
            RuleMark(
                xStart: .value("From", 1),
                xEnd: .value("To", 4),
                y: .value("Band", "A")
            )
        }
    }
}
"#;
        with_interp(src_seg, |interp| {
            let mark = chart_single_mark(interp, "DemoSeg", "RuleMark");
            assert_plottable(&mark, "xStart", "From", SwiftValue::int(1));
            assert_plottable(&mark, "xEnd", "To", SwiftValue::int(4));
            assert_plottable(&mark, "y", "Band", SwiftValue::Str("A".into()));
        });

        // RuleMark(yStart:yEnd:x:) vertical segment
        let src_vseg = r#"
struct DemoV: View {
    var body: some View {
        Chart {
            RuleMark(
                yStart: .value("Low", 0),
                yEnd: .value("High", 10),
                x: .value("Cat", "B")
            )
        }
    }
}
"#;
        with_interp(src_vseg, |interp| {
            let mark = chart_single_mark(interp, "DemoV", "RuleMark");
            assert_plottable(&mark, "yStart", "Low", SwiftValue::int(0));
            assert_plottable(&mark, "yEnd", "High", SwiftValue::int(10));
            assert_plottable(&mark, "x", "Cat", SwiftValue::Str("B".into()));
        });
    }

    #[test]
    fn chart_rectangle_mark_renders_into_uiir() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            RectangleMark(
                x: .value("X", 1),
                y: .value("Y", 2),
                width: 8,
                height: 12
            )
        }
    }
}
"#;
        with_interp(src, |interp| {
            let mark = chart_single_mark(interp, "Demo", "RectangleMark");
            assert_plottable(&mark, "x", "X", SwiftValue::int(1));
            assert_plottable(&mark, "y", "Y", SwiftValue::int(2));
            assert_eq!(mark.get("width"), Some(&SwiftValue::int(8)));
            assert_eq!(mark.get("height"), Some(&SwiftValue::int(12)));
            let view = render_root(interp, "Demo").expect("render");
            assert_uiir_kinds(&view, &["Chart", "RectangleMark"]);
        });
    }

    #[test]
    fn chart_sector_mark_renders_into_uiir() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            SectorMark(
                angle: .value("Share", 40),
                innerRadius: 20,
                angularInset: 2
            )
        }
    }
}
"#;
        with_interp(src, |interp| {
            let mark = chart_single_mark(interp, "Demo", "SectorMark");
            assert_plottable(&mark, "angle", "Share", SwiftValue::int(40));
            assert_eq!(mark.get("innerRadius"), Some(&SwiftValue::int(20)));
            assert_eq!(mark.get("angularInset"), Some(&SwiftValue::int(2)));
            let view = render_root(interp, "Demo").expect("render");
            assert_uiir_kinds(&view, &["Chart", "SectorMark"]);
        });
    }

    #[test]
    fn chart_multi_mark_line_and_point() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            LineMark(x: .value("X", 1), y: .value("Y", 2))
            PointMark(x: .value("X", 1), y: .value("Y", 2))
        }
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            assert_eq!(view_type_name(&view), Some("Chart"));
            let SwiftValue::Struct(obj) = &view else {
                panic!("expected Chart struct");
            };
            let Some(SwiftValue::Array(children)) = obj.get(CHILDREN_FIELD) else {
                panic!("expected Chart _children");
            };
            assert_eq!(children.len(), 2, "Chart should have LineMark + PointMark");
            assert_eq!(view_type_name(&children[0]), Some("LineMark"));
            assert_eq!(view_type_name(&children[1]), Some("PointMark"));
            assert_uiir_kinds(&view, &["Chart", "LineMark", "PointMark"]);
        });
    }

    // ── Slice 3: mark modifiers → `_Modifier` records on the mark ───────────

    #[test]
    fn mark_foreground_style_color_and_by() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
                .foregroundStyle(.red)
            BarMark(x: .value("N", "B"), y: .value("C", 2))
                .foregroundStyle(by: .value("Type", "x"))
        }
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            let SwiftValue::Struct(chart) = &view else {
                panic!("Chart");
            };
            let Some(SwiftValue::Array(children)) = chart.get(CHILDREN_FIELD) else {
                panic!("children");
            };
            assert_eq!(children.len(), 2);
            let SwiftValue::Struct(m0) = &children[0] else {
                panic!("mark0");
            };
            let mod0 = mark_modifier(m0, "foregroundStyle");
            let Some(SwiftValue::Struct(color)) = mod0.get("value") else {
                panic!("expected positional color, got {:?}", mod0);
            };
            assert_eq!(color.type_name, "Color");
            assert_eq!(color.get("token"), Some(&SwiftValue::Str("red".into())));

            let SwiftValue::Struct(m1) = &children[1] else {
                panic!("mark1");
            };
            let mod1 = mark_modifier(m1, "foregroundStyle");
            let Some(SwiftValue::Struct(by)) = mod1.get("by") else {
                panic!("expected by: PlottableValue, got {:?}", mod1);
            };
            assert_eq!(by.type_name, "PlottableValue");
            assert_eq!(by.get("label"), Some(&SwiftValue::Str("Type".into())));
        });
    }

    #[test]
    fn mark_symbol_and_symbol_size() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            PointMark(x: .value("X", 1), y: .value("Y", 2))
                .symbol(.circle)
                .symbolSize(40)
            PointMark(x: .value("X", 3), y: .value("Y", 4))
                .symbol(by: .value("Series", "A"))
        }
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            let SwiftValue::Struct(chart) = &view else {
                panic!("Chart");
            };
            let Some(SwiftValue::Array(children)) = chart.get(CHILDREN_FIELD) else {
                panic!("children");
            };
            let SwiftValue::Struct(m0) = &children[0] else {
                panic!("mark0");
            };
            let sym = mark_modifier(m0, "symbol");
            let Some(SwiftValue::Struct(shape)) = sym.get("value") else {
                panic!("symbol shape {:?}", sym);
            };
            assert_eq!(shape.type_name, "ChartSymbolShape");
            assert_eq!(shape.get("token"), Some(&SwiftValue::Str("circle".into())));
            let size = mark_modifier(m0, "symbolSize");
            assert_eq!(size.get("value"), Some(&SwiftValue::int(40)));

            let SwiftValue::Struct(m1) = &children[1] else {
                panic!("mark1");
            };
            let sym_by = mark_modifier(m1, "symbol");
            let Some(SwiftValue::Struct(by)) = sym_by.get("by") else {
                panic!("symbol by {:?}", sym_by);
            };
            assert_eq!(by.type_name, "PlottableValue");
        });
    }

    #[test]
    fn mark_line_style_and_interpolation() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            LineMark(x: .value("X", 1), y: .value("Y", 2))
                .lineStyle(StrokeStyle(lineWidth: 2))
                .interpolationMethod(.catmullRom)
        }
    }
}
"#;
        with_interp(src, |interp| {
            let mark = chart_single_mark(interp, "Demo", "LineMark");
            let ls = mark_modifier(&mark, "lineStyle");
            let Some(SwiftValue::Struct(stroke)) = ls.get("value") else {
                panic!("lineStyle value {:?}", ls);
            };
            assert_eq!(stroke.type_name, "StrokeStyle");
            assert_eq!(stroke.get("lineWidth"), Some(&SwiftValue::Double(2.0)));

            let im = mark_modifier(&mark, "interpolationMethod");
            let Some(SwiftValue::Struct(method)) = im.get("value") else {
                panic!("interpolation {:?}", im);
            };
            assert_eq!(method.type_name, "InterpolationMethod");
            assert_eq!(
                method.get("token"),
                Some(&SwiftValue::Str("catmullRom".into()))
            );
        });
    }

    #[test]
    fn mark_annotation_captures_content_child() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 3))
                .annotation(position: .top) {
                    Text("label")
                }
        }
    }
}
"#;
        with_interp(src, |interp| {
            let mark = chart_single_mark(interp, "Demo", "BarMark");
            let ann = mark_modifier(&mark, "annotation");
            let Some(SwiftValue::Struct(pos)) = ann.get("position") else {
                panic!("annotation position {:?}", ann);
            };
            assert_eq!(pos.type_name, "AnnotationPosition");
            assert_eq!(pos.get("token"), Some(&SwiftValue::Str("top".into())));
            // Content is the trailing @ViewBuilder child (stored as `value`).
            let Some(content) = ann.get("value") else {
                panic!("annotation missing content child: {:?}", ann);
            };
            assert_eq!(view_type_name(content), Some("Text"));
        });
    }

    #[test]
    fn mark_corner_radius_opacity_offset() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
                .cornerRadius(4)
                .opacity(0.5)
                .offset(x: 2, y: 3)
        }
    }
}
"#;
        with_interp(src, |interp| {
            let mark = chart_single_mark(interp, "Demo", "BarMark");
            let cr = mark_modifier(&mark, "cornerRadius");
            assert_eq!(cr.get("value"), Some(&SwiftValue::int(4)));
            let op = mark_modifier(&mark, "opacity");
            assert_eq!(op.get("value"), Some(&SwiftValue::Double(0.5)));
            let off = mark_modifier(&mark, "offset");
            assert_eq!(off.get("x"), Some(&SwiftValue::int(2)));
            assert_eq!(off.get("y"), Some(&SwiftValue::int(3)));
        });
    }

    #[test]
    fn mark_position_by() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
                .position(by: .value("Group", "g1"))
        }
    }
}
"#;
        with_interp(src, |interp| {
            let mark = chart_single_mark(interp, "Demo", "BarMark");
            let pos = mark_modifier(&mark, "position");
            let Some(SwiftValue::Struct(by)) = pos.get("by") else {
                panic!("position by {:?}", pos);
            };
            assert_eq!(by.type_name, "PlottableValue");
            assert_eq!(by.get("label"), Some(&SwiftValue::Str("Group".into())));
            assert_eq!(by.get("value"), Some(&SwiftValue::Str("g1".into())));
            assert_has_modifier(&mark, "position");
        });
    }

    // ── Slice 4: chart-level modifiers → `_Modifier` on Chart ───────────────

    /// First `_Modifier` on a chart/view with the given `name`, or panic.
    fn chart_modifier<'a>(chart: &'a StructObj, name: &str) -> &'a StructObj {
        mark_modifier(chart, name)
    }

    #[test]
    fn chart_x_axis_builder_captures_axis_marks() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartXAxis {
            AxisMarks()
        }
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            assert_eq!(view_type_name(&view), Some("Chart"));
            let SwiftValue::Struct(chart) = &view else {
                panic!("Chart");
            };
            let mod_ = chart_modifier(chart, "chartXAxis");
            // Builder content → AxisMarks child on the modifier `value`.
            let Some(content) = mod_.get("value") else {
                panic!("chartXAxis missing AxisMarks child: {:?}", mod_);
            };
            assert_eq!(view_type_name(content), Some("AxisMarks"));
        });
    }

    #[test]
    fn chart_y_axis_builder_and_hidden_visibility() {
        let src = r#"
struct DemoBuilder: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartYAxis {
            AxisMarks {
                AxisGridLine()
                AxisTick()
                AxisValueLabel()
            }
        }
    }
}
struct DemoHidden: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartXAxis(.hidden)
        .chartYAxis(.hidden)
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "DemoBuilder").expect("render");
            let SwiftValue::Struct(chart) = &view else {
                panic!("Chart");
            };
            let mod_ = chart_modifier(chart, "chartYAxis");
            let Some(content) = mod_.get("value") else {
                panic!("chartYAxis missing content: {:?}", mod_);
            };
            assert_eq!(view_type_name(content), Some("AxisMarks"));
            // Nested AxisMarkBuilder children on AxisMarks.
            let SwiftValue::Struct(marks) = content else {
                panic!("AxisMarks");
            };
            let Some(SwiftValue::Array(kids)) = marks.get(CHILDREN_FIELD) else {
                panic!("AxisMarks children: {:?}", marks);
            };
            assert_eq!(kids.len(), 3, "grid/tick/label");
            assert_eq!(view_type_name(&kids[0]), Some("AxisGridLine"));
            assert_eq!(view_type_name(&kids[1]), Some("AxisTick"));
            assert_eq!(view_type_name(&kids[2]), Some("AxisValueLabel"));

            let hidden = render_root(interp, "DemoHidden").expect("render");
            let SwiftValue::Struct(chart_h) = &hidden else {
                panic!("Chart");
            };
            let x = chart_modifier(chart_h, "chartXAxis");
            let Some(SwiftValue::Struct(vis)) = x.get("value") else {
                panic!("chartXAxis(.hidden) value {:?}", x);
            };
            assert_eq!(vis.type_name, "Visibility");
            assert_eq!(vis.get("token"), Some(&SwiftValue::Str("hidden".into())));
            let y = chart_modifier(chart_h, "chartYAxis");
            let Some(SwiftValue::Struct(vis_y)) = y.get("value") else {
                panic!("chartYAxis(.hidden)");
            };
            assert_eq!(vis_y.get("token"), Some(&SwiftValue::Str("hidden".into())));
        });
    }

    #[test]
    fn chart_axis_labels() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartXAxisLabel("Category")
        .chartYAxisLabel("Count")
    }
}
struct DemoBuilder: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartXAxisLabel {
            Text("x")
        }
        .chartYAxisLabel {
            Text("y")
        }
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            let SwiftValue::Struct(chart) = &view else {
                panic!("Chart");
            };
            let x = chart_modifier(chart, "chartXAxisLabel");
            assert_eq!(x.get("value"), Some(&SwiftValue::Str("Category".into())));
            let y = chart_modifier(chart, "chartYAxisLabel");
            assert_eq!(y.get("value"), Some(&SwiftValue::Str("Count".into())));

            // Builder forms collect Text children — never a raw Closure.
            let b = render_root(interp, "DemoBuilder").expect("render");
            let SwiftValue::Struct(chart_b) = &b else {
                panic!("Chart");
            };
            let xb = chart_modifier(chart_b, "chartXAxisLabel");
            let Some(x_content) = xb.get("value") else {
                panic!("chartXAxisLabel builder missing value: {:?}", xb);
            };
            assert!(
                !matches!(x_content, SwiftValue::Closure(_)),
                "builder must not store raw Closure: {:?}",
                x_content
            );
            assert_eq!(view_type_name(x_content), Some("Text"));
            let yb = chart_modifier(chart_b, "chartYAxisLabel");
            let Some(y_content) = yb.get("value") else {
                panic!("chartYAxisLabel builder missing value: {:?}", yb);
            };
            assert!(
                !matches!(y_content, SwiftValue::Closure(_)),
                "builder must not store raw Closure: {:?}",
                y_content
            );
            assert_eq!(view_type_name(y_content), Some("Text"));
        });
    }

    #[test]
    fn chart_y_scale_domain() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartYScale(domain: 0...100)
        .chartXScale(domain: ["A", "B", "C"])
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            let SwiftValue::Struct(chart) = &view else {
                panic!("Chart");
            };
            let y = chart_modifier(chart, "chartYScale");
            let Some(domain) = y.get("domain") else {
                panic!("chartYScale missing domain: {:?}", y);
            };
            match domain {
                SwiftValue::Range { lo, hi, inclusive } => {
                    assert_eq!(*lo, 0);
                    assert_eq!(*hi, 100);
                    assert!(*inclusive);
                }
                other => panic!("expected domain range, got {:?}", other),
            }
            let x = chart_modifier(chart, "chartXScale");
            let Some(SwiftValue::Array(domain)) = x.get("domain") else {
                panic!("chartXScale domain {:?}", x);
            };
            assert_eq!(domain.len(), 3);
            assert_eq!(domain[0], SwiftValue::Str("A".into()));
        });
    }

    #[test]
    fn chart_foreground_style_scale_mapping() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
                .foregroundStyle(by: .value("Type", "A"))
        }
        .chartForegroundStyleScale(["A": Color.red, "B": Color.blue])
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            let SwiftValue::Struct(chart) = &view else {
                panic!("Chart");
            };
            let mod_ = chart_modifier(chart, "chartForegroundStyleScale");
            // Dictionary / KeyValuePairs stored generically as unlabeled value.
            assert!(
                mod_.get("value").is_some() || mod_.fields.iter().any(|(k, _)| k != "name"),
                "chartForegroundStyleScale should store mapping: {:?}",
                mod_
            );
            // Prefer `value` (positional dict).
            if let Some(v) = mod_.get("value") {
                match v {
                    SwiftValue::Dict(_) | SwiftValue::Array(_) | SwiftValue::Struct(_) => {}
                    other => panic!("unexpected mapping shape {:?}", other),
                }
            }
        });
    }

    #[test]
    fn chart_legend_visibility_and_position() {
        let src = r#"
struct DemoHidden: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartLegend(.hidden)
    }
}
struct DemoPos: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartLegend(position: .top)
    }
}
struct DemoBuilder: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartLegend {
            Text("Legend")
        }
    }
}
"#;
        with_interp(src, |interp| {
            let h = render_root(interp, "DemoHidden").expect("render");
            let SwiftValue::Struct(chart) = &h else {
                panic!("Chart");
            };
            let mod_ = chart_modifier(chart, "chartLegend");
            let Some(SwiftValue::Struct(vis)) = mod_.get("value") else {
                panic!("chartLegend(.hidden) {:?}", mod_);
            };
            assert_eq!(vis.type_name, "Visibility");
            assert_eq!(vis.get("token"), Some(&SwiftValue::Str("hidden".into())));

            let p = render_root(interp, "DemoPos").expect("render");
            let SwiftValue::Struct(chart_p) = &p else {
                panic!("Chart");
            };
            let mod_p = chart_modifier(chart_p, "chartLegend");
            let Some(SwiftValue::Struct(pos)) = mod_p.get("position") else {
                panic!("chartLegend(position:) {:?}", mod_p);
            };
            assert_eq!(pos.type_name, "AnnotationPosition");
            assert_eq!(pos.get("token"), Some(&SwiftValue::Str("top".into())));

            // Builder form collects Text child — not a raw Closure.
            let b = render_root(interp, "DemoBuilder").expect("render");
            let SwiftValue::Struct(chart_b) = &b else {
                panic!("Chart");
            };
            let mod_b = chart_modifier(chart_b, "chartLegend");
            let Some(content) = mod_b.get("value") else {
                panic!("chartLegend builder missing value: {:?}", mod_b);
            };
            assert!(
                !matches!(content, SwiftValue::Closure(_)),
                "builder must not store raw Closure: {:?}",
                content
            );
            assert_eq!(view_type_name(content), Some("Text"));
        });
    }

    #[test]
    fn chart_plot_style_records_modifier() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartPlotStyle { plotArea in
            plotArea.background(Color.gray)
        }
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            let SwiftValue::Struct(chart) = &view else {
                panic!("Chart");
            };
            let mod_ = chart_modifier(chart, "chartPlotStyle");
            let Some(content) = mod_.get("value") else {
                panic!("chartPlotStyle missing value: {:?}", mod_);
            };
            // Must be a structured view/marker — never a raw Closure / (Function).
            assert!(
                !matches!(content, SwiftValue::Closure(_)),
                "chartPlotStyle must not store raw Closure: {:?}",
                content
            );
            let SwiftValue::Struct(content_obj) = content else {
                panic!("expected structured plot-style content, got {:?}", content);
            };
            // Prefer expanded ChartPlotContent (placeholder + .background);
            // ChartPlotStyleContent marker is the fallback.
            assert!(
                content_obj.type_name == "ChartPlotContent"
                    || content_obj.type_name == "ChartPlotStyleContent",
                "unexpected plot-style content type: {:?}",
                content_obj.type_name
            );
            if content_obj.type_name == "ChartPlotContent" {
                // Placeholder was invoked; .background should be on _modifiers.
                let Some(SwiftValue::Array(mods)) = content_obj.get(MODIFIERS_FIELD) else {
                    panic!("ChartPlotContent missing modifiers: {:?}", content_obj);
                };
                assert!(
                    mods.iter().any(|m| {
                        matches!(m, SwiftValue::Struct(o) if o.get("name")
                            == Some(&SwiftValue::Str("background".into())))
                    }),
                    "expected .background on plot content: {:?}",
                    mods
                );
            }
        });
    }

    #[test]
    fn axis_marks_values_and_value_label_builder() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartXAxis {
            AxisMarks(values: [0, 50, 100]) {
                AxisGridLine()
                AxisValueLabel {
                    Text("tick")
                }
            }
        }
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            let SwiftValue::Struct(chart) = &view else {
                panic!("Chart");
            };
            let mod_ = chart_modifier(chart, "chartXAxis");
            let Some(content) = mod_.get("value") else {
                panic!("chartXAxis missing content: {:?}", mod_);
            };
            assert_eq!(view_type_name(content), Some("AxisMarks"));
            let SwiftValue::Struct(marks) = content else {
                panic!("AxisMarks");
            };
            // values: stored as a field.
            let Some(SwiftValue::Array(vals)) = marks.get("values") else {
                panic!("AxisMarks(values:) missing values: {:?}", marks);
            };
            assert_eq!(vals.len(), 3, "values: [0, 50, 100]");
            // Nested builder children include AxisGridLine + AxisValueLabel.
            let Some(SwiftValue::Array(kids)) = marks.get(CHILDREN_FIELD) else {
                panic!("AxisMarks(values:) children missing: {:?}", marks);
            };
            assert_eq!(kids.len(), 2, "grid + value label: {:?}", kids);
            assert_eq!(view_type_name(&kids[0]), Some("AxisGridLine"));
            assert_eq!(view_type_name(&kids[1]), Some("AxisValueLabel"));
            // AxisValueLabel builder collected Text child.
            let SwiftValue::Struct(label) = &kids[1] else {
                panic!("AxisValueLabel");
            };
            let Some(SwiftValue::Array(label_kids)) = label.get(CHILDREN_FIELD) else {
                panic!("AxisValueLabel builder children: {:?}", label);
            };
            assert_eq!(label_kids.len(), 1);
            assert_eq!(view_type_name(&label_kids[0]), Some("Text"));
            assert!(
                !label
                    .fields
                    .iter()
                    .any(|(_, v)| matches!(v, SwiftValue::Closure(_))),
                "AxisValueLabel must not store raw Closure: {:?}",
                label
            );
        });
    }

    #[test]
    fn chart_x_selection_captures_binding() {
        let src = r#"
struct Demo: View {
    @State private var selected: String? = nil
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartXSelection(value: $selected)
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            let SwiftValue::Struct(chart) = &view else {
                panic!("Chart");
            };
            let mod_ = chart_modifier(chart, "chartXSelection");
            let Some(binding) = mod_.get("value") else {
                panic!("chartXSelection missing value binding: {:?}", mod_);
            };
            let SwiftValue::Struct(b) = binding else {
                panic!("expected Binding struct, got {:?}", binding);
            };
            assert_eq!(b.type_name, "Binding");
            // Binding carries the shared `_StateBox`.
            assert!(b.get("box").is_some(), "Binding should have box: {:?}", b);
        });
    }

    // ── Slice 7: broader 2D surface (selection/scales/scroll/mark visuals) ──

    #[test]
    fn chart_y_and_angle_selection_capture_bindings() {
        let src = r#"
struct Demo: View {
    @State private var ySel: Double? = nil
    @State private var angleSel: Double? = nil
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartYSelection(value: $ySel)
        .chartAngleSelection(value: $angleSel)
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            let SwiftValue::Struct(chart) = &view else {
                panic!("Chart");
            };
            for name in ["chartYSelection", "chartAngleSelection"] {
                let mod_ = chart_modifier(chart, name);
                let Some(binding) = mod_.get("value") else {
                    panic!("{name} missing value binding: {:?}", mod_);
                };
                let SwiftValue::Struct(b) = binding else {
                    panic!("{name}: expected Binding, got {:?}", binding);
                };
                assert_eq!(b.type_name, "Binding");
                assert!(b.get("box").is_some(), "{name} Binding needs box");
            }
        });
    }

    #[test]
    fn chart_symbol_and_line_style_scales_record_args() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            PointMark(x: .value("N", "A"), y: .value("C", 1))
                .symbol(by: .value("Type", "A"))
        }
        .chartSymbolScale(["A": ChartSymbolShape.circle])
        .chartSymbolSizeScale(domain: 0...10)
        .chartLineStyleScale(["A": StrokeStyle(lineWidth: 2)])
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            let SwiftValue::Struct(chart) = &view else {
                panic!("Chart");
            };
            for name in [
                "chartSymbolScale",
                "chartSymbolSizeScale",
                "chartLineStyleScale",
            ] {
                let mod_ = chart_modifier(chart, name);
                assert!(
                    mod_.fields.iter().any(|(k, _)| k != "name"),
                    "{name} should store scale args: {:?}",
                    mod_
                );
            }
        });
    }

    #[test]
    fn chart_background_and_overlay_capture_content() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartBackground { proxy in
            Color.gray
        }
        .chartOverlay { proxy in
            Text("overlay")
        }
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            let SwiftValue::Struct(chart) = &view else {
                panic!("Chart");
            };
            for (name, kinds) in [
                (
                    "chartBackground",
                    &["Color", "ChartBackgroundContent", "ChartProxy"][..],
                ),
                (
                    "chartOverlay",
                    &["Text", "ChartOverlayContent", "ChartProxy"][..],
                ),
            ] {
                let mod_ = chart_modifier(chart, name);
                let Some(content) = mod_.get("value") else {
                    panic!("{name} missing value: {:?}", mod_);
                };
                assert!(
                    !matches!(content, SwiftValue::Closure(_)),
                    "{name} must not store raw Closure: {:?}",
                    content
                );
                let SwiftValue::Struct(obj) = content else {
                    panic!("{name}: expected structured content, got {:?}", content);
                };
                assert!(
                    kinds.iter().any(|k| obj.type_name == *k) || obj.type_name == "ZStack",
                    "{name} unexpected content type {:?}, want one of {:?}",
                    obj.type_name,
                    kinds
                );
            }
            let json = uiir::to_json(&view);
            assert!(
                json.contains("chartBackground") && json.contains("chartOverlay"),
                "UIIR missing background/overlay mods: {json}"
            );
        });
    }

    #[test]
    fn chart_scroll_and_visible_domain_record_args() {
        let src = r#"
struct Demo: View {
    @State private var scrollX: String = "A"
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartScrollableAxes(.horizontal)
        .chartScrollPosition(x: $scrollX)
        .chartScrollTargetBehavior(true)
        .chartXVisibleDomain(length: 5)
        .chartYVisibleDomain(length: 10)
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            let SwiftValue::Struct(chart) = &view else {
                panic!("Chart");
            };
            let scroll_axes = chart_modifier(chart, "chartScrollableAxes");
            let Some(SwiftValue::Struct(axis)) = scroll_axes.get("value") else {
                panic!("chartScrollableAxes value: {:?}", scroll_axes);
            };
            assert_eq!(axis.type_name, "Axis");
            assert_eq!(
                axis.get("token"),
                Some(&SwiftValue::Str("horizontal".into()))
            );

            let pos = chart_modifier(chart, "chartScrollPosition");
            let Some(binding) = pos.get("x") else {
                panic!("chartScrollPosition(x:) missing x: {:?}", pos);
            };
            let SwiftValue::Struct(b) = binding else {
                panic!("expected Binding for scroll x, got {:?}", binding);
            };
            assert_eq!(b.type_name, "Binding");

            assert_has_modifier(chart, "chartScrollTargetBehavior");

            let xdom = chart_modifier(chart, "chartXVisibleDomain");
            assert_eq!(xdom.get("length"), Some(&SwiftValue::int(5)));
            let ydom = chart_modifier(chart, "chartYVisibleDomain");
            assert_eq!(ydom.get("length"), Some(&SwiftValue::int(10)));
        });
    }

    #[test]
    fn chart_axis_style_captures_content() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
        }
        .chartXAxisStyle { axis in
            axis
        }
        .chartYAxisStyle { axis in
            axis
        }
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            let SwiftValue::Struct(chart) = &view else {
                panic!("Chart");
            };
            for name in ["chartXAxisStyle", "chartYAxisStyle"] {
                let mod_ = chart_modifier(chart, name);
                let Some(content) = mod_.get("value") else {
                    panic!("{name} missing value: {:?}", mod_);
                };
                assert!(
                    !matches!(content, SwiftValue::Closure(_)),
                    "{name} must not store raw Closure"
                );
                let SwiftValue::Struct(obj) = content else {
                    panic!("{name}: expected structured content, got {:?}", content);
                };
                assert!(
                    obj.type_name == "ChartAxisContent" || obj.type_name == "ChartAxisStyleContent",
                    "{name} unexpected type {:?}",
                    obj.type_name
                );
            }
        });
    }

    #[test]
    fn mark_visual_modifiers_append_records() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
                .zIndex(2.0)
                .clipShape(Circle())
                .blur(radius: 1.5)
                .shadow(radius: 4, x: 1, y: 2)
                .mask {
                    Rectangle()
                }
        }
    }
}
"#;
        with_interp(src, |interp| {
            let mark = chart_single_mark(interp, "Demo", "BarMark");
            assert_has_modifier(&mark, "zIndex");
            assert_has_modifier(&mark, "clipShape");
            assert_has_modifier(&mark, "blur");
            assert_has_modifier(&mark, "shadow");
            assert_has_modifier(&mark, "mask");

            let z = mark_modifier(&mark, "zIndex");
            assert_eq!(z.get("value"), Some(&SwiftValue::Double(2.0)));

            let blur = mark_modifier(&mark, "blur");
            assert_eq!(blur.get("radius"), Some(&SwiftValue::Double(1.5)));

            let shadow = mark_modifier(&mark, "shadow");
            assert_eq!(shadow.get("radius"), Some(&SwiftValue::int(4)));

            let mask = mark_modifier(&mark, "mask");
            let Some(content) = mask.get("value") else {
                panic!("mask missing content: {:?}", mask);
            };
            assert_eq!(view_type_name(content), Some("Rectangle"));

            let clip = mark_modifier(&mark, "clipShape");
            let Some(shape) = clip.get("value") else {
                panic!("clipShape missing shape: {:?}", clip);
            };
            assert_eq!(view_type_name(shape), Some("Circle"));

            let view = render_root(interp, "Demo").expect("render");
            let json = uiir::to_json(&view);
            for needle in ["zIndex", "clipShape", "blur", "shadow", "mask"] {
                assert!(json.contains(needle), "UIIR missing {needle}: {json}");
            }
        });
    }

    #[test]
    fn mark_a11y_and_compositing_modifiers_append_records() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
                .accessibilityHidden(true)
                .accessibilityIdentifier("bar-a")
                .accessibilityLabel("Series A")
                .accessibilityValue("1")
                .compositingLayer()
                .alignsMarkStylesWithPlotArea(true)
        }
    }
}
"#;
        with_interp(src, |interp| {
            let mark = chart_single_mark(interp, "Demo", "BarMark");
            assert_has_modifier(&mark, "accessibilityHidden");
            assert_has_modifier(&mark, "accessibilityIdentifier");
            assert_has_modifier(&mark, "accessibilityLabel");
            assert_has_modifier(&mark, "accessibilityValue");
            assert_has_modifier(&mark, "compositingLayer");
            assert_has_modifier(&mark, "alignsMarkStylesWithPlotArea");

            let hidden = mark_modifier(&mark, "accessibilityHidden");
            assert_eq!(hidden.get("value"), Some(&SwiftValue::Bool(true)));

            let id = mark_modifier(&mark, "accessibilityIdentifier");
            assert_eq!(id.get("value"), Some(&SwiftValue::Str("bar-a".into())));

            let label = mark_modifier(&mark, "accessibilityLabel");
            assert_eq!(
                label.get("value"),
                Some(&SwiftValue::Str("Series A".into()))
            );

            let value = mark_modifier(&mark, "accessibilityValue");
            assert_eq!(value.get("value"), Some(&SwiftValue::Str("1".into())));

            let aligns = mark_modifier(&mark, "alignsMarkStylesWithPlotArea");
            assert_eq!(aligns.get("value"), Some(&SwiftValue::Bool(true)));

            let view = render_root(interp, "Demo").expect("render");
            let json = uiir::to_json(&view);
            for needle in [
                "accessibilityHidden",
                "accessibilityIdentifier",
                "accessibilityLabel",
                "accessibilityValue",
                "compositingLayer",
                "alignsMarkStylesWithPlotArea",
            ] {
                assert!(json.contains(needle), "UIIR missing {needle}: {json}");
            }
        });
    }

    // ── Slice 6 review: plottable wire form + long modifier chains ──────────

    /// PlottableValue UIIR: string stays string (even numeric-looking), Double
    /// is a JSON number, non-string/non-number coerces to a string, quotes escape.
    #[test]
    fn plottable_value_uiir_is_always_string_or_number() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(
                x: .value("Label", "3"),
                y: .value("Y", 1.5)
            )
            BarMark(
                x: .value("La\"bel", "a\"b"),
                y: .value("Flag", true)
            )
        }
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            let json = uiir::to_json(&view);
            // Numeric-looking String → JSON string, not number.
            assert!(
                json.contains(r#""$":"plottable","label":"Label","value":"3""#),
                "string plottable must stay JSON string: {json}"
            );
            // Double → JSON number.
            assert!(
                json.contains(r#""$":"plottable","label":"Y","value":1.5"#),
                "double plottable must be JSON number: {json}"
            );
            // Quote escaping in label and value.
            assert!(
                json.contains(r#""$":"plottable","label":"La\"bel","value":"a\"b""#),
                "quotes must be escaped in plottable label/value: {json}"
            );
            // Bool (or any non-string/non-number) → JSON string via display form.
            assert!(
                json.contains(r#""$":"plottable","label":"Flag","value":"true""#),
                "bool plottable must coerce to JSON string, not bool: {json}"
            );
            assert!(
                !json.contains(r#""label":"Flag","value":true"#),
                "bool must not serialize as JSON boolean: {json}"
            );
        });
    }

    /// RuleMark range forms keep every bound in the UIIR args object.
    #[test]
    fn rule_mark_range_forms_carry_all_bounds_in_uiir() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            RuleMark(
                xStart: .value("From", 1),
                xEnd: .value("To", 4),
                y: .value("Band", "A")
            )
            RuleMark(
                yStart: .value("Low", 0),
                yEnd: .value("High", 10),
                x: .value("Cat", "B")
            )
        }
    }
}
"#;
        with_interp(src, |interp| {
            let view = render_root(interp, "Demo").expect("render");
            let json = uiir::to_json(&view);
            // Horizontal segment: xStart + xEnd + y all present.
            assert!(
                json.contains(r#""xStart":{"$":"plottable","label":"From","value":1}"#),
                "missing xStart: {json}"
            );
            assert!(
                json.contains(r#""xEnd":{"$":"plottable","label":"To","value":4}"#),
                "missing xEnd: {json}"
            );
            assert!(
                json.contains(r#""y":{"$":"plottable","label":"Band","value":"A"}"#),
                "missing y on x-range rule: {json}"
            );
            // Vertical segment: yStart + yEnd + x all present.
            assert!(
                json.contains(r#""yStart":{"$":"plottable","label":"Low","value":0}"#),
                "missing yStart: {json}"
            );
            assert!(
                json.contains(r#""yEnd":{"$":"plottable","label":"High","value":10}"#),
                "missing yEnd: {json}"
            );
            assert!(
                json.contains(r#""x":{"$":"plottable","label":"Cat","value":"B"}"#),
                "missing x on y-range rule: {json}"
            );
        });
    }

    /// >12 mark modifiers must all appear on the mark UIIR (no host-style cap
    /// at the runtime serialization layer).
    #[test]
    fn mark_with_more_than_twelve_modifiers_keeps_all_in_uiir() {
        let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("N", "A"), y: .value("C", 1))
                .opacity(0.01)
                .opacity(0.02)
                .opacity(0.03)
                .opacity(0.04)
                .opacity(0.05)
                .opacity(0.06)
                .opacity(0.07)
                .opacity(0.08)
                .opacity(0.09)
                .opacity(0.10)
                .opacity(0.11)
                .opacity(0.12)
                .opacity(0.13)
                .cornerRadius(4)
        }
    }
}
"#;
        with_interp(src, |interp| {
            let mark = chart_single_mark(interp, "Demo", "BarMark");
            let Some(SwiftValue::Array(mods)) = mark.get(MODIFIERS_FIELD) else {
                panic!("expected _modifiers");
            };
            assert_eq!(
                mods.len(),
                14,
                "runtime must keep all 14 modifiers, got {mods:?}"
            );
            // Last is cornerRadius; thirteenth opacity is still present.
            let last = mark_modifier(&mark, "cornerRadius");
            assert_eq!(last.get("value"), Some(&SwiftValue::int(4)));
            let json = uiir::to_json(&render_root(interp, "Demo").expect("render"));
            let opacity_count = json.matches(r#""name":"opacity""#).count();
            assert_eq!(
                opacity_count, 13,
                "UIIR must list all 13 opacity modifiers: {json}"
            );
            assert!(
                json.contains(r#""name":"cornerRadius""#),
                "UIIR missing cornerRadius after long chain: {json}"
            );
        });
    }

    /// Integration: leading-dot `.value(...)` works under the exact host prelude
    /// composition (no test-only extra prepend). Proves cli/wasm prepare wiring.
    #[test]
    fn host_prelude_composition_leading_dot_bar_mark() {
        let user = r#"
struct HostDemo: View {
    var body: some View {
        Chart {
            BarMark(x: .value("Name", "A"), y: .value("Count", 3))
        }
    }
}
"#;
        // Build only what hosts build — see `host_program`.
        assert!(
            host_program(user).contains(PRELUDE.trim()),
            "host program must include charts PRELUDE"
        );
        with_interp(user, |interp| {
            let view = render_root(interp, "HostDemo").expect("render");
            assert_eq!(view_type_name(&view), Some("Chart"));
            let SwiftValue::Struct(obj) = &view else {
                panic!("expected Chart struct");
            };
            let Some(SwiftValue::Array(children)) = obj.get(CHILDREN_FIELD) else {
                panic!("expected Chart _children");
            };
            assert_eq!(children.len(), 1);
            assert_eq!(view_type_name(&children[0]), Some("BarMark"));
            let SwiftValue::Struct(mark) = &children[0] else {
                panic!("expected BarMark");
            };
            let Some(SwiftValue::Struct(x)) = mark.get("x") else {
                panic!("leading-dot x failed under host prelude");
            };
            assert_eq!(x.type_name, "PlottableValue");
            assert_eq!(x.get("label"), Some(&SwiftValue::Str("Name".into())));
        });
    }
}
