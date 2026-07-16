// Toolbar bar-targeted modifiers (Visibility / ColorScheme token + a `for:`
// ToolbarPlacement bar selector) plus value passthroughs contentMargins
// (CGFloat) and previewDevice (String). Verifies the new ToolbarPlacement
// namespace resolves in the labeled `for:` position alongside the leading
// Visibility/ColorScheme token.
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("bars")
                .toolbarBackground(.hidden, for: .navigationBar)
                .toolbarBackgroundVisibility(.visible, for: .tabBar)
                .toolbarColorScheme(.dark, for: .navigationBar)
                .toolbarVisibility(.hidden, for: .bottomBar)
            Text("layout")
                .contentMargins(16)
                .previewDevice("iPhone 15")
        }
    }
}
