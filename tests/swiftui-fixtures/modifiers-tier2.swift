struct ModifiersTier2View: View {
    var body: some View {
        VStack {
            Image("photo")
                .resizable()
                .scaledToFit()
            Image("banner")
                .resizable()
                .scaledToFill()
            Text("Fixed both")
                .fixedSize()
            Text("Fixed horiz")
                .fixedSize(horizontal: true, vertical: false)
            HStack {
                Text("low")
                    .layoutPriority(0)
                Text("high")
                    .layoutPriority(1)
            }
            Rectangle()
                .aspectRatio(1.777, contentMode: .fit)
            Text("front")
                .zIndex(2.0)
        }
        .navigationTitle("Tier 2")
    }
}
