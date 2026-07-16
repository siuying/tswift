// Token modifiers with dedicated namespaces (alternatingRowBackgrounds,
// buttonSizing, defaultAdaptableTabBarPlacement, tabBarMinimizeBehavior,
// searchPresentationToolbarBehavior, searchToolbarBehavior,
// handGestureShortcut), a multi-token style + `for:` Edge.Set
// (scrollEdgeEffectStyle), a Color + `for:` ToolbarPlacement
// (toolbarForegroundStyle), and two no-arg markers
// (horizontalRadioGroupLayout, backgroundExtensionEffect).
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("rows")
                .alternatingRowBackgrounds(.enabled)
                .buttonSizing(.flexible)
                .defaultAdaptableTabBarPlacement(.sidebarAdaptable)
            Text("tabs")
                .tabBarMinimizeBehavior(.onScrollDown)
                .searchPresentationToolbarBehavior(.avoidHidingContent)
                .searchToolbarBehavior(.minimize)
            Text("edge")
                .handGestureShortcut(.primaryAction)
                .scrollEdgeEffectStyle(.soft, for: .top)
                .toolbarForegroundStyle(Color.red, for: .navigationBar)
            Text("marker")
                .horizontalRadioGroupLayout()
                .backgroundExtensionEffect()
        }
    }
}
