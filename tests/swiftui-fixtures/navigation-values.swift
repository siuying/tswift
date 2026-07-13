// Value-based navigation (ADR-0013 §1, slice 5): a `NavigationLink(value:)`
// resolves the enclosing `.navigationDestination(for:)`; a `NavigationPath`
// bound via `NavigationStack(path:)` drives the stack depth, so both a value
// link and a programmatic `path.append(_)` push, and `back` pops the path.
import SwiftUI

struct RootView: View {
    @State private var path = NavigationPath()
    var body: some View {
        NavigationStack(path: $path) {
            VStack {
                Text("Home")
                NavigationLink("Go to 1", value: 1)
                Button("Push 2") { path.append(2) }
            }
            .navigationDestination(for: Int.self) { n in
                Text("Detail \(n)").navigationTitle("Item \(n)")
            }
        }
    }
}
