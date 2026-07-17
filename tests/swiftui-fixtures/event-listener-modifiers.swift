import SwiftUI

struct EventListenerModifiers: View {
    var body: some View {
        VStack {
            Text("Events")
                .onKeyPress { _ in .handled }
                .onContinueUserActivity("com.example.activity") { _ in }
                .onScrollTargetVisibilityChange(idType: Int.self, threshold: 0.5) { _ in }
            Text("Drops")
                .onDrop(of: ["public.text"], isTargeted: nil) { _, _ in true }
                .onDragSessionUpdated { _ in }
                .onDropSessionUpdated { _ in }
            Text("Platform")
                .onInteractiveResizeChange { _ in }
                .onLongTouchGesture { }
                .onVolumeViewpointChange { _, _ in }
                .onWorldRecenter { }
        }
    }
}
