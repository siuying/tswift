// List tab — a keyed ForEach whose row order is driven by @State, so a reorder
// reconciles with `move` patches rather than rebuilding the rows.
struct ListView: View {
    @State var fruits = ["Apple", "Banana", "Cherry"]

    var body: some View {
        VStack {
            ForEach(fruits, id: \.self) { fruit in
                Text(fruit)
                    .padding()
            }
            Button("Reverse") { fruits = ["Cherry", "Banana", "Apple"] }
        }
    }
}
