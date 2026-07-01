prefix operator √
postfix operator °
prefix func √ (value: Double) -> Double {
  var guess = value / 2
  for _ in 0..<20 {
    guess = (guess + value / guess) / 2
  }
  return guess
}
postfix func ° (degrees: Double) -> Double {
  degrees * 3.14159 / 180
}
print(√16.0)
print(180.0°)
let d = 90.0
print(d°)
