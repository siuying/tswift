struct CounterView: View {
    @State private var count = 0
    var body: some View {
        VStack {
            Text("\(count)")
                .font(.largeTitle)
                .fontWeight(.bold)
                .foregroundColor(.white)
            Button("Increment") { count += 1 }
                .padding()
                .background(Color.blue)
                .cornerRadius(8)
        }
    }
}
