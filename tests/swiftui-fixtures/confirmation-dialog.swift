import SwiftUI

struct ConfirmDialogView: View {
    @State private var show = false
    @State private var choice = "none"
    var body: some View {
        VStack {
            Button("Options") { show = true }
            Text(choice)
        }
        .confirmationDialog("Pick one", isPresented: $show, actions: {
            Button("First") { choice = "first" }
            Button("Second") { choice = "second" }
        })
    }
}
