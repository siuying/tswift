//! Axis-content and chart axis modifier tests.

use super::support::{chart_modifier, with_interp};
use tswift_core::SwiftValue;
use tswift_swiftui::{render_root, view_type_name, CHILDREN_FIELD};

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
