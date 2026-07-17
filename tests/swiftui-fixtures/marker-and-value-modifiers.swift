import SwiftUI

// No-arg marker modifiers (all-defaulted overloads → bare call) and
// single-value passthroughs that carry no leading-dot token.
struct V: View {
    var body: some View {
        VStack {
            Text("eq")
                .equatable()
            Text("focus")
                .focusSection()
            Text("safe")
                .ignoresSafeArea()
            ScrollView {
                Text("scroll")
            }
            .coordinateSpace(name: "scrollArea")
            Text("drag")
                .draggable("payload")
        }
    }
}
