import SwiftUI

// Search recording modifiers. Verifies:
// - searchable(text:placement:prompt:) snapshots the bound query text and
//   records the SearchFieldPlacement token + prompt;
// - searchScopes(_:activation:scopes:) snapshots the scope selection, records
//   the SearchScopeActivation token, and lowers the @ViewBuilder scope list;
// - searchSuggestions { } lowers a @ViewBuilder suggestion subtree;
// - searchFocused(_:) / searchSelection(_:) snapshot their bindings.
struct V: View {
    @State private var query = "cat"
    @State private var scope = "all"
    @State private var focused = false
    @State private var selection = "a"
    var body: some View {
        NavigationStack {
            List {
                Text("row")
            }
            .searchable(text: $query, placement: .toolbar, prompt: "Search")
            .searchScopes($scope, activation: .onTextEntry) {
                Text("All").tag("all")
                Text("Mine").tag("mine")
            }
            .searchSuggestions {
                Text("recent")
            }
            .searchFocused($focused)
            .searchSelection($selection)
        }
    }
}
