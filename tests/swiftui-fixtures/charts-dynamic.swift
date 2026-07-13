// Charts — dynamic data: button bumps @State, must emit setArgs on BarMark
// so the web host re-paints SVG (Slice 5 dynamic-patch coverage).
struct V: View {
    @State private var sales = 40

    var body: some View {
        VStack {
            Chart {
                BarMark(
                    x: .value("Fruit", "Apple"),
                    y: .value("Sales", sales)
                )
                .foregroundStyle(.blue)
                BarMark(
                    x: .value("Fruit", "Banana"),
                    y: .value("Sales", 25)
                )
                .foregroundStyle(.green)
            }
            .chartXAxisLabel("Fruit")
            .chartYAxisLabel("Sales")
            .frame(width: 320, height: 220)

            Button("Bump") { sales += 15 }
        }
    }
}
