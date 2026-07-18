//! Mark constructor / multi-mark render tests.

use super::support::{
    assert_plottable, assert_uiir_kinds, chart_single_mark, host_program, with_interp,
};
use crate::PRELUDE;
use tswift_core::SwiftValue;
use tswift_swiftui::{render_root, uiir, view_type_name, CHILDREN_FIELD};

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

#[test]
fn chart_body_materializes_the_existing_chart_uiir() {
    let src = r#"
struct Demo: View {
    var body: some View {
        Chart {
            PointMark(x: .value("X", 1), y: .value("Y", 2))
        }.body
    }
}
"#;
    with_interp(src, |interp| {
        let view = render_root(interp, "Demo").expect("render");
        assert_eq!(view_type_name(&view), Some("Chart"));
        let SwiftValue::Struct(chart) = &view else {
            panic!("expected Chart body");
        };
        let Some(SwiftValue::Array(children)) = chart.get(CHILDREN_FIELD) else {
            panic!("Chart.body lost children: {chart:?}");
        };
        assert_eq!(children.len(), 1);
        assert_eq!(view_type_name(&children[0]), Some("PointMark"));
        assert_uiir_kinds(&view, &["Chart", "PointMark"]);
    });
}
