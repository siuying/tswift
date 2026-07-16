// Presentation/search/window View modifiers. Token modifiers with dedicated
// namespaces (presentationContentInteraction, presentationSizing,
// searchDictationBehavior, windowToolbarFullScreenVisibility), a reused
// UnitPoint namespace (windowResizeAnchor), a leading Bool + `for:` Edge.Set
// (scrollEdgeEffectHidden), and two value passthroughs (presentationBackground
// Color, submitScope Bool).
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("sheet")
                .presentationContentInteraction(.scrolls)
                .presentationSizing(.form)
                .presentationBackground(Color.blue)
            Text("search")
                .searchDictationBehavior(.inactive)
                .submitScope(true)
            Text("window")
                .windowToolbarFullScreenVisibility(.onHover)
                .windowResizeAnchor(.topLeading)
                .scrollEdgeEffectHidden(true, for: .bottom)
        }
    }
}
