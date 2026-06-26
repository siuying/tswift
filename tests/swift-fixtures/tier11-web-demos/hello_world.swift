// expected-no-diagnostics
// Tier 11 / Web demo — Hello World: string interpolation, let/var, ternary.

let language = "Swift"
let version = 6
print("Hello from \(language) \(version)! 👋")

let π = 3.14159
let radius = 5.0
let area = π * radius * radius
print("Circle area (r=\(radius)): \(area)")

let score = 87
let grade = score >= 90 ? "A" : score >= 80 ? "B" : score >= 70 ? "C" : "F"
print("Score \(score) → Grade \(grade)")
