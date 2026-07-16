// Profile tab — view composition: custom sub-views with parameters threaded
// down, composed inside a container, each collapsing to its own `body`.
import SwiftUI

struct InfoRow: View {
    let label: String
    let value: String
    var body: some View {
        HStack {
            Text(label).foregroundColor(.secondary)
            Spacer()
            Text(value).fontWeight(.semibold)
        }
    }
}

struct ProfileView: View {
    var body: some View {
        VStack {
            Text("Profile").font(.largeTitle).fontWeight(.bold)
            InfoRow(label: "Name", value: "Ada")
            InfoRow(label: "Role", value: "Engineer")
        }
        .padding()
    }
}
