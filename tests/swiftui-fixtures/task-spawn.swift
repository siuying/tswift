import SwiftUI

struct TaskSpawnView: View {
    @State private var label = "Idle"

    func load() async -> String { "Finished" }

    var body: some View {
        VStack {
            Text(label)
            Button("Run") {
                Task { label = await load() }
            }
        }
    }
}
