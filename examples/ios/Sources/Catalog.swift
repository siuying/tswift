import Foundation

// MARK: - Demo kind

/// Discriminates how a catalog item is executed/rendered.
enum DemoKind: Hashable {
    case console
    case swiftUI(needsNetwork: Bool)
}

// MARK: - Catalog item

/// A single runnable Swift example.
struct CatalogItem: Identifiable, Hashable {
    let id: String          // stable, human-readable slug
    let title: String
    let subtitle: String
    let source: String      // Swift source code shown on the left panel
    let kind: DemoKind
}

// MARK: - Catalog group

/// A named collection of related items shown as one sidebar section.
struct CatalogGroup: Identifiable {
    let id: String
    let name: String
    let items: [CatalogItem]
}

// MARK: - Static catalog

/// The full catalog.  Real content is added in later slices; this slice
/// provides the minimum needed to prove navigation end-to-end.
enum Catalog {
    static let all: [CatalogGroup] = [basicsGroup, swiftUIGroup]

    // MARK: Basics
    private static let basicsGroup = CatalogGroup(
        id: "basics",
        name: "Basics",
        items: [
            CatalogItem(
                id: "basics-hello-world",
                title: "Hello World",
                subtitle: "Print to console",
                source: """
                print("Hello from tswift")
                """,
                kind: .console
            ),
        ]
    )

    // MARK: SwiftUI
    private static let swiftUIGroup = CatalogGroup(
        id: "swiftui",
        name: "SwiftUI",
        items: [
            CatalogItem(
                id: "swiftui-counter",
                title: "Counter",
                subtitle: "Stateful button",
                source: """
                import SwiftUI

                struct CounterView: View {
                    @State private var count = 0

                    var body: some View {
                        VStack(spacing: 16) {
                            Text("Count: \\(count)")
                                .font(.largeTitle)
                            Button("Tap me") { count += 1 }
                                .buttonStyle(.borderedProminent)
                        }
                    }
                }
                """,
                kind: .swiftUI(needsNetwork: false)
            ),
        ]
    )
}
