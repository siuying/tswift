// Charts — bar marks with series coloring + axis label.
struct V: View {
    var body: some View {
        Chart {
            BarMark(
                x: .value("Fruit", "Apple"),
                y: .value("Sales", 40)
            )
            .foregroundStyle(by: .value("Store", "North"))
            BarMark(
                x: .value("Fruit", "Apple"),
                y: .value("Sales", 25)
            )
            .foregroundStyle(by: .value("Store", "South"))
            BarMark(
                x: .value("Fruit", "Banana"),
                y: .value("Sales", 55)
            )
            .foregroundStyle(by: .value("Store", "North"))
            BarMark(
                x: .value("Fruit", "Banana"),
                y: .value("Sales", 30)
            )
            .foregroundStyle(by: .value("Store", "South"))
            BarMark(
                x: .value("Fruit", "Cherry"),
                y: .value("Sales", 20)
            )
            .foregroundStyle(by: .value("Store", "North"))
            BarMark(
                x: .value("Fruit", "Cherry"),
                y: .value("Sales", 35)
            )
            .foregroundStyle(by: .value("Store", "South"))
        }
        .chartXAxisLabel("Fruit")
        .chartYAxisLabel("Sales")
        .frame(width: 320, height: 220)
    }
}
