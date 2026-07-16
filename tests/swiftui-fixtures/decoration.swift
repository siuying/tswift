// C4 — visual decoration modifiers: border, shadow, clipShape, clipped
// (issue #191). background(_ view)/overlay deferred (arbitrary-view compositing).
import SwiftUI

struct V: View {
    var body: some View {
        VStack(spacing: 16) {
            Text("Bordered")
                .padding()
                .border(.blue, width: 2)
            Text("Shadowed")
                .padding()
                .background(.white)
                .shadow(color: .gray, radius: 6, x: 0, y: 3)
            Text("Clipped")
                .frame(width: 80, height: 80)
                .background(.indigo)
                .clipShape(Circle())
            Text("A very long line that should be cut off by clipped")
                .frame(width: 80, height: 24)
                .clipped()
        }
    }
}
