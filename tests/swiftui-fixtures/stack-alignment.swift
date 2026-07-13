// C2 stack alignment (issue #189): VStack/HStack/ZStack honor `alignment:` on
// their cross axis, with the leading-dot token resolved against the right
// 1-D/2-D namespace (issue #203). Also exercises `Spacer(minLength:)` and
// `offset` alongside the alignment.
import SwiftUI

struct V: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Title")
            Text("A longer subtitle line")
            HStack(alignment: .bottom, spacing: 8) {
                Text("Left")
                Spacer(minLength: 16)
                Text("Right").offset(x: 0, y: -4)
            }
            ZStack(alignment: .bottomTrailing) {
                Rectangle().fill(.blue).frame(width: 80, height: 48)
                Text("Tag")
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
