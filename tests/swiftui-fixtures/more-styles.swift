import SwiftUI

// Additional style setters sharing the `_ControlStyle` token namespace. Each
// carries a leading-dot token; the host disambiguates by the modifier name.
// `.linear` is intentionally avoided for progressViewStyle (it collides with
// Animation.linear); `.circular` is used instead.
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Label("label", systemImage: "star")
                .labelStyle(.iconOnly)
            ProgressView(value: 0.5)
                .progressViewStyle(.circular)
            TextField("editor", text: .constant("x"))
                .textEditorStyle(.plain)
        }
        .tableStyle(.inset)
        .navigationViewStyle(.stack)
        .navigationSplitViewStyle(.balanced)
    }
}
