import SwiftUI

// C6 — list & scroll styling modifiers. Verifies the no-arg render hints
// (compositingGroup, drawingGroup, unredacted), the Bool toggles
// (scrollClipDisabled, interactiveDismissDisabled, accessibilityHidden,
// flipsForRightToLeftLayoutDirection), the Visibility-token separators/scroll
// controls (listRowSeparator, listSectionSeparator, scrollContentBackground,
// scrollIndicators), and the Color separator tints.
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("grouped").compositingGroup().drawingGroup().unredacted()
            Text("toggles").scrollClipDisabled(true).interactiveDismissDisabled(true)
            Text("a11y").accessibilityHidden(true).flipsForRightToLeftLayoutDirection(false)
            List {
                Text("row")
                    .listRowSeparator(.hidden)
                    .listRowSeparatorTint(.red)
            }
            .listSectionSeparator(.visible)
            .listSectionSeparatorTint(.gray)
            .scrollContentBackground(.hidden)
            .scrollIndicators(.visible)
        }
    }
}
