// C3 — structural containers: Group, Divider, ScrollView (issue #190).
struct V: View {
    var body: some View {
        ScrollView {
            VStack(spacing: 12) {
                Group {
                    Text("Section one")
                        .font(.headline)
                    Text("first row")
                }
                Divider()
                Group {
                    Text("Section two")
                        .font(.headline)
                    Text("second row")
                }
            }
        }
    }
}
