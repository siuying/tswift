import SwiftUI

// Closure-driven layout / effect / scroll / event modifiers (recorded-only).
// Each records a bare marker; the trailing closure is stashed under the same
// event key and never serialized (a headless runtime synthesizes no
// Transaction/GeometryProxy/ScrollGeometry/preference value to feed it). This
// verifies the markers reach the UIIR in order without carrying closure noise.
struct WidthKey: PreferenceKey {
    static var defaultValue: Double = 0
    static func reduce(value: inout Double, nextValue: () -> Double) {}
}

struct V: View {
    var body: some View {
        ScrollView {
            Text("hi")
                .transaction { t in t.disablesAnimations = true }
                .visualEffect { content, proxy in content }
                .transformEnvironment(\.self) { _ in }
                .scrollTransition { content, phase in content }
                .onGeometryChange(for: Double.self, of: { _ in 0.0 }, action: { _ in })
                .onPreferenceChange(WidthKey.self) { _ in }
                .onModifierKeysChanged { _, _ in }
        }
        .onScrollPhaseChange { _, _ in }
        .onScrollVisibilityChange(threshold: 0.5) { _ in }
        .onScrollGeometryChange(for: Double.self, of: { _ in 0.0 }, action: { _, _ in })
    }
}
