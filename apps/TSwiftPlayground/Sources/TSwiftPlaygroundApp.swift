import SwiftUI

/// TSwift Studio — a mini-IDE over the tswift runtime: multi-file projects, a
/// Runestone code editor, a symbol outline, and a run pane that renders SwiftUI
/// live (PreviewSession over tswift-ffi → UiirRenderer) or captures console
/// output. The quick single-file playground remains reachable as a Scratchpad
/// tab. The product sibling of `examples/ios`, which stays a link-smoke demo.
@main
struct TSwiftPlaygroundApp: App {
    var body: some Scene {
        WindowGroup {
            RootView()
        }
    }
}
