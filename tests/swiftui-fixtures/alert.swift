struct AlertView: View {
    @State private var show = false
    @State private var confirmed = 0
    var body: some View {
        VStack {
            Button("Warn") { show = true }
            Text("confirmed \(confirmed)")
        }
        .alert("Delete?", isPresented: $show, actions: {
            Button("OK") { confirmed += 1 }
        }, message: {
            Text("This cannot be undone")
        })
    }
}
