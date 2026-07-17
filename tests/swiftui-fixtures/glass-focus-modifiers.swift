import SwiftUI

struct GlassFocusModifiers: View {
    @Namespace private var ns

    var body: some View {
        VStack {
            Text("Glass")
                .glassEffectID("hero", in: ns)
                .glassEffectUnion(id: "group", namespace: ns)
            Text("Focus")
                .focusScope(ns)
            Text("Storage")
                .defaultAppStorage(UserDefaults.standard)
        }
    }
}
