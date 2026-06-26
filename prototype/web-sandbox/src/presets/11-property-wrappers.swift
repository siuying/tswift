// @propertyWrapper: Clamped + UserDefault-style logging
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

struct AudioSettings {
    @Clamped(0...100) var volume: Int = 50
    @Clamped(0...10)  var bass:   Int = 5
}

var audio = AudioSettings()
print("default  volume=\(audio.volume) atMax=\(audio.$volume)")
audio.volume = 150
print("set 150  volume=\(audio.volume) atMax=\(audio.$volume)")
audio.volume = -5
print("set -5   volume=\(audio.volume) atMax=\(audio.$volume)")
audio.volume = 100
print("set 100  volume=\(audio.volume) atMax=\(audio.$volume)")

audio.bass = 11
print("bass clamped to \(audio.bass)")
