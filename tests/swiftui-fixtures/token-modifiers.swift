// C7 — token-enum view modifiers. Verifies the modifiers that carry a typed
// leading-dot token: blendMode (BlendMode), controlSize (ControlSize),
// symbolRenderingMode (SymbolRenderingMode), redacted(reason:)
// (RedactionReasons), and truncationMode (TruncationMode).
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("blend").blendMode(.multiply)
            Text("overlay").blendMode(.overlay)
            Text("size").controlSize(.small)
            Text("symbol").symbolRenderingMode(.hierarchical)
            Text("redacted").redacted(reason: .placeholder)
            Text("truncate").truncationMode(.middle)
        }
    }
}
