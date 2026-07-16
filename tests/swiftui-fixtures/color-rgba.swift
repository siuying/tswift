// Explicit SwiftUI RGB colors are semantic RGBA values, not host-specific
// pixels. The UIIR golden covers both default and non-default opacity.
struct ColorRgbaView: View {
    var body: some View {
        VStack {
            Text("Ocean")
                .foregroundColor(Color(red: 0.1, green: 0.4, blue: 0.8))
            Circle()
                .fill(Color(red: 0.9, green: 0.2, blue: 0.1, opacity: 0.35))
        }
    }
}
