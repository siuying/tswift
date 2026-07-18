import SwiftUI

struct ModifierClosureDispatchView: View {
    @State private var longPresses = 0
    @State private var drags = 0
    @State private var refreshes = 0
    @State private var lifecycle = "idle"
    @State private var name = ""
    @State private var changed = "none"

    var body: some View {
        VStack {
            Text("long=\(longPresses) drag=\(drags) refresh=\(refreshes) life=\(lifecycle) change=\(changed)")
            Text("Hold")
                .onLongPressGesture { longPresses += 1 }
            Text("Drag")
                .onDrag {
                    drags += 1
                    return "payload"
                }
            Text("Refresh")
                .refreshable { refreshes += 1 }
            Text("Lifecycle")
                .onAppear { lifecycle = "appeared" }
                .task { lifecycle = "task" }
                .onDisappear { lifecycle = "disappeared" }
            TextField("Name", text: $name)
                .onSubmit { lifecycle = "submitted:\(name)" }
        }
        .onChange(of: longPresses) { old, new in changed = "\(old)->\(new)" }
    }
}
