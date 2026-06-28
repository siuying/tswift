// Controls tab — a Slider bound to a Double and a Stepper bound to an Int, each
// echoed in a Text. Dragging/stepping emits `set` events through the bindings.
struct ControlsView: View {
    @State private var brightness = 0.5
    @State private var volume = 30

    var body: some View {
        VStack {
            Slider(value: $brightness, in: 0...1, step: 0.1)
            Text("Brightness \(brightness)")
            Stepper("Volume", value: $volume, in: 0...100, step: 5)
            Text("Volume is \(volume)")
        }
    }
}
