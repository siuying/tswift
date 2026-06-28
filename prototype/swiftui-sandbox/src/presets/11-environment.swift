// EnvironmentObject — a shared ObservableObject injected with
// .environmentObject and read by a child via @EnvironmentObject. Tapping the
// button mutates the shared object; the views that read it re-render.
class Settings: ObservableObject {
    @Published var theme = "Dark"
    func toggle() { theme = theme == "Dark" ? "Light" : "Dark" }
}

struct ThemeCard: View {
    @EnvironmentObject var settings: Settings
    var body: some View {
        VStack {
            Text("Settings")
                .font(.largeTitle)
                .fontWeight(.bold)
            Text("Current theme: \(settings.theme)")
                .font(.headline)
                .foregroundColor(.secondary)
            Button("Switch theme") { settings.toggle() }
                .foregroundColor(.white)
                .padding()
                .background(Color.blue)
                .cornerRadius(10)
        }
        .padding()
    }
}

struct EnvironmentView: View {
    @StateObject var settings = Settings()
    var body: some View {
        ThemeCard().environmentObject(settings)
    }
}
