// expected-no-diagnostics
// Tier 5 — @propertyWrapper with wrappedValue and a projected value.

@propertyWrapper
struct Clamped {
    private var value: Int
    private let range: ClosedRange<Int>

    init(wrappedValue: Int, _ range: ClosedRange<Int>) {
        self.range = range
        self.value = min(max(wrappedValue, range.lowerBound), range.upperBound)
    }

    var wrappedValue: Int {
        get { value }
        set { value = min(max(newValue, range.lowerBound), range.upperBound) }
    }

    var projectedValue: Bool { value == range.upperBound }
}

struct Settings {
    @Clamped(0 ... 100) var volume: Int = 50
}

var settings = Settings()
settings.volume = 200
let clampedVolume = settings.volume
let atMaximum = settings.$volume

let _ = (clampedVolume, atMaximum)
