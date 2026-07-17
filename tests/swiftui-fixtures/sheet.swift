struct SheetView: View {
    @State private var showSheet = false
    @State private var dismissed = 0
    var body: some View {
        VStack {
            Button("Show") { showSheet = true }
            Text("dismissed \(dismissed)")
        }
        .sheet(isPresented: $showSheet, onDismiss: { dismissed += 1 }) {
            VStack {
                Text("Sheet body")
                Button("Close") { showSheet = false }
            }
        }
    }
}
