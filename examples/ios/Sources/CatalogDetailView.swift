import SwiftUI

/// Detail panel shown when a catalog item is selected.
///
/// - `.console` items: `SplitDemoView` with an editable `TextEditor` on the
///   left and `ConsoleDemoView` (stdout / diagnostics) on the right.
/// - `.swiftUI` items: placeholder — implemented in slice 3.
///
/// The `@State` source is seeded from `item.source` on first appearance and
/// reset whenever the selected item changes (via `.onChange(of:)`).
/// `.id(item.id)` on the split container recreates the demo pane (and its
/// `@StateObject ConsoleRunner`) so auto-run fires for each new selection.
struct CatalogDetailView: View {
    let item: CatalogItem

    /// Editable copy of the source code; lives in this view so the
    /// TextEditor binding doesn't reach into the immutable model.
    @State private var editableSource: String

    init(item: CatalogItem) {
        self.item = item
        // Seed the initial state value directly in the init so the very first
        // render shows the item's source without waiting for onAppear.
        self._editableSource = State(initialValue: item.source)
    }

    var body: some View {
        contentView
            .navigationTitle(item.title)
            .navigationBarTitleDisplayMode(.inline)
            // iOS 16-compatible single-param form; resets the editor when the
            // selected item changes in the sidebar.
            .onChange(of: item) { newItem in
                editableSource = newItem.source
            }
    }

    // MARK: - Content

    @ViewBuilder
    private var contentView: some View {
        switch item.kind {
        case .console:
            SplitDemoView(source: $editableSource) {
                ConsoleDemoView(source: editableSource)
            }
            // New identity per item ⟹ ConsoleRunner is recreated and
            // onAppear fires automatically, auto-running the new source.
            .id(item.id)

        case .swiftUI:
            swiftUIPlaceholder
        }
    }

    // MARK: - Placeholders

    private var swiftUIPlaceholder: some View {
        VStack(spacing: 12) {
            Image(systemName: "swift")
                .font(.system(size: 48))
                .foregroundStyle(.orange)
            Text("SwiftUI Demo")
                .font(.title3.bold())
            Text("Coming in slice 3.")
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
