// Preference / phase-animation / command modifiers (recorded-only).
// `preference(key:value:)` records the value payload (the key metatype is not
// representable and is dropped); the closure-driven ones record a bare marker
// and stash their trailing closure (never serialized).
struct WidthKey: PreferenceKey {
    static var defaultValue: Double = 0
    static func reduce(value: inout Double, nextValue: () -> Double) {}
}

struct V: View {
    var body: some View {
        Text("hi")
            .preference(key: WidthKey.self, value: 3.0)
            .transformPreference(WidthKey.self) { _ in }
            .phaseAnimator([0, 1]) { content, phase in content }
            .onCommand(#selector(NSText.copy(_:))) { }
            .onPasteCommand(of: ["public.text"]) { _ in }
    }
}
