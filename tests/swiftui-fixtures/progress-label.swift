// C5 ProgressView label (issue #206): the title-string initializers carry a
// `label` arg the host renders alongside the bar — a determinate labelled bar,
// an indeterminate labelled spinner, and a bare value-only bar (no label).
struct V: View {
    var body: some View {
        VStack(spacing: 16) {
            ProgressView("Downloading", value: 0.45)
            ProgressView("Please wait")
            ProgressView(value: 0.8)
        }
    }
}
