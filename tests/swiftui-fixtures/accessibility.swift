// Accessibility metadata modifiers. Verifies the semantic-data modifiers that
// record onto the view node: trait sets (AccessibilityTraits), heading rank
// (AccessibilityHeadingLevel), element grouping (AccessibilityChildBehavior),
// input labels ([String]), sort priority (Double), and the Bool toggles.
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("button")
                .accessibilityLabel("Submit")
                .accessibilityAddTraits(.isButton)
                .accessibilityRemoveTraits(.isImage)
            Text("heading")
                .accessibilityHeading(.h1)
                .accessibilitySortPriority(10)
            Text("field")
                .accessibilityInputLabels(["name", "full name"])
                .accessibilityRespondsToUserInteraction(true)
            VStack(spacing: 2) {
                Text("row")
            }
            .accessibilityElement(children: .combine)
            .accessibilityIgnoresInvertColors(true)
            .accessibilityDirectTouch(true)
            .accessibilityShowsLargeContentViewer()
            Text("points")
                .accessibilityActivationPoint(.center)
                .accessibilityTextContentType(.sourceCode)
                .accessibilityCustomContent("Points", "42")
                .accessibilityChartDescriptor("trend")
            Text("grouped")
                .accessibilityChildren {
                    Text("child a")
                    Text("child b")
                }
                .accessibilityRepresentation {
                    Text("standard control")
                }
                .accessibilityActions {
                    Text("action label")
                }
        }
    }
}
