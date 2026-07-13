//! Shared test harness: host prelude stack + interpreter setup + assertions.

use std::rc::Rc;

use tswift_core::{Interpreter, StructObj, SwiftValue};
use tswift_swiftui::{
    render_root, uiir, view_type_name, CHILDREN_FIELD, MODIFIERS_FIELD, PRELUDE as SWIFTUI_PRELUDE,
};

use crate::{install, PRELUDE};

/// Same prelude stack hosts use (`tswift-cli` / `tswift-wasm` prepare):
/// SwiftUI PRELUDE + SwiftData QUERY_PRELUDE + Charts PRELUDE + user source.
/// Do not invent a different composition here — hosts must stay in lockstep.
pub(super) fn host_program(user: &str) -> String {
    format!(
        "{SWIFTUI_PRELUDE}\n{}\n{PRELUDE}\n{user}\n",
        tswift_swiftdata::QUERY_PRELUDE,
    )
}

/// Analyze + run `src` with the host prelude stack and SwiftUI + Charts installed.
pub(super) fn with_interp<R>(src: &str, f: impl FnOnce(&mut Interpreter) -> R) -> R {
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

/// First `_Modifier` on a mark with the given `name`, or panic.
pub(super) fn mark_modifier<'a>(mark: &'a StructObj, name: &str) -> &'a StructObj {
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

pub(super) fn assert_has_modifier(mark: &StructObj, name: &str) {
    let _ = mark_modifier(mark, name);
}

/// Assert `Chart { mark }` has one child of `kind` and returns that child struct.
pub(super) fn chart_single_mark(interp: &mut Interpreter, root: &str, kind: &str) -> Rc<StructObj> {
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

pub(super) fn assert_plottable(mark: &StructObj, field: &str, label: &str, value: SwiftValue) {
    let Some(SwiftValue::Struct(pv)) = mark.get(field) else {
        panic!("expected {field} PlottableValue, got {:?}", mark.get(field));
    };
    assert_eq!(pv.type_name, "PlottableValue");
    assert_eq!(pv.get("label"), Some(&SwiftValue::Str(label.into())));
    assert_eq!(pv.get("value"), Some(&value));
}

pub(super) fn assert_uiir_kinds(view: &SwiftValue, kinds: &[&str]) {
    let json = uiir::to_json(view);
    for kind in kinds {
        let needle = format!(r#""kind":"{kind}""#);
        assert!(json.contains(&needle), "UIIR missing {kind}: {json}");
    }
}

/// First `_Modifier` on a chart/view with the given `name`, or panic.
pub(super) fn chart_modifier<'a>(chart: &'a StructObj, name: &str) -> &'a StructObj {
    mark_modifier(chart, name)
}
