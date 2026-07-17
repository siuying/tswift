// Container-corner / assistive-icon / section-index / hover-group / nav-
// transition modifiers. Each records its real value (Edge.Set token + Bool,
// String, no-arg, NavigationTransition token) onto the view node.
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("container")
                .containerCornerOffset(.horizontal, sizeToFit: true)
                .assistiveAccessNavigationIcon(systemImage: "house")
            Text("index")
                .sectionIndexLabel("A")
                .hoverEffectGroup()
                .navigationTransition(.slide)
        }
    }
}
