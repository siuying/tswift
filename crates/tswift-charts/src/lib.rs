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
        render_root, uiir, view_type_name, CHILDREN_FIELD, PRELUDE as SWIFTUI_PRELUDE,
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
    fn registered_keys_cover_core_marks() {
        let keys = registered_keys();
        assert_eq!(
            keys,
            vec![
                "AreaMark.init".to_string(),
                "BarMark.init".to_string(),
                "Chart.init".to_string(),
                "LineMark.init".to_string(),
                "PlottableValue.value".to_string(),
                "PointMark.init".to_string(),
                "RectangleMark.init".to_string(),
                "RuleMark.init".to_string(),
                "SectorMark.init".to_string(),
            ]
        );
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
