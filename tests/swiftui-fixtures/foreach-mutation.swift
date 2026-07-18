// Keyed ForEach identity: inserting and removing elements produces minimal
// keyed patches (insert/remove of the affected row's `\.self` key) rather than
// rebuilding every row. `id: \.self` gives each row a stable identity so the
// diff can target exactly the changed child.
import SwiftUI

struct RootView: View {
    @State private var items = ["a", "b", "c"]

    var body: some View {
        VStack {
            Button("Insert") { items.insert("z", at: 1) }
            Button("Remove") { items.removeLast() }
            ForEach(items, id: \.self) { item in
                Text(item)
            }
        }
    }
}
