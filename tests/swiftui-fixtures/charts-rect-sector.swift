// Charts — RectangleMark (x/y/width/height) + SectorMark (lone full arc).
import SwiftUI
import Charts

struct V: View {
    var body: some View {
        VStack(spacing: 16) {
            Chart {
                RectangleMark(
                    x: .value("X", 2),
                    y: .value("Y", 5),
                    width: 48,
                    height: 36
                )
                .foregroundStyle(.orange)
                RectangleMark(
                    x: .value("X", 5),
                    y: .value("Y", 8),
                    width: 40,
                    height: 28
                )
                .foregroundStyle(.purple)
            }
            .chartXAxisLabel("X")
            .chartYAxisLabel("Y")
            .frame(width: 320, height: 180)

            Chart {
                SectorMark(
                    angle: .value("Share", 100),
                    innerRadius: 24
                )
                .foregroundStyle(.blue)
            }
            .frame(width: 320, height: 180)
        }
    }
}
