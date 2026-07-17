import SwiftUI

// C5 — graphic/visual-effect modifiers. Verifies the Core Animation-style
// filters that record a scalar, Bool, token, Color, or Angle the host applies:
// blur/brightness/contrast/saturation/grayscale, colorInvert/colorMultiply,
// hueRotation/rotationEffect (Angle), scaleEffect, and the layout/visibility
// toggles (hidden, allowsHitTesting, lineSpacing, minimumScaleFactor,
// allowsTightening, scrollDisabled).
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("blur").blur(radius: 2.0)
            Text("brightness").brightness(0.1)
            Text("contrast").contrast(1.2)
            Text("saturation").saturation(0.5)
            Text("grayscale").grayscale(0.3)
            Text("invert").colorInvert()
            Text("multiply").colorMultiply(.red)
            Text("hue").hueRotation(.degrees(45))
            Text("rotate").rotationEffect(.degrees(90))
            Text("scale").scaleEffect(1.5)
            Text("hidden").hidden()
            Text("hitless").allowsHitTesting(false)
            Text("spacing").lineSpacing(4.0)
            Text("minScale").minimumScaleFactor(0.6)
            Text("tighten").allowsTightening(true)
            ScrollView { Text("locked") }.scrollDisabled(true)
        }
    }
}
