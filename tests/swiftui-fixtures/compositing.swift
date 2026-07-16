// C4 arbitrary-view compositing (issue #204): background(_ view) and
// overlay(_ view, alignment:) where the value is a nested view subtree with its
// own 0-rooted id space. `alignment:` resolves via the typed-token shims (#203).
import SwiftUI

struct V: View {
    var body: some View {
        VStack(spacing: 20) {
            Text("Badge")
                .padding(12)
                .background(Circle().fill(.blue))
                .overlay(Text("9"), alignment: .topTrailing)
            Text("Framed")
                .padding(8)
                .overlay(alignment: .bottom) {
                    Rectangle().fill(.red).frame(height: 3)
                }
            Text("Tinted").padding(6).background(.yellow)
        }
    }
}
