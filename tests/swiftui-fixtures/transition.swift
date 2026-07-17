import SwiftUI

// UI9 — AnyTransition factories and combinators. Exercises the `AnyTransition`
// value type end to end via `.transition(_:)`: the built-in curves (opacity,
// slide, scale, identity), parameterized forms (scale(anchor:), move, push,
// offset), the combinators (`combined(with:)`, `asymmetric(insertion:removal:)`)
// and the `.animation(_:)` attachment.
struct V: View {
    var body: some View {
        VStack(spacing: 8) {
            Text("opacity").transition(.opacity)
            Text("slide").transition(.slide)
            Text("scale").transition(.scale)
            Text("scaleAnchor").transition(.scale(scale: 0.5, anchor: .topLeading))
            Text("move").transition(.move(edge: .bottom))
            Text("push").transition(.push(from: .trailing))
            Text("offset").transition(.offset(x: 20, y: 10))
            Text("identity").transition(.identity)
            Text("combined").transition(.opacity.combined(with: .slide))
            Text("asymmetric").transition(.asymmetric(insertion: .scale, removal: .opacity))
            Text("animated").transition(.slide.animation(.easeInOut(duration: 0.3)))
        }
    }
}
