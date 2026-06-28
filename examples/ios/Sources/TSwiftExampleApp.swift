import SwiftUI

/// Demo app proving the TSwift package products link the one TSwiftFFI
/// .xcframework: a Run screen (TSwiftCore) and a live Preview screen (TSwiftUI,
/// rendered through UiirRenderer's RenderHostView).
@main
struct TSwiftExampleApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}

struct ContentView: View {
    var body: some View {
        TabView {
            RunView()
                .tabItem { Label("Run", systemImage: "play.fill") }
            PreviewView()
                .tabItem { Label("Preview", systemImage: "eye.fill") }
        }
    }
}
