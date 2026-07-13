//! Phase A/B module tagging + receiver-module struct-method dispatch (ADR-0020).

use tswift_core::{Interpreter, SwiftValue};
use tswift_swiftui::{render_root, view_type_name, PRELUDE as SWIFTUI_PRELUDE};

use super::support::{mark_modifier, with_interp};
use crate::{install, PRELUDE};

#[test]
fn install_scopes_stamp_type_and_struct_method_modules() {
    let mut sink = std::io::sink();
    let mut interp = Interpreter::new(&mut sink);
    tswift_std::install(&mut interp);
    tswift_swiftui::install(&mut interp);
    crate::install(&mut interp);

    assert_eq!(interp.type_module("Text"), Some("SwiftUI"));
    assert_eq!(interp.type_module("BarMark"), Some("Charts"));
    assert_eq!(interp.type_module("Chart"), Some("Charts"));
    // Shared name: both modules keep a candidate (no last-wins clobber).
    let fg = interp.struct_method_modules("foregroundStyle");
    assert!(
        fg.contains(&"SwiftUI") && fg.contains(&"Charts"),
        "expected both modules, got {fg:?}"
    );
    assert_eq!(
        interp.struct_method_module_for("foregroundStyle", "Text"),
        Some("SwiftUI")
    );
    assert_eq!(
        interp.struct_method_module_for("foregroundStyle", "BarMark"),
        Some("Charts")
    );
    // A SwiftUI-only modifier keeps its module (Charts does not re-register it).
    assert_eq!(interp.struct_method_module("padding"), Some("SwiftUI"));
    assert_eq!(
        interp.struct_method_module_for("padding", "Text"),
        Some("SwiftUI")
    );
    // Shared pure View/ChartContent modifier: Charts owns its candidate;
    // BarMark resolves to Charts, not SwiftUI fallback.
    assert_eq!(
        interp.struct_method_module_for("opacity", "BarMark"),
        Some("Charts")
    );
    assert_eq!(
        interp.struct_method_module_for("opacity", "Text"),
        Some("SwiftUI")
    );
    // Stdlib free-fn constructor under the base module.
    assert_eq!(interp.type_module("print"), Some("Swift"));
}

/// Install order must not change which handler fires or the resulting
/// `_Modifier` wire fields for a shared name on either receiver.
#[test]
fn install_order_independent_foreground_style_dispatch() {
    let src_text = r#"
        struct Root: View {
            var body: some View {
                Text("hi").foregroundStyle(.red)
            }
        }
    "#;
    let src_mark = r#"
        struct Root: View {
            var body: some View {
                Chart {
                    BarMark(
                        x: .value("N", "A"),
                        y: .value("V", 1)
                    )
                    .foregroundStyle(by: .value("Type", "x"))
                }
            }
        }
    "#;

    let text_swiftui_first = run_uiir_json(install_std_swiftui_charts, src_text);
    let text_charts_first = run_uiir_json(install_std_charts_swiftui, src_text);
    assert_eq!(
        text_swiftui_first, text_charts_first,
        "Text.foregroundStyle wire must be install-order independent"
    );

    let mark_swiftui_first = run_uiir_json(install_std_swiftui_charts, src_mark);
    let mark_charts_first = run_uiir_json(install_std_charts_swiftui, src_mark);
    assert_eq!(
        mark_swiftui_first, mark_charts_first,
        "BarMark.foregroundStyle wire must be install-order independent"
    );

    // Sanity: both forms produce a foregroundStyle modifier record.
    assert!(text_swiftui_first.contains(r#""name":"foregroundStyle""#));
    assert!(mark_swiftui_first.contains(r#""name":"foregroundStyle""#));
    assert!(
        mark_swiftui_first.contains(r#""by""#)
            || mark_swiftui_first.contains("PlottableValue")
            || mark_swiftui_first.contains(r#""$":"plottable""#)
    );
}

/// Receiver routing: Text → SwiftUI candidate, BarMark → Charts candidate;
/// both emit the expected `_Modifier` record (identical to today's wire).
#[test]
fn receiver_routes_foreground_style_by_module() {
    with_interp(
        r#"
        struct Root: View {
            var body: some View {
                VStack {
                    Text("hi").foregroundStyle(.red)
                    Chart {
                        BarMark(
                            x: .value("N", "A"),
                            y: .value("V", 1)
                        )
                        .foregroundStyle(by: .value("Type", "x"))
                    }
                }
            }
        }
        "#,
        |interp| {
            assert_eq!(
                interp.struct_method_module_for("foregroundStyle", "Text"),
                Some("SwiftUI")
            );
            assert_eq!(
                interp.struct_method_module_for("foregroundStyle", "BarMark"),
                Some("Charts")
            );

            let view = render_root(interp, "Root").expect("render");
            let json = tswift_swiftui::uiir::to_json(&view);
            // Text form: solid color token.
            assert!(
                json.contains(r#""name":"foregroundStyle""#)
                    && json.contains(r#""$":"color""#)
                    && json.contains(r#""name":"red""#),
                "Text.foregroundStyle(.red) missing color wire: {json}"
            );
            // BarMark form: series-by plottable.
            assert!(
                json.contains(r#""$":"plottable""#) || json.contains("PlottableValue"),
                "BarMark.foregroundStyle(by:) missing plottable wire: {json}"
            );
        },
    );
}

/// User / unknown receivers still resolve a shared modifier via the fallback
/// path (base Swift → alphabetical by module id) — preserves today's generic seam.
#[test]
fn user_struct_fallback_resolves_shared_modifier() {
    with_interp(
        r#"
        struct Widget {}
        struct Root: View {
            var body: some View {
                Text("x")
            }
        }
        "#,
        |interp| {
            // Unknown type has no type_module entry → fallback alphabetical
            // among non-Swift candidates: Charts < SwiftUI.
            assert_eq!(interp.type_module("Widget"), None);
            assert_eq!(
                interp.struct_method_module_for("foregroundStyle", "Widget"),
                Some("Charts")
            );
            assert_eq!(
                interp.struct_method_module_for("opacity", "Widget"),
                Some("Charts")
            );
        },
    );

    // End-to-end: Charts mark uses Charts' own shared-modifier candidates.
    with_interp(
        r#"
        struct Root: View {
            var body: some View {
                Chart {
                    BarMark(x: .value("N", "A"), y: .value("V", 1))
                        .opacity(0.5)
                        .cornerRadius(4)
                }
            }
        }
        "#,
        |interp| {
            let view = render_root(interp, "Root").expect("render");
            assert_eq!(view_type_name(&view), Some("Chart"));
            let SwiftValue::Struct(chart) = &view else {
                panic!("expected Chart");
            };
            let Some(SwiftValue::Array(children)) = chart.get(tswift_swiftui::CHILDREN_FIELD)
            else {
                panic!("expected children");
            };
            let SwiftValue::Struct(mark) = &children[0] else {
                panic!("expected mark");
            };
            let _ = mark_modifier(mark, "opacity");
            let _ = mark_modifier(mark, "cornerRadius");
            assert_eq!(
                interp.struct_method_module_for("opacity", "BarMark"),
                Some("Charts")
            );
            assert_eq!(
                interp.struct_method_module_for("cornerRadius", "BarMark"),
                Some("Charts")
            );
        },
    );
}

/// Charts is self-contained: with only std + charts (no SwiftUI installed),
/// `BarMark(...).foregroundStyle(...)` still resolves via Charts' own candidate.
#[test]
fn charts_only_install_resolves_own_foreground_style() {
    // Capture print so we can prove the method call actually executed.
    let program = format!(
        "{PRELUDE}\n\
         let mark = BarMark(\n\
             x: .value(\"N\", \"A\"),\n\
             y: .value(\"V\", 1)\n\
         ).foregroundStyle(by: .value(\"Type\", \"x\"))\n\
         print(mark)\n"
    );
    let analysis =
        tswift_frontend::Analysis::analyze(&program, "charts_only.swift").expect("analyze");
    let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
    let mut out = Vec::new();
    let mut interp = Interpreter::new(&mut out);
    tswift_std::install(&mut interp);
    // Intentionally no tswift_swiftui::install — Charts must not depend on it.
    install(&mut interp);

    assert_eq!(interp.type_module("BarMark"), Some("Charts"));
    let fg_mods = interp.struct_method_modules("foregroundStyle");
    assert!(
        fg_mods.contains(&"Charts") && !fg_mods.contains(&"SwiftUI"),
        "expected Charts-only foregroundStyle, got {fg_mods:?}"
    );
    assert_eq!(
        interp.struct_method_module_for("foregroundStyle", "BarMark"),
        Some("Charts")
    );
    assert_eq!(
        interp.struct_method_module_for("opacity", "BarMark"),
        Some("Charts")
    );

    interp.run(analysis).expect("run without SwiftUI");
    drop(interp);
    let printed = String::from_utf8_lossy(&out);
    // Display of the mark must mention BarMark + the Charts-applied modifier.
    assert!(
        printed.contains("BarMark") && printed.contains("foregroundStyle"),
        "expected BarMark with Charts foregroundStyle applied, got: {printed}"
    );
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn install_std_swiftui_charts(interp: &mut Interpreter<'_>) {
    tswift_std::install(interp);
    tswift_swiftui::install(interp);
    install(interp);
}

fn install_std_charts_swiftui(interp: &mut Interpreter<'_>) {
    tswift_std::install(interp);
    // Charts before SwiftUI — order that would previously clobber wrong.
    install(interp);
    tswift_swiftui::install(interp);
}

fn run_uiir_json(install_fn: fn(&mut Interpreter<'_>), user: &str) -> String {
    let program = format!(
        "{SWIFTUI_PRELUDE}\n{}\n{PRELUDE}\n{user}\n",
        tswift_swiftdata::QUERY_PRELUDE,
    );
    let analysis =
        tswift_frontend::Analysis::analyze(&program, "order_test.swift").expect("analyze");
    let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
    let mut sink = std::io::sink();
    let mut interp = Interpreter::new(&mut sink);
    install_fn(&mut interp);
    interp.run(analysis).expect("run");
    let view = render_root(&mut interp, "Root").expect("render");
    tswift_swiftui::uiir::to_json(&view)
}
