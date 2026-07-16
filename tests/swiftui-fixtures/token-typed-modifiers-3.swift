// Third typed-seam token batch: text-content/selection, scroll input, dialog
// severity, default hover effect (reusing HoverEffect) and presentation drag
// indicator (reusing Visibility).
struct V: View {
    var body: some View {
        VStack {
            TextField("user", text: .constant(""))
                .textContentType(.username)
                .textSelectionAffinity(.downstream)
            ScrollView {
                Text("s")
            }
            .scrollInputBehavior(.enabled)
            Text("hover")
                .defaultHoverEffect(.highlight)
            Text("sheet")
                .presentationDragIndicator(.visible)
            Text("dialog")
                .dialogSeverity(.critical)
        }
    }
}
