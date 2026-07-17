import SwiftUI

// Container / layout / bar-item recording modifiers. Verifies:
// - containerBackground(_:for:) with a ShapeStyle token + placement token, and
//   the containerBackground(for:) { content } @ViewBuilder form;
// - navigationBarItems(leading:trailing:) recording nested accessory views;
// - layoutValue(key:value:) and previewContext(_:) value passthroughs.
struct K: LayoutValueKey {
    static let defaultValue = 0
}
struct V: View {
    var body: some View {
        VStack {
            Text("home")
                .containerBackground(.blue, for: .navigation)
                .navigationBarItems(leading: Text("back"), trailing: Text("done"))
            Text("cell")
                .layoutValue(key: K.self, value: 3)
                .previewContext("preview")
            VStack {
                Text("panel")
            }
            .containerBackground(for: .window) {
                Text("backdrop")
            }
        }
    }
}
