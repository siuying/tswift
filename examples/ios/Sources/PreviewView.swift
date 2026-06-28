import SwiftUI
import TSwiftUI
import UiirRenderer

/// Playground-style: edit a SwiftUI program, compile it, and interact with the
/// live preview (taps/controls round-trip through the native render session).
struct PreviewView: View {
    @StateObject private var session = PreviewSession()
    @State private var didCompile = false
    @State private var source = """
    struct CounterView: View {
        @State private var count = 0
        var body: some View {
            VStack(spacing: 16) {
                Text("Count: \\(count)").font(.largeTitle)
                Button("Increment") { count += 1 }
            }
        }
    }
    """

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("SwiftUI source").font(.headline)
            TextEditor(text: $source)
                .font(.system(.body, design: .monospaced))
                .frame(height: 160)
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(.gray.opacity(0.4)))

            Button("Compile & Preview") { session.compile(source) }
                .buttonStyle(.borderedProminent)

            if let error = session.lastError {
                Text(error).foregroundStyle(.red).font(.footnote)
            }

            Divider()
            Text("Live preview").font(.headline)
            RenderHostView(model: session.model)
                .uiirEventSink(session.makeEventSink())
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(.gray.opacity(0.4)))
            Spacer()
        }
        .padding()
        .onAppear {
            // Compile once on first appearance; re-appearing the tab must not
            // reset interaction state.
            guard !didCompile else { return }
            didCompile = true
            session.compile(source)
        }
    }
}
