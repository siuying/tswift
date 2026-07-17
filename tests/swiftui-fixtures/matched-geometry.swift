import SwiftUI

// Namespace geometry-identity modifiers (recorded-only). A `@Namespace` yields
// an opaque identity token (no layout engine to match geometry in a headless
// runtime), so `matchedGeometryEffect`/`matchedTransitionSource` record their
// `id:` (and `isSource:` when present) and drop the `in:` namespace.
struct V: View {
    @Namespace private var ns
    var body: some View {
        VStack {
            Text("source")
                .matchedGeometryEffect(id: "box", in: ns, isSource: true)
                .matchedTransitionSource(id: "row", in: ns)
            Text("target")
                .matchedGeometryEffect(id: "box", in: ns)
        }
    }
}
