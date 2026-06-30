import Foundation

// Construction and description.
let distance = Measurement(value: 5, unit: UnitLength.kilometers)
print(distance.description)
print(distance.value)
print(distance.unit.symbol)

// Linear conversion km -> miles.
let miles = distance.converted(to: .miles)
print(miles.value)

// Implicit-member units.
let weight = Measurement(value: 2, unit: UnitMass.kilograms)
print(weight.converted(to: .pounds).value)

// Affine conversion (temperature). Round to dodge float noise.
let boiling = Measurement(value: 100, unit: UnitTemperature.celsius)
print(boiling.converted(to: .fahrenheit).value.rounded())
let freezing = Measurement(value: 32, unit: UnitTemperature.fahrenheit)
print(freezing.converted(to: .celsius).value.rounded())

// mutating convert.
var trip = Measurement(value: 1, unit: UnitDuration.hours)
trip.convert(to: .minutes)
print(trip.value)
print(trip.unit.symbol)

// Arithmetic (same dimension) and scalar scaling.
let a = Measurement(value: 1, unit: UnitLength.kilometers)
let b = Measurement(value: 500, unit: UnitLength.meters)
print((a + b).description)
print((a - b).value)
print((a * 2).value)
print((a / 4).value)

// Comparison in base units.
print(a > b)
print(a == Measurement(value: 1000, unit: UnitLength.meters))
print(a < b)

// Affine arithmetic: 0°C + 32°F converts the rhs into °C (0°C), not base sum.
let zeroC = Measurement(value: 0, unit: UnitTemperature.celsius)
let freezF = Measurement(value: 32, unit: UnitTemperature.fahrenheit)
print((zeroC + freezF).value)
print((zeroC + freezF).unit.symbol)

// Hashable: equal magnitudes hash equally across display units.
let oneKm = Measurement(value: 1, unit: UnitLength.kilometers)
let thousandM = Measurement(value: 1000, unit: UnitLength.meters)
print(oneKm == thousandM)
print(oneKm.hashValue == thousandM.hashValue)
print(oneKm.debugDescription)
