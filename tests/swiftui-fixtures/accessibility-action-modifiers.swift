import SwiftUI

struct AccessibilityActionModifiers: View {
    var body: some View {
        VStack {
            Text("Actions")
                .accessibilityAction { }
                .accessibilityAdjustableAction { _ in }
            Text("Scroll & Zoom")
                .accessibilityScrollAction { _ in }
                .accessibilityZoomAction { _ in }
            Text("Drop")
                .dropDestination(for: String.self) { _, _ in true }
        }
    }
}
