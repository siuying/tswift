// Charts Phase 2d — Chart.body plus a ChartProxy gesture builder.
import SwiftUI
import Charts

struct V: View {
    @State private var taps: Int = 1

    var body: some View {
        Chart {
            PointMark(
                x: .value("Day", "Mon"),
                y: .value("Count", taps)
            )
        }
        .chartGesture { _ in
            TapGesture(count: 2).onEnded { taps += 1 }
        }
        .body
    }
}
