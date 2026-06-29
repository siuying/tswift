import SwiftUI
import TSwiftUI
import UiirRenderer

/// The product playground: edit Swift in a Runestone `CodeEditor` (top) and see
/// a live, interactive SwiftUI preview (bottom). Typing debounces a recompile;
/// preview interactions round-trip through the native `PreviewSession`.
struct PlaygroundView: View {
    @StateObject private var session = PreviewSession()
    @State private var source = Samples.initial.code
    @State private var recompileTask: Task<Void, Never>?

    /// How long to wait after the last keystroke before recompiling.
    private static let debounce: Duration = .milliseconds(250)

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                CodeEditor(text: $source, diagnostics: session.diagnostics)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)

                if !session.diagnostics.isEmpty {
                    diagnosticsList
                }

                Divider()

                preview
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .navigationTitle("Playground")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    samplesMenu
                }
            }
            .onChange(of: source) { _ in scheduleRecompute() }
            .onAppear { computeNow() }
        }
    }

    // MARK: Preview pane

    @ViewBuilder
    private var preview: some View {
        VStack(spacing: 0) {
            if let error = session.lastError {
                errorBanner(error)
            }
            RenderHostView(model: session.model)
                .uiirEventSink(session.makeEventSink())
        }
    }

    private func errorBanner(_ message: String) -> some View {
        Text(message)
            .font(.footnote.monospaced())
            .foregroundStyle(.white)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(8)
            .background(Color.red.opacity(0.85))
    }

    // MARK: Diagnostics list

    /// The frontend's syntax/sema diagnostics, one tappable row each
    /// (`⚠︎ Ln:Col message`), capped in height so it never crowds the preview.
    private var diagnosticsList: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(session.diagnostics) { d in
                    HStack(alignment: .top, spacing: 6) {
                        Image(systemName: d.isError
                            ? "xmark.octagon.fill" : "exclamationmark.triangle.fill")
                            .foregroundStyle(d.isError ? .red : .orange)
                            .font(.caption2)
                        Text("\(d.line):\(d.col)")
                            .foregroundStyle(.secondary)
                            .font(.caption2.monospaced())
                        Text(d.message)
                            .font(.caption2.monospaced())
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }
            .padding(8)
        }
        .frame(maxHeight: 120)
        .background(Color(.secondarySystemBackground))
    }

    // MARK: Samples

    private var samplesMenu: some View {
        Menu {
            ForEach(Samples.all) { sample in
                Button(sample.label) { load(sample) }
            }
        } label: {
            Label("Samples", systemImage: "square.grid.2x2")
        }
    }

    private func load(_ sample: Sample) {
        source = sample.code
        // `onChange` will schedule the recompile.
    }

    // MARK: Debounced compile

    private func scheduleRecompute() {
        recompileTask?.cancel()
        let current = source
        recompileTask = Task {
            try? await Task.sleep(for: Self.debounce)
            guard !Task.isCancelled else { return }
            compute(current)
        }
    }

    private func computeNow() {
        recompileTask?.cancel()
        compute(source)
    }

    /// Lint (for inline error feedback) and recompile (for the live preview) the
    /// same source in lockstep — both read the frontend, so they stay consistent.
    private func compute(_ current: String) {
        session.diagnose(current)
        session.compile(current)
    }
}
