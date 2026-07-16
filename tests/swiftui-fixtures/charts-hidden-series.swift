// Charts — series split via foregroundStyle(by:) with axes + legend hidden.
import SwiftUI
import Charts

struct V: View {
    var body: some View {
        Chart {
            BarMark(
                x: .value("Q", "Q1"),
                y: .value("Rev", 50)
            )
            .foregroundStyle(by: .value("Region", "East"))
            BarMark(
                x: .value("Q", "Q1"),
                y: .value("Rev", 30)
            )
            .foregroundStyle(by: .value("Region", "West"))
            BarMark(
                x: .value("Q", "Q2"),
                y: .value("Rev", 70)
            )
            .foregroundStyle(by: .value("Region", "East"))
            BarMark(
                x: .value("Q", "Q2"),
                y: .value("Rev", 45)
            )
            .foregroundStyle(by: .value("Region", "West"))
            BarMark(
                x: .value("Q", "Q3"),
                y: .value("Rev", 40)
            )
            .foregroundStyle(by: .value("Region", "East"))
            BarMark(
                x: .value("Q", "Q3"),
                y: .value("Rev", 60)
            )
            .foregroundStyle(by: .value("Region", "West"))
        }
        .chartXAxis(.hidden)
        .chartYAxis(.hidden)
        .chartLegend(.hidden)
        .frame(width: 320, height: 220)
    }
}
