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
    // Also expose `PlottableValue.value` as a Rust static intrinsic so
    // `PlottableValue.value(...)` (qualified) works without the prelude and
    // so coverage sees a live registry key. Leading-dot still needs PRELUDE
    // (resolve_implicit_static_method only sees user-declared statics).
    let plottable = BuiltinReceiver::register_extension("PlottableValue");
    interp.register_static(plottable, "value", plottable_value_static);
    // ChartContent mark modifiers (any mark view value; COW `_modifiers` append).
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

/// Free-fn mark / container names that become `Type.init` coverage keys.
const MARK_INIT_NAMES: &[&str] = &[
    "AreaMark",
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
            if MARK_INIT_NAMES.contains(&key.as_str()) {
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
            "BarMark.init",
            "Chart.init",
            "ChartContent.annotation",
            "ChartContent.cornerRadius",
            "ChartContent.foregroundStyle",
            "ChartContent.interpolationMethod",
            "ChartContent.lineStyle",
            "ChartContent.offset",
            "ChartContent.opacity",
            "ChartContent.position",
            "ChartContent.symbol",
            "ChartContent.symbolSize",
            "LineMark.init",
            "PlottableValue.value",
            "PointMark.init",
            "RectangleMark.init",
            "RuleMark.init",
            "SectorMark.init",
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
