// Environment tab — a shared ObservableObject injected via .environmentObject
// and read by a child through @EnvironmentObject. Tapping toggles the theme,
// mutating the injected object without an owner reference in scope.
class Settings: ObservableObject {
    @Published var theme = "Dark"
    func toggle() { theme = theme == "Dark" ? "Light" : "Dark" }
}

struct ThemeLabel: View {
    @EnvironmentObject var settings: Settings
    var body: some View {
        Button("Theme: \(settings.theme)") { settings.toggle() }
    }
}

struct EnvironmentView: View {
    @StateObject var settings = Settings()
    var body: some View {
        ThemeLabel().environmentObject(settings)
    }
}
