import SwiftUI

struct PresentationStylesView: View {
    @State private var showCover = false
    @State private var showPopover = false
    var body: some View {
        VStack {
            Button("Cover") { showCover = true }
            Button("Popover") { showPopover = true }
        }
        .fullScreenCover(isPresented: $showCover) {
            Text("Full screen")
        }
        .popover(isPresented: $showPopover) {
            Text("Popover body")
        }
    }
}
