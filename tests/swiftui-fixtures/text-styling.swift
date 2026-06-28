// C1 — text & universal styling modifiers (issue #188).
struct V: View {
    var body: some View {
        VStack {
            Text("Headline")
                .font(.title)
                .bold()
                .underline()
            Text("subtitle")
                .italic()
                .strikethrough()
                .foregroundStyle(.secondary)
                .textCase(.uppercase)
            Text("A longer paragraph that should clamp to two lines when it overflows the available width of its container.")
                .lineLimit(2)
                .multilineTextAlignment(.center)
                .opacity(0.8)
                .tint(.indigo)
        }
    }
}
