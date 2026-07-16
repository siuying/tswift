//! UIIR wire-form tests (PlottableValue JSON, RuleMark bounds, long modifier chains).

use super::support::{assert_has_modifier, chart_single_mark, mark_modifier, with_interp};
use tswift_core::SwiftValue;
use tswift_swiftui::{render_root, uiir, view_type_name, MODIFIERS_FIELD};

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

/// More than 12 mark modifiers must all appear on the mark UIIR (no host-style
/// cap at the runtime serialization layer).
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
