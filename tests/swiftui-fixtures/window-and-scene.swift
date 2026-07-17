import SwiftUI

// Window / scene / container token modifiers. Verifies Edge.Set (scenePadding,
// defersSystemGestures), Axis.Set (containerRelativeFrame), Visibility
// (pointerVisibility) tokens reusing existing namespaces, and the four
// window-interaction behaviors carrying the WindowInteractionBehavior token.
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("scene")
                .scenePadding(.horizontal)
                .defersSystemGestures(edges: .bottom)
            Text("frame")
                .containerRelativeFrame(.horizontal)
                .pointerVisibility(.hidden)
            Text("window")
                .windowResizeBehavior(.disabled)
                .windowMinimizeBehavior(.enabled)
                .windowDismissBehavior(.automatic)
                .windowFullScreenBehavior(.disabled)
        }
    }
}
