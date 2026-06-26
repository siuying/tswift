// expected-no-diagnostics
// Tier 2b — computed (get/set), read-only computed, observers, static, lazy.

struct Temperature {
    var celsius: Double

    var fahrenheit: Double {
        get { celsius * 9 / 5 + 32 }
        set { celsius = (newValue - 32) * 5 / 9 }
    }

    var kelvin: Double { celsius + 273.15 }
}

struct Tracker {
    var total = 0
    var count = 0 {
        willSet { print("count will become \(newValue)") }
        didSet { total += count - oldValue }
    }
    static let limit = 100
    lazy var firstTen: Int = (1 ... 10).reduce(0, +)
}

var temp = Temperature(celsius: 100)
temp.fahrenheit = 212

var tracker = Tracker()
tracker.count = 5
let lazilyComputed = tracker.firstTen

let _ = (temp.celsius, temp.kelvin, tracker.total, lazilyComputed, Tracker.limit)