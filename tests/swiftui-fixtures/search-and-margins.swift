import SwiftUI

// Search / status-bar / section-margin / glass token & value modifiers. Each
// records its real value (String, Bool, Edge.Set + CGFloat, Glass token +
// nested shape) onto the view node.
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("search")
                .searchCompletion("apple pie")
                .statusBar(hidden: true)
            Text("margins")
                .listSectionMargins(.horizontal, 12)
                .glassEffect(.regular, in: Capsule())
        }
    }
}
