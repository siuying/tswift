// C5 — content views: Label, Image(systemName:), Image(_ name), ProgressView
// (issue #192).
struct V: View {
    var body: some View {
        VStack(spacing: 12) {
            Label("Favorites", systemImage: "star.fill")
                .font(.headline)
            Image(systemName: "house.fill")
                .foregroundStyle(.blue)
            Image("logo")
            ProgressView()
            ProgressView(value: 0.6)
        }
    }
}
