import SwiftUI

struct ThemeLabel: View {
    @Environment(\.themeName) var themeName: String

    var body: some View {
        Text("Theme: \(themeName)")
    }
}

struct EnvironmentTheming: View {
    @State private var dark = true

    var body: some View {
        VStack {
            ThemeLabel().environment(\.themeName, dark ? "Dark" : "Light")
            Button("Toggle theme") { dark.toggle() }
        }
    }
}
