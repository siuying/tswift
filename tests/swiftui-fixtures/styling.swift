// C7 — control styling + accessibility no-ops (issue #194).
import SwiftUI

struct V: View {
    @State private var name = ""
    var body: some View {
        VStack(spacing: 12) {
            Button("Prominent") { }
                .buttonStyle(.borderedProminent)
            Button("Plain") { }
                .buttonStyle(.plain)
                .disabled(false)
            Button("Disabled") { }
                .buttonStyle(.bordered)
                .disabled(true)
            TextField("Name", text: $name)
                .textFieldStyle(.roundedBorder)
                .accessibilityHint("enter your name")
                .accessibilityValue(name)
            List {
                Text("One")
                Text("Two")
            }
            .listStyle(.plain)
            .accessibilityLabel("items list")
            .accessibilityIdentifier("list-1")
        }
    }
}
