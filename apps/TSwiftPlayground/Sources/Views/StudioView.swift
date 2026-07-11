import SwiftUI
import TSwiftUI
import UiirRenderer

/// The mini-IDE for one project: a file/symbol sidebar (a `NavigationSplitView`
/// column — a sidebar on iPad, a slide-over drawer on iPhone), a Runestone code
/// editor, and a run pane that shows either a live SwiftUI preview or a console
/// output pane.
struct StudioView: View {
    @StateObject var model: StudioModel
    @State private var columnVisibility: NavigationSplitViewVisibility = .automatic

    var body: some View {
        NavigationSplitView(columnVisibility: $columnVisibility) {
            SidebarView(model: model)
                .navigationTitle(model.projectName)
        } detail: {
            editorAndOutput
                .navigationTitle(model.selectedFileName)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar { runToolbar }
        }
        .onDisappear { model.saveNow() }
    }

    // MARK: Editor + output

    private var editorAndOutput: some View {
        VStack(spacing: 0) {
            CodeEditor(
                text: model.selectedText,
                diagnostics: model.session.diagnostics,
                jumpTarget: model.jumpTarget
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            if !model.session.diagnostics.isEmpty {
                DiagnosticsList(diagnostics: model.session.diagnostics)
            }

            Divider()

            outputPane
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    @ViewBuilder
    private var outputPane: some View {
        switch model.mode {
        case .preview:
            VStack(spacing: 0) {
                if let error = model.session.lastError {
                    ErrorBanner(message: error)
                }
                RenderHostView(model: model.session.model)
                    .uiirEventSink(model.session.makeEventSink())
            }
        case .console:
            ConsoleView(text: model.consoleOutput, isError: model.consoleIsError)
        }
    }

    // MARK: Toolbar

    @ToolbarContentBuilder
    private var runToolbar: some ToolbarContent {
        ToolbarItem(placement: .topBarTrailing) {
            Picker("Mode", selection: $model.mode) {
                ForEach(RunMode.allCases) { mode in
                    Label(mode.label, systemImage: mode.systemImage).tag(mode)
                }
            }
            .pickerStyle(.segmented)
            .onChange(of: model.mode) { _ in model.recomputeNow() }
        }
        ToolbarItem(placement: .topBarTrailing) {
            Button {
                model.run()
            } label: {
                Image(systemName: "play.fill")
            }
        }
    }
}

/// The diagnostics strip under the editor (tappable rows, capped height).
struct DiagnosticsList: View {
    let diagnostics: [PreviewSession.Diagnostic]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(diagnostics) { d in
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
        .frame(maxHeight: 110)
        .background(Color(.secondarySystemBackground))
    }
}

/// A red banner surfacing a compile/dispatch error over the preview.
struct ErrorBanner: View {
    let message: String
    var body: some View {
        Text(message)
            .font(.footnote.monospaced())
            .foregroundStyle(.white)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(8)
            .background(Color.red.opacity(0.85))
    }
}

/// A scrollable monospaced console for captured stdout (or a run error).
struct ConsoleView: View {
    let text: String
    let isError: Bool

    var body: some View {
        ScrollView {
            Text(text.isEmpty ? "Run to see output." : text)
                .font(.callout.monospaced())
                .foregroundStyle(isError ? Color.red : Color.primary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
                .padding(10)
        }
        .background(Color(.systemBackground))
    }
}
