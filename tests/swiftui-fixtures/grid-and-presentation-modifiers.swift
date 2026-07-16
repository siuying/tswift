// Grid cell/column layout modifiers reusing UnitPoint / HorizontalAlignment /
// Axis.Set namespaces, scroll/presentation/material/palette token modifiers
// with dedicated namespaces, and Color value passthroughs (listItemTint,
// listRowPlatterColor). Verifies each namespace resolves without collisions.
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("grid")
                .defaultScrollAnchor(.top)
                .gridCellAnchor(.topLeading)
                .gridColumnAlignment(.leading)
                .gridCellUnsizedAxes(.horizontal)
            Text("present")
                .presentationBackgroundInteraction(.enabled)
                .presentationCompactAdaptation(.sheet)
                .scrollTargetBehavior(.paging)
            Text("style")
                .materialActiveAppearance(.inactive)
                .paletteSelectionEffect(.symbolVariant)
                .writingToolsAffordanceVisibility(.hidden)
            Text("tint")
                .listItemTint(Color.blue)
                .listRowPlatterColor(Color.green)
        }
    }
}
