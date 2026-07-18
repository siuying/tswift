import SwiftUI

struct FormFields: View {
    @Binding var name: String
    @Binding var enabled: Bool

    var body: some View {
        VStack {
            TextField("Name", text: $name)
            Toggle("Enabled", isOn: $enabled)
        }
    }
}

struct BindingChildForm: View {
    @State private var name = "Ada"
    @State private var enabled = false

    var body: some View {
        VStack {
            FormFields(
                name: Binding(get: { name }, set: { name = $0 }),
                enabled: $enabled
            )
            Text(name)
            Text(enabled ? "on" : "off")
        }
    }
}
