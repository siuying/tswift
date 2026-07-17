import SwiftUI

struct ScopedValueModifiers: View {
    var body: some View {
        VStack {
            Text("Env")
                .environment(\.lineLimit, 3)
            Text("Focus")
                .focusedValue(\.selectedCount, 7)
                .focusedSceneValue(\.selectedCount, 9)
            Text("Container")
                .containerValue(\.spacing, 12)
        }
    }
}
