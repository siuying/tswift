// Nested-subtree View modifiers: `.contentShape` (hit-test shape, recorded like
// `clipShape`), `.swipeActions` (row action buttons as a nested subtree),
// `.safeAreaInset`/`.safeAreaBar` (edge-pinned inset/bar views), and
// `.inspector(isPresented:)` (a binding-gated pane realized as a `Presentation`
// child, closed here so no pane renders).
struct V: View {
    @State private var showInspector = false

    var body: some View {
        VStack(spacing: 8) {
            Text("Row")
                .contentShape(Rectangle())
                .swipeActions {
                    Button("Delete") {}
                }
            Text("Body")
                .safeAreaInset(edge: .bottom) {
                    Text("Inset")
                }
                .safeAreaBar(edge: .top) {
                    Text("Bar")
                }
        }
        .inspector(isPresented: $showInspector) {
            Text("Inspector")
        }
    }
}
