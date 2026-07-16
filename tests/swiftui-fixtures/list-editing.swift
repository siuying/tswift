// List-editing, row-layout, and misc identity modifiers. Each records a scalar,
// Bool, String, or passthrough value (no leading-dot token): deleteDisabled /
// moveDisabled / selectionDisabled (Bool), listRowSpacing / listSectionSpacing
// (CGFloat), badge (Int), id (Hashable), interactionActivityTrackingTag
// (String), and the no-arg geometryGroup / invalidatableContent hints.
struct V: View {
    var body: some View {
        List {
            Text("row")
                .badge(3)
                .id("row-1")
                .deleteDisabled(true)
                .moveDisabled(false)
                .selectionDisabled(true)
                .listRowSpacing(6)
            Text("group")
                .geometryGroup()
                .invalidatableContent()
                .interactionActivityTrackingTag("group-tag")
        }
        .listSectionSpacing(12)
    }
}
