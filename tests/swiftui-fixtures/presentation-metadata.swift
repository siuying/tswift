// Presentation / window / list metadata passthrough modifiers. Verifies that
// each records its scalar / Bool / String / [String] / URL / token value onto
// the view node so the serialized UIIR carries the semantic data. The hosts
// honor or ignore these (no on-device presentation stack in a headless run).
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("sheet")
                .presentationCornerRadius(16)
                .dialogPreventsAppTermination(true)
            Text("secret")
                .contentCaptureProtected(true)
                .typeSelectEquivalent("secret")
            Text("row")
                .listRowHoverEffect(.highlight)
                .listRowHoverEffectDisabled(true)
                .sliderThumbVisibility(.visible)
            Text("doc")
                .handlesExternalEvents(preferring: ["open"], allowing: ["*"])
                .navigationDocument("file:///tmp/doc.txt")
        }
    }
}
