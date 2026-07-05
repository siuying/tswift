import SwiftUI

/// Placeholder detail panel shown when a catalog item is selected.
/// Slice 1 only: displays the item title, kind badge, and raw source.
/// The real split-screen demo (code on left, live output on right) is added
/// in later slices.
struct CatalogDetailView: View {
    let item: CatalogItem

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // ── Header ──────────────────────────────────────────────────────
            VStack(alignment: .leading, spacing: 4) {
                Text(item.title)
                    .font(.title2.bold())
                Text(item.subtitle)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                kindBadge
                    .padding(.top, 2)
            }
            .padding()

            Divider()

            // ── Source code ─────────────────────────────────────────────────
            ScrollView([.horizontal, .vertical]) {
                Text(item.source)
                    .font(.system(.body, design: .monospaced))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
            }
            .background(Color(.secondarySystemBackground))
        }
        .navigationTitle(item.title)
        .navigationBarTitleDisplayMode(.inline)
    }

    // MARK: - Helpers

    @ViewBuilder
    private var kindBadge: some View {
        switch item.kind {
        case .console:
            badge(label: "Console", color: .orange)
        case .swiftUI(let needsNetwork):
            badge(label: needsNetwork ? "SwiftUI + Network" : "SwiftUI", color: .blue)
        }
    }

    private func badge(label: String, color: Color) -> some View {
        Text(label)
            .font(.caption.bold())
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(color.opacity(0.15))
            .foregroundStyle(color)
            .clipShape(Capsule())
    }
}
