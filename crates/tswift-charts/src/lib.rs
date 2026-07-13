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

/// Register every currently-supported Charts constructor into `interp`.
pub fn install(interp: &mut Interpreter<'_>) {
    // `Chart { … }` — trailing content-builder closure becomes `_children`
    // (same shape as SwiftUI containers).
    interp.register_free_fn("Chart", chart_init);
    // `BarMark(x:y:)` — typed so leading-dot `.value(...)` resolves against
    // `PlottableValue` (see PRELUDE).
    interp.register_free_fn_typed(
        "BarMark",
        bar_mark_init,
        vec![
            BuiltinParam::labeled("x", "PlottableValue"),
            BuiltinParam::labeled("y", "PlottableValue"),
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

/// `BarMark(x:y:)` — mark leaf carrying its plottable x/y args.
fn bar_mark_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut x: Option<SwiftValue> = None;
    let mut y: Option<SwiftValue> = None;
    for arg in args {
        match arg.label.as_deref() {
            Some("x") => x = Some(arg.value),
            Some("y") => y = Some(arg.value),
            _ => {}
        }
    }
    let mut fields = Vec::new();
    if let Some(x) = x {
        fields.push(("x".into(), x));
    }
    if let Some(y) = y {
        fields.push(("y".into(), y));
    }
    Ok(view_value("BarMark", fields))
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

/// Every Charts entry registered by [`install`], as coverage keys
/// (`Type.member`, matching `tools/framework-inventory/coverage.py`).
pub fn registered_keys() -> Vec<String> {
    let mut sink = std::io::sink();
    let mut interp = Interpreter::new(&mut sink);
    install(&mut interp);
    let mut keys: Vec<String> = interp
        .registered_keys()
        .into_iter()
        .filter_map(|key| match key.as_str() {
            "Chart" | "BarMark" => Some(format!("{key}.init")),
            "PlottableValue.value" => Some(key),
            _ => None,
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
    use tswift_core::Interpreter;
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
    fn registered_keys_cover_slice1() {
        let keys = registered_keys();
        assert_eq!(
            keys,
            vec![
                "BarMark.init".to_string(),
                "Chart.init".to_string(),
                "PlottableValue.value".to_string(),
            ]
        );
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
            let view = render_root(interp, "Demo").expect("render");
            assert_eq!(view_type_name(&view), Some("Chart"));
            let SwiftValue::Struct(obj) = &view else {
                panic!("expected Chart struct");
            };
            let Some(SwiftValue::Array(children)) = obj.get(CHILDREN_FIELD) else {
                panic!("expected Chart _children");
            };
            assert_eq!(children.len(), 1, "Chart should have one BarMark child");
            assert_eq!(view_type_name(&children[0]), Some("BarMark"));
            let SwiftValue::Struct(mark) = &children[0] else {
                panic!("expected BarMark struct");
            };
            // x: .value("Name", "A")
            let Some(SwiftValue::Struct(x)) = mark.get("x") else {
                panic!("expected x PlottableValue, got {:?}", mark.get("x"));
            };
            assert_eq!(x.type_name, "PlottableValue");
            assert_eq!(x.get("label"), Some(&SwiftValue::Str("Name".into())));
            assert_eq!(x.get("value"), Some(&SwiftValue::Str("A".into())));
            // y: .value("Count", 3)
            let Some(SwiftValue::Struct(y)) = mark.get("y") else {
                panic!("expected y PlottableValue, got {:?}", mark.get("y"));
            };
            assert_eq!(y.type_name, "PlottableValue");
            assert_eq!(y.get("label"), Some(&SwiftValue::Str("Count".into())));
            assert_eq!(y.get("value"), Some(&SwiftValue::int(3)));

            // UIIR JSON must name Chart + BarMark (hosts key off `kind`).
            let json = uiir::to_json(&view);
            assert!(
                json.contains(r#""kind":"Chart""#),
                "UIIR missing Chart: {json}"
            );
            assert!(
                json.contains(r#""kind":"BarMark""#),
                "UIIR missing BarMark: {json}"
            );
            assert!(
                json.contains("Name") && json.contains("Count"),
                "UIIR missing labels: {json}"
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
