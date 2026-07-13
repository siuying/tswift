//! Phase A module-tagging metadata (ADR-0020): install scopes stamp type and
//! struct-method ownership without changing dispatch.

use tswift_core::Interpreter;

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
    // Shared modifier name: last install wins; Charts re-registers after SwiftUI.
    assert_eq!(
        interp.struct_method_module("foregroundStyle"),
        Some("Charts")
    );
    // A SwiftUI-only modifier keeps its module.
    assert_eq!(interp.struct_method_module("padding"), Some("SwiftUI"));
    // Stdlib free-fn constructor under the base module.
    assert_eq!(interp.type_module("print"), Some("Swift"));
}
