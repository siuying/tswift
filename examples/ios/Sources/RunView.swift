import SwiftUI
import TSwiftCore

/// CodeSandbox-style: edit a Swift program, run it, see stdout / diagnostics.
struct RunView: View {
    @State private var source = #"print("Hello from tswift")"#
    @State private var result: TSwiftCore.RunResult?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Source").font(.headline)
            TextEditor(text: $source)
                .font(.system(.body, design: .monospaced))
                .frame(height: 180)
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(.gray.opacity(0.4)))

            Button("Run") { result = TSwiftCore.run(source) }
                .buttonStyle(.borderedProminent)

            if let result {
                Text(result.ok ? "Output" : "Error")
                    .font(.headline)
                    .foregroundStyle(result.ok ? Color.primary : Color.red)
                ScrollView {
                    Text(result.ok ? result.stdout : result.diagnostics)
                        .font(.system(.body, design: .monospaced))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .textSelection(.enabled)
                }
            }
            Spacer()
        }
        .padding()
    }
}
