import SwiftUI

// Prominence + button-border-shape token modifiers, all sharing the
// `_ControlStyle` leading-dot namespace; the host disambiguates by modifier
// name.
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Section("header") {
                Text("row")
            }
            .headerProminence(.increased)
            Text("badge")
                .badgeProminence(.decreased)
            Button("tap") {}
                .buttonBorderShape(.roundedRectangle)
            Button("cap") {}
                .buttonBorderShape(.capsule)
        }
    }
}
