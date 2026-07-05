// gestures.swift — slice 7: TapGesture / LongPressGesture via .gesture(_:)
// Verifies that .gesture(TapGesture().onEnded { }) and
// .gesture(LongPressGesture(minimumDuration:).onEnded { }) lower to the same
// onTapGesture / onLongPressGesture marker + handler key route used by the
// direct modifiers.  Session events exercise the handler dispatch path.

struct GesturesView: View {
    @State private var taps = 0
    @State private var held = false

    var body: some View {
        VStack {
            Text("taps=\(taps) held=\(held)")
            Text("Tap me via gesture")
                .gesture(TapGesture().onEnded { _ in taps += 1 })
            Text("Multi-tap (2) via gesture")
                .gesture(TapGesture(count: 2).onEnded { _ in taps += 10 })
            Text("Hold me via gesture")
                .gesture(LongPressGesture(minimumDuration: 0.5).onEnded { _ in held = true })
        }
    }
}
