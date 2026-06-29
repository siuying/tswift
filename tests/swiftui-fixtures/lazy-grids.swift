// C6 lazy grids (issue #205): LazyVGrid/LazyHGrid driven by a `[GridItem]`
// track array. `.flexible()`/`.fixed(_)`/`.adaptive(minimum:)` resolve against
// the typed `columns:`/`rows:` signatures (#203) and serialize as a JSON array.
struct V: View {
    var body: some View {
        VStack(spacing: 16) {
            LazyVGrid(columns: [.flexible(), .fixed(60), .flexible()], spacing: 8) {
                Text("1")
                Text("2")
                Text("3")
                Text("4")
                Text("5")
                Text("6")
            }
            LazyHGrid(rows: [.fixed(40), .fixed(40)], spacing: 8) {
                Text("A")
                Text("B")
                Text("C")
                Text("D")
            }
        }
    }
}
