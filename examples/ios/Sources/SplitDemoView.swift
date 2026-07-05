import SwiftUI

/// A reusable split container: editable source on the left (or top tab),
/// a generic demo view on the right (or bottom tab).
///
/// - Regular width (iPad / landscape): side-by-side HStack with Divider.
/// - Compact width (iPhone portrait): segmented Picker at the top;
///   only the selected pane is shown. Defaults to "Demo".
struct SplitDemoView<Demo: View>: View {
    @Binding var source: String
    private let demo: () -> Demo

    @Environment(\.horizontalSizeClass) private var sizeClass

    private enum Mode: String, CaseIterable {
        case demo = "Demo"
        case code = "Code"
    }

    @State private var mode: Mode = .demo

    init(source: Binding<String>, @ViewBuilder demo: @escaping () -> Demo) {
        self._source = source
        self.demo = demo
    }

    var body: some View {
        if sizeClass == .compact {
            compactLayout
        } else {
            regularLayout
        }
    }

    // MARK: - Regular (side-by-side)

    private var regularLayout: some View {
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

    // MARK: - Compact (segmented picker)

    private var compactLayout: some View {
        VStack(spacing: 0) {
            Picker("View", selection: $mode) {
                ForEach(Mode.allCases, id: \.self) { m in
                    Text(m.rawValue).tag(m)
                }
            }
            .pickerStyle(.segmented)
            .padding(.horizontal)
            .padding(.vertical, 8)

            Divider()

            switch mode {
            case .code:
                VStack(alignment: .leading, spacing: 0) {
                    paneHeader("Source")
                    Divider()
                    TextEditor(text: $source)
                        .font(.system(.body, design: .monospaced))
                        .autocorrectionDisabled()
                        .textInputAutocapitalization(.never)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)

            case .demo:
                VStack(alignment: .leading, spacing: 0) {
                    demo()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
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
