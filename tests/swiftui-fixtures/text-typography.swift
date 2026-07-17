import SwiftUI

// C1 — Text typography modifiers (Text -> Text). Verifies the weight/emphasis
// stylers (bold, italic, underline, strikethrough) alongside the numeric
// letter-spacing/baseline adjustments (kerning, tracking, baselineOffset) and
// the monospacing toggles (monospaced, monospacedDigit), plus font/weight and
// foreground styling.
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("bold").bold()
            Text("italic").italic()
            Text("underline").underline()
            Text("strike").strikethrough()
            Text("kerning").kerning(1.5)
            Text("tracking").tracking(2.0)
            Text("baseline").baselineOffset(3.0)
            Text("mono").monospaced()
            Text("monoDigit").monospacedDigit()
            Text("styled").font(.headline).fontWeight(.semibold).foregroundColor(.blue)
        }
    }
}
