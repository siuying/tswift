import SwiftUI
import TSwiftUI
import UiirRenderer

/// Live SwiftUI preview pane: compiles the given source and renders the
/// resulting UIIR tree, round-tripping taps/controls through the render session.
///
/// Mirrors `ConsoleDemoView`'s layout (header + toolbar + output area) but the
/// "output" is an interactive `RenderHostView` instead of stdout text.
///
/// Compiles once on first appearance (guarded by `didCompile`) so re-appearing
/// the view does not reset interaction state (counters, toggles, …).
struct SwiftUIDemoView: View {
    /// The Swift source to compile.  Updated by the parent's TextEditor.
    let source: String

    @StateObject private var session = PreviewSession()
    @State private var didCompile = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // ── Toolbar ──────────────────────────────────────────────────
            HStack {
                Text("Preview")
                    .font(.caption.bold())
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Compile & Preview") {
                    session.compile(source)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
            }
            .padding(.horizontal)
            .padding(.vertical, 8)

            Divider()

            // ── Live preview ─────────────────────────────────────────────
            RenderHostView(model: session.model)
                .uiirEventSink(session.makeEventSink())
                .frame(maxWidth: .infinity, maxHeight: .infinity)

            if let error = session.lastError {
                Divider()
                Text(error)
                    .font(.footnote)
                    .foregroundStyle(Color.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal)
                    .padding(.vertical, 8)
            }
        }
        .background(Color(.secondarySystemBackground))
        .onAppear {
            // Compile once on first appearance; re-appearing must not reset
            // interaction state.
            guard !didCompile else { return }
            didCompile = true
            session.compile(source)
        }
    }
}
