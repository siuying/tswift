import SwiftUI

// Token modifiers introducing dedicated namespaces (each typed so its
// leading-dot member arg resolves contextually) plus `edgesIgnoringSafeArea`
// (reusing the Edge.Set namespace) and `backgroundStyle` (a Color value
// passthrough). Verifies the new namespaces resolve without cross-namespace
// token collisions.
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("nav")
                .navigationBarTitleDisplayMode(.inline)
                .toolbarTitleDisplayMode(.inlineLarge)
                .toolbarRole(.editor)
            Text("interaction")
                .springLoadingBehavior(.enabled)
                .layoutDirectionBehavior(.mirrors)
                .textSelection(.enabled)
            Text("preview")
                .previewLayout(.sizeThatFits)
                .previewInterfaceOrientation(.landscapeLeft)
            Text("symbol")
                .symbolColorRenderingMode(.gradient)
                .symbolVariableValueMode(.draw)
            Text("safe")
                .edgesIgnoringSafeArea(.all)
                .backgroundStyle(Color.gray)
        }
    }
}
