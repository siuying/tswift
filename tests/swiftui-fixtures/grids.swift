// C6 — lazy stacks, Grid/GridRow, Form (issue #193).
// LazyVGrid/LazyHGrid deferred (need GridItem + array-valued arg serialization).
struct V: View {
    var body: some View {
        Form {
            LazyVStack(spacing: 6) {
                Text("Row one")
                Text("Row two")
            }
            Grid {
                GridRow {
                    Text("0,0")
                    Text("0,1")
                }
                GridRow {
                    Text("1,0")
                    Text("1,1")
                }
            }
        }
    }
}
