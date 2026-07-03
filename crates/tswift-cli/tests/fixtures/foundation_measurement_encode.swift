import Foundation

// Measurement Codable / JSON encoding.
// Ground-truth: `JSONEncoder().encode(Measurement(value:5,unit:UnitLength.meters))`
// with `.sortedKeys` produces:
// {"unit":{"converter":{"coefficient":1,"constant":0},"symbol":"m"},"value":5}

struct MeasureWrapper: Codable {
    let distance: Measurement<UnitLength>
    let mass: Measurement<UnitMass>
}

var enc = JSONEncoder()
enc.outputFormatting = .sortedKeys

// 1. Direct encode UnitLength.meters (coefficient=1, constant=0).
let m1 = Measurement(value: 5, unit: UnitLength.meters)
let d1 = try enc.encode(m1)
print(String(data: d1, encoding: .utf8)!)

// 2. UnitTemperature.celsius (coefficient=1, constant=273.15).
let m2 = Measurement(value: 100.0, unit: UnitTemperature.celsius)
let d2 = try enc.encode(m2)
print(String(data: d2, encoding: .utf8)!)

// 3. UnitLength.kilometers (coefficient=1000, constant=0).
let m3 = Measurement(value: 1000, unit: UnitLength.kilometers)
let d3 = try enc.encode(m3)
print(String(data: d3, encoding: .utf8)!)

// 4. Struct containing two Measurement fields (sorted keys).
let wrapper = MeasureWrapper(
    distance: Measurement(value: 2.5, unit: UnitLength.miles),
    mass: Measurement(value: 70, unit: UnitMass.kilograms)
)
let d4 = try enc.encode(wrapper)
print(String(data: d4, encoding: .utf8)!)
