// Gesture-composition, rename-action, and @ViewBuilder-content modifiers.
// - highPriorityGesture / simultaneousGesture lower to the same marker+handler
//   route as .gesture(_:) (onTapGesture / onLongPressGesture markers);
// - renameAction records a bare marker + stashed closure (recorded-only);
// - toolbarTitleMenu / sectionActions lower their @ViewBuilder to a nested
//   child subtree.
struct V: View {
    var body: some View {
        Text("hi")
            .highPriorityGesture(TapGesture().onEnded { })
            .simultaneousGesture(LongPressGesture().onEnded { _ in })
            .renameAction { }
            .toolbarTitleMenu {
                Button("Rename") {}
            }
            .sectionActions {
                Button("Add") {}
            }
    }
}
