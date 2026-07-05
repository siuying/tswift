import SwiftUI

/// A reusable side-by-side split container: editable source on the left,
/// a generic demo view on the right.
///
/// - Parameters:
///   - source: Binding to the Swift source text shown in the left pane.
///   - demo: ViewBuilder that produces the right-pane demo content.
///
/// Adaptive / compact-width behaviour is deferred to slice 4; for now the
/// layout is always side-by-side (HStack + Divider).
struct SplitDemoView<Demo: View>: View {
    @Binding var source: String
    private let demo: () -> Demo

    init(source: Binding<String>, @ViewBuilder demo: @escaping () -> Demo) {
        self._source = source
        self.demo = demo
    }

    var body: some View {
        HStack(spacing: 0) {
            // ── Left: editable source ────────────────────────────────────
            VStack(alignment: .leading, spacing: 0) {
                paneHeader("Source")
                Divider()
                TextEditor(text: $source)
                    .font(.system(.body, design: .monospaced))
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            Divider()

            // ── Right: demo ──────────────────────────────────────────────
            VStack(alignment: .leading, spacing: 0) {
                demo()
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    // MARK: - Helpers

    private func paneHeader(_ title: String) -> some View {
        Text(title)
            .font(.caption.bold())
            .foregroundStyle(.secondary)
            .padding(.horizontal)
            .padding(.vertical, 8)
    }
}
