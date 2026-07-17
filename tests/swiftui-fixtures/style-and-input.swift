import SwiftUI

// Container/control style modifiers (each carries a leading-dot `_ControlStyle`
// token; the host disambiguates by modifier name) plus text-input modifiers
// (submitLabel/textInputAutocapitalization tokens, autocorrectionDisabled/
// disableAutocorrection/focusable Bool toggles). Uses only tokens with a unique
// leading-dot name — `.automatic` is intentionally avoided (it is shared by
// Visibility and needs contextual typing to resolve).
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Toggle("toggle", isOn: .constant(true)).toggleStyle(.button)
            Text("field")
                .textInputAutocapitalization(.words)
                .submitLabel(.done)
                .autocorrectionDisabled(true)
                .disableAutocorrection(false)
                .focusable(true)
        }
        .formStyle(.grouped)
        .menuStyle(.borderlessButton)
        .tabViewStyle(.page)
        .indexViewStyle(.page)
        .datePickerStyle(.compact)
        .controlGroupStyle(.navigationLink)
        .gaugeStyle(.accessoryCircular)
    }
}
