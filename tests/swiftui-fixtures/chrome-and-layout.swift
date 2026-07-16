// Visibility-token chrome modifiers (Visibility `.visible` / `.hidden`) and
// scalar layout modifiers (Int span / CGFloat width/spacing/height), all
// recorded straight onto the view node.
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("chrome")
                .persistentSystemOverlays(.hidden)
                .menuIndicator(.visible)
                .listSectionIndexVisibility(.hidden)
                .navigationLinkIndicatorVisibility(.visible)
            Text("layout")
                .gridCellColumns(2)
                .labelIconToTitleSpacing(6)
                .labelReservedIconWidth(20)
                .inspectorColumnWidth(200)
                .navigationSplitViewColumnWidth(180)
                .defaultWheelPickerItemHeight(32)
        }
    }
}
