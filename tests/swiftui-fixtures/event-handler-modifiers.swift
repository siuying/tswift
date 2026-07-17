import SwiftUI

// Single-closure event-handler modifiers: each records a marker; the closure
// is bound under a distinct handler key and never serializes (only the marker
// reaches the UIIR).
struct V: View {
    var body: some View {
        VStack {
            Text("hover")
                .onHover { hovering in
                    print(hovering)
                }
            Text("url")
                .onOpenURL { url in
                    print(url)
                }
            List {
                Text("row")
            }
            .refreshable {
                print("refresh")
            }
            Text("cmd")
                .onDeleteCommand {
                    print("delete")
                }
                .onExitCommand {
                    print("exit")
                }
                .onPlayPauseCommand {
                    print("play")
                }
            Text("drag")
                .onDrag {
                    "payload"
                }
        }
    }
}
