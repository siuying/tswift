import SwiftUI

/// TSwift Playground — edit Swift in a Runestone code editor and watch a live,
/// interactive SwiftUI preview rendered by the tswift runtime (PreviewSession
/// over tswift-ffi → UiirRenderer). The product sibling of `examples/ios`, which
/// stays a minimal link-smoke demo.
@main
struct TSwiftPlaygroundApp: App {
    var body: some Scene {
        WindowGroup {
            PlaygroundView()
        }
    }
}
