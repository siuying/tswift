// Preference-value transforms and ornament content.
// - background/overlayPreferenceValue take a (Value) -> View transform; the
//   preference Value is not computed by a headless runtime, so they record a
//   bare marker and stash the closure (recorded-only);
// - ornament lowers its trailing @ViewBuilder to a nested child subtree.
struct WidthKey: PreferenceKey {
    static var defaultValue: Double = 0
    static func reduce(value: inout Double, nextValue: () -> Double) {}
}

struct V: View {
    var body: some View {
        Text("hi")
            .backgroundPreferenceValue(WidthKey.self) { _ in Color.red }
            .overlayPreferenceValue(WidthKey.self) { _ in Color.blue }
            .ornament {
                Text("badge")
            }
    }
}
