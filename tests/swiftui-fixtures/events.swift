import SwiftUI

struct EventsView: View {
    @State private var taps = 0
    @State private var doubled = 0
    @State private var name = ""
    @State private var status = "idle"
    var body: some View {
        VStack {
            Text("taps=\(taps) doubled=\(doubled) status=\(status)")
            Text("Tap me")
                .onTapGesture { taps += 1 }
            TextField("Name", text: $name)
                .onSubmit { status = "submitted:\(name)" }
        }
        .onAppear { status = "appeared" }
        .onChange(of: taps) { old, new in doubled = new * 2 }
        .onChange(of: doubled) { old, new in status = "d\(old)->\(new)" }
    }
}
