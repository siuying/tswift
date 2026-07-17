import SwiftUI

// Scene/window presentation-style token modifiers plus a Bool passthrough.
// `.presentedWindowStyle` (WindowStyle token), `.presentedWindowToolbarStyle`
// (WindowToolbarStyle token), `.typesettingLanguage` (TypesettingLanguage
// token), `.digitalCrownAccessory` (a `Visibility` token, reusing that
// namespace), and `.touchBarItemPrincipal` (a plain `Bool`). All recorded
// straight onto their view node so the goldens exercise serialization.
struct V: View {
    var body: some View {
        VStack(spacing: 8) {
            Text("Window")
                .presentedWindowStyle(.plain)
                .presentedWindowToolbarStyle(.unified)
            Text("Language")
                .typesettingLanguage(.automatic)
            Text("Crown")
                .digitalCrownAccessory(.visible)
                .touchBarItemPrincipal(true)
        }
    }
}
