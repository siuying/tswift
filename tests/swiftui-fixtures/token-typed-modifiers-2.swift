// More token-valued modifiers via the typed seam. Each leading-dot arg
// resolves against a dedicated parameter type; several reuse shared names
// (.small/.medium/.large/.none/.default/.words/.standard) that resolve
// per-modifier rather than by global uniqueness.
struct V: View {
    var body: some View {
        VStack {
            Image(systemName: "star")
                .imageScale(.large)
                .allowedDynamicRange(.high)
            TextField("email", text: .constant(""))
                .keyboardType(.emailAddress)
                .autocapitalization(.words)
                .textInputAutocapitalization(.sentences)
            Text("scale")
                .textScale(.secondary)
                .foregroundColor(.secondary)
            Button("menu") {}
                .menuActionDismissBehavior(.disabled)
                .buttonRepeatBehavior(.enabled)
            Text("writing")
                .writingToolsBehavior(.complete)
            Section("prominent") {
                Text("row")
            }
            .headerProminence(.standard)
            Text("labels")
                .labelsVisibility(.hidden)
        }
    }
}
