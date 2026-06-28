// Greeting tab — Toggle bound to a Bool @State plus a ternary in body. Flipping
// the toggle writes through the binding and switches the conditional content.
struct GreetingView: View {
    @State private var formal = true
    var body: some View {
        VStack {
            Toggle("Use formal greeting", isOn: $formal)
            Text(formal ? "Good evening." : "Hey there!")
                .font(.title)
        }
        .padding()
    }
}
