// Charts — line + point + area marks with axis labels.
struct V: View {
    var body: some View {
        Chart {
            AreaMark(
                x: .value("Day", "Mon"),
                y: .value("Temp", 12)
            )
            .foregroundStyle(.blue)
            .opacity(0.25)
            AreaMark(
                x: .value("Day", "Tue"),
                y: .value("Temp", 18)
            )
            .foregroundStyle(.blue)
            .opacity(0.25)
            AreaMark(
                x: .value("Day", "Wed"),
                y: .value("Temp", 15)
            )
            .foregroundStyle(.blue)
            .opacity(0.25)
            AreaMark(
                x: .value("Day", "Thu"),
                y: .value("Temp", 22)
            )
            .foregroundStyle(.blue)
            .opacity(0.25)
            AreaMark(
                x: .value("Day", "Fri"),
                y: .value("Temp", 19)
            )
            .foregroundStyle(.blue)
            .opacity(0.25)

            LineMark(
                x: .value("Day", "Mon"),
                y: .value("Temp", 12)
            )
            .foregroundStyle(.blue)
            .lineStyle(StrokeStyle(lineWidth: 2))
            LineMark(
                x: .value("Day", "Tue"),
                y: .value("Temp", 18)
            )
            .foregroundStyle(.blue)
            LineMark(
                x: .value("Day", "Wed"),
                y: .value("Temp", 15)
            )
            .foregroundStyle(.blue)
            LineMark(
                x: .value("Day", "Thu"),
                y: .value("Temp", 22)
            )
            .foregroundStyle(.blue)
            LineMark(
                x: .value("Day", "Fri"),
                y: .value("Temp", 19)
            )
            .foregroundStyle(.blue)

            PointMark(
                x: .value("Day", "Mon"),
                y: .value("Temp", 12)
            )
            .foregroundStyle(.blue)
            .symbolSize(64)
            PointMark(
                x: .value("Day", "Tue"),
                y: .value("Temp", 18)
            )
            .foregroundStyle(.blue)
            .symbolSize(64)
            PointMark(
                x: .value("Day", "Wed"),
                y: .value("Temp", 15)
            )
            .foregroundStyle(.blue)
            .symbolSize(64)
            PointMark(
                x: .value("Day", "Thu"),
                y: .value("Temp", 22)
            )
            .foregroundStyle(.blue)
            .symbolSize(64)
            PointMark(
                x: .value("Day", "Fri"),
                y: .value("Temp", 19)
            )
            .foregroundStyle(.blue)
            .symbolSize(64)
        }
        .chartXAxisLabel("Day")
        .chartYAxisLabel("Temp °C")
        .frame(width: 320, height: 220)
    }
}
