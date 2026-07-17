import Charts
import SwiftUI

struct Sale: Identifiable {
    let id: Int
    let month: String
    let revenue: Double
}

struct ChartsDemo: View {
    let sales = [
        Sale(id: 1, month: "Jan", revenue: 120),
        Sale(id: 2, month: "Feb", revenue: 90),
        Sale(id: 3, month: "Mar", revenue: 150),
    ]

    var body: some View {
        VStack {
            // Data-driven bars: one BarMark per element.
            Chart(sales) { sale in
                BarMark(
                    x: .value("Month", sale.month),
                    y: .value("Revenue", sale.revenue),
                    width: .fixed(20),
                    stacking: .center
                )
            }

            // Static marks of several kinds, exercising every dimension and
            // stacking token.
            Chart {
                LineMark(x: .value("Month", "Jan"), y: .value("Revenue", 120))
                PointMark(x: .value("Month", "Feb"), y: .value("Revenue", 90))
                AreaMark(
                    x: .value("Month", "Mar"),
                    y: .value("Revenue", 150),
                    stacking: .standard
                )
                AreaMark(
                    x: .value("Month", "Apr"),
                    y: .value("Revenue", 80),
                    stacking: .normalized
                )
                BarMark(
                    x: .value("Month", "May"),
                    y: .value("Revenue", 70),
                    width: .automatic,
                    stacking: .unstacked
                )
                RuleMark(y: .value("Target", 100))
                RectangleMark(
                    x: .value("Month", "Jan"),
                    y: .value("Revenue", 60),
                    width: .inset(4)
                )
                SectorMark(angle: .value("Share", 40), innerRadius: .ratio(0.5))
            }
        }
    }
}
