import SwiftUI

// Token-valued modifiers resolved via the typed seam: each leading-dot arg
// resolves against a dedicated parameter type, so shared names like
// .automatic / .fill / .circle never collide with other namespaces.
struct V: View {
    var body: some View {
        VStack {
            Text("scheme")
                .colorScheme(.dark)
                .preferredColorScheme(.light)
            Image(systemName: "star")
                .symbolVariant(.fill)
            Text("hover")
                .hoverEffect(.lift)
            Text("menu")
                .menuOrder(.fixed)
            Text("trans")
                .contentTransition(.numericText)
            ScrollView {
                Text("s")
            }
            .scrollBounceBehavior(.basedOnSize)
            .scrollDismissesKeyboard(.interactively)
            Text("size")
                .dynamicTypeSize(.xLarge)
        }
    }
}
