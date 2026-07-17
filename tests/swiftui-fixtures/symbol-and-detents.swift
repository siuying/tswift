// Token/value passthrough modifiers: `.symbolEffect` (SymbolEffect token),
// `.sensoryFeedback` (SensoryFeedback token + a `trigger:` value),
// `.presentationDetents` (a `[PresentationDetent]` token array), and the
// geometry-effect value passthroughs `.transformEffect` / `.projectionEffect`.
// All recorded straight onto their view node (rendered in the visible tree so
// the goldens exercise serialization). `.selection` is used (not `.success`,
// which collides with the builtin `Result` enum case).
struct V: View {
    @State private var count = 0

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: "star")
                .symbolEffect(.bounce)
            Text("Tap")
                .sensoryFeedback(.selection, trigger: count)
                .transformEffect(count)
                .projectionEffect(count)
            Text("Sheet")
                .presentationDetents([.medium, .large])
        }
    }
}
