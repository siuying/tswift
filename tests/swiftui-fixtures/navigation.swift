import SwiftUI

struct RootView: View {
    @State private var count = 0
    var body: some View {
        NavigationStack {
            VStack {
                Text("Home")
                Button("Increment") { count += 1 }
                NavigationLink("Show Detail") {
                    VStack {
                        Text("Count: \(count)")
                    }
                    .navigationTitle("Detail")
                }
            }
            .navigationTitle("Home")
        }
    }
}
