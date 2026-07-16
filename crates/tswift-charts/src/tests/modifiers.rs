//! Mark and chart-level modifier tests.

use super::support::{
    assert_has_modifier, chart_modifier, chart_single_mark, mark_modifier, with_interp,
};
use tswift_core::SwiftValue;
use tswift_swiftui::{render_root, uiir, view_type_name, CHILDREN_FIELD, MODIFIERS_FIELD};

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
