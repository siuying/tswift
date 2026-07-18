//! Coverage-key dump and registered_keys assertions.

use crate::registered_keys;

#[test]
fn dump_registered_keys() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = root.join("frameworks/charts/registered_keys.txt");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = registered_keys().join("\n") + "\n";
    std::fs::write(&path, body).expect("write registered_keys.txt");
}

#[test]
fn registered_keys_cover_core_marks_and_modifiers() {
    let keys = registered_keys();
    for expected in [
        "AreaMark.init",
        "AxisGridLine.init",
        "AxisMarks.init",
        "AxisTick.init",
        "AxisValueLabel.init",
        "BarMark.init",
        "Chart.init",
        "Chart.body",
        "ChartContent.accessibilityHidden",
        "ChartContent.accessibilityIdentifier",
        "ChartContent.accessibilityLabel",
        "ChartContent.accessibilityValue",
        "ChartContent.alignsMarkStylesWithPlotArea",
        "ChartContent.annotation",
        "ChartContent.blur",
        "ChartContent.clipShape",
        "ChartContent.compositingLayer",
        "ChartContent.cornerRadius",
        "ChartContent.foregroundStyle",
        "ChartContent.interpolationMethod",
        "ChartContent.lineStyle",
        "ChartContent.mask",
        "ChartContent.offset",
        "ChartContent.opacity",
        "ChartContent.position",
        "ChartContent.shadow",
        "ChartContent.symbol",
        "ChartContent.symbolSize",
        "ChartContent.zIndex",
        "LineMark.init",
        "PlottableValue.value",
        "PointMark.init",
        "RectangleMark.init",
        "RuleMark.init",
        "SectorMark.init",
        "View.chartAngleSelection",
        "View.chartBackground",
        "View.chartForegroundStyleScale",
        "View.chartLegend",
        "View.chartLineStyleScale",
        "View.chartOverlay",
        "View.chartPlotStyle",
        "View.chartScrollPosition",
        "View.chartScrollTargetBehavior",
        "View.chartScrollableAxes",
        "View.chartSymbolScale",
        "View.chartSymbolSizeScale",
        "View.chartXAxis",
        "View.chartXAxisLabel",
        "View.chartXAxisStyle",
        "View.chartXScale",
        "View.chartXSelection",
        "View.chartXVisibleDomain",
        "View.chartYAxis",
        "View.chartYAxisLabel",
        "View.chartYAxisStyle",
        "View.chartYScale",
        "View.chartYSelection",
        "View.chartYVisibleDomain",
        "View.chartGesture",
    ] {
        assert!(
            keys.iter().any(|k| k == expected),
            "missing coverage key {expected}; keys={keys:?}"
        );
    }
}
