// Picker tab — a segmented choice bound to @State; each option is tagged, and
// choosing one emits `set` with its tag, echoed live in a Text.
import SwiftUI

struct PickerView: View {
    @State private var flavor = "vanilla"

    var body: some View {
        VStack {
            Picker("Flavor", selection: $flavor) {
                Text("Vanilla").tag("vanilla")
                Text("Chocolate").tag("chocolate")
                Text("Strawberry").tag("strawberry")
            }
            Text("You picked \(flavor)")
        }
    }
}
