struct PropertyWrapperView: View {
    @State private var count = 3

    var body: some View {
        VStack {
            Text("count: \(count)")
            TextField("constant", text: Binding.constant("fixed"))
        }
    }
}
