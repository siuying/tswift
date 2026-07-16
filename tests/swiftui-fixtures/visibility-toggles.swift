// Chrome-visibility & interaction-disabling toggles plus accessibility speech
// hints. Each carries a plain Bool / String / Double value (no leading-dot
// token); the host records the scalar straight onto the view node.
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("title")
                .navigationBarBackButtonHidden(true)
                .navigationBarHidden(false)
                .navigationSubtitle("subtitle")
                .statusBarHidden(true)
            Text("preview")
                .previewDisplayName("Light")
                .privacySensitive(true)
            Text("focus")
                .focusEffectDisabled(true)
                .hoverEffectDisabled(true)
                .findDisabled(true)
                .replaceDisabled(false)
                .allowsWindowActivationEvents(true)
            Image(systemName: "star")
                .symbolEffectsRemoved(true)
            ScrollView {
                Text("row")
            }
            .scrollTargetLayout(true)
            .scrollIndicatorsFlash(true)
            Text("speech")
                .speechAdjustedPitch(0.5)
                .speechAlwaysIncludesPunctuation(true)
                .speechAnnouncementsQueued(false)
                .speechSpellsOutCharacters(true)
        }
    }
}
