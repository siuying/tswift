// UI3 — Color system palette. Verifies the named system colors stay semantic
// tokens across the UIIR boundary, plus `accentColor`, the primary/secondary
// content colors, and the `.opacity(_:)` alpha adjustment on both named and
// explicit RGB colors.
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("red").foregroundColor(.red)
            Text("orange").foregroundColor(.orange)
            Text("yellow").foregroundColor(.yellow)
            Text("green").foregroundColor(.green)
            Text("mint").foregroundColor(.mint)
            Text("teal").foregroundColor(.teal)
            Text("cyan").foregroundColor(.cyan)
            Text("blue").foregroundColor(.blue)
            Text("indigo").foregroundColor(.indigo)
            Text("purple").foregroundColor(.purple)
            Text("pink").foregroundColor(.pink)
            Text("brown").foregroundColor(.brown)
            Text("gray").foregroundColor(.gray)
            Text("black").foregroundColor(Color.black)
            Text("white").foregroundColor(.white)
            Text("clear").foregroundColor(.clear)
            Text("primary").foregroundColor(.primary)
            Text("secondary").foregroundColor(.secondary)
            Text("accent").foregroundColor(.accentColor)
            Text("faded").foregroundColor(.blue.opacity(0.5))
            Text("fadedRGB").foregroundColor(Color(red: 0.2, green: 0.4, blue: 0.6).opacity(0.25))
        }
    }
}
