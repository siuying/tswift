import SwiftUI

@main
struct TSwiftExampleApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}

// MARK: - Root split-view shell

struct ContentView: View {
    @State private var selectedItem: CatalogItem?

    var body: some View {
        NavigationSplitView {
            List(selection: $selectedItem) {
                ForEach(Catalog.all) { group in
                    Section(group.name) {
                        ForEach(group.items) { item in
                            NavigationLink(value: item) {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(item.title)
                                        .font(.body)
                                    Text(item.subtitle)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
                }
            }
            .navigationTitle("tswift")
        } detail: {
            if let item = selectedItem {
                CatalogDetailView(item: item)
            } else {
                VStack(spacing: 12) {
                    Image(systemName: "chevron.left")
                        .font(.largeTitle)
                        .foregroundStyle(.secondary)
                    Text("Select an example")
                        .font(.title3.bold())
                    Text("Choose a Swift feature from the sidebar.")
                        .foregroundStyle(.secondary)
                }
            }
        }
    }
}
