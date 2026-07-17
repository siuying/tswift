import SwiftUI

// More single-closure event handlers: edit/pencil/hover commands. Each records
// a marker; the closure binds under a distinct handler key and never
// serializes.
struct V: View {
    var body: some View {
        VStack {
            Text("edit")
                .onCutCommand {
                    print("cut")
                    return []
                }
                .onCopyCommand {
                    print("copy")
                    return []
                }
                .onMoveCommand { direction in
                    print(direction)
                }
            Text("pencil")
                .onPencilDoubleTap { value in
                    print(value)
                }
                .onPencilSqueeze { phase in
                    print(phase)
                }
            Text("hover")
                .onContinuousHover { phase in
                    print(phase)
                }
        }
    }
}
