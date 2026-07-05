import SwiftUI
import TSwiftCore

// MARK: - Runner

/// Executes Swift source on a background thread and publishes the result.
@MainActor
final class ConsoleRunner: ObservableObject {
    @Published var output: String = ""
    @Published var diagnostics: String = ""
    @Published var isOk: Bool = true
    @Published var isRunning: Bool = false

    func run(source: String) {
        guard !isRunning else { return }
        isRunning = true
        output = ""
        diagnostics = ""

        // Capture before hopping threads so the compiler sees a Sendable String.
        let src = source
        Task.detached(priority: .userInitiated) {
            let result = TSwiftCore.run(src)
            await MainActor.run { [self] in
                self.output = result.stdout
                self.diagnostics = result.diagnostics
                self.isOk = result.ok
                self.isRunning = false
            }
        }
    }
}

// MARK: - View

/// Displays a "Run" button and the stdout / diagnostics from `TSwiftCore.run`.
/// Auto-runs once on first appearance.
struct ConsoleDemoView: View {
    /// The Swift source to execute.  Updated by the parent's TextEditor.
    let source: String

    @StateObject private var runner = ConsoleRunner()

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // ── Toolbar ──────────────────────────────────────────────────
            HStack {
                Text("Output")
                    .font(.caption.bold())
                    .foregroundStyle(.secondary)
                Spacer()
                if runner.isRunning {
                    ProgressView()
                        .scaleEffect(0.75)
                }
                Button("Run") {
                    runner.run(source: source)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
                .disabled(runner.isRunning)
            }
            .padding(.horizontal)
            .padding(.vertical, 8)

            Divider()

            // ── Output area ───────────────────────────────────────────────
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    if !runner.output.isEmpty {
                        Text(runner.output)
                            .font(.system(.body, design: .monospaced))
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    if !runner.diagnostics.isEmpty {
                        Text(runner.diagnostics)
                            .font(.system(.caption, design: .monospaced))
                            .foregroundStyle(runner.isOk ? Color.secondary : Color.red)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    if runner.output.isEmpty && runner.diagnostics.isEmpty && !runner.isRunning {
                        Text(" ")   // keep minimum height while idle
                            .font(.system(.body, design: .monospaced))
                    }
                }
                .padding()
            }
            .background(Color(.secondarySystemBackground))
        }
        .onAppear {
            runner.run(source: source)
        }
    }
}
