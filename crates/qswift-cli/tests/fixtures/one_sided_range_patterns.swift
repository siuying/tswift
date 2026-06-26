// One-sided range patterns in switch: `n...`, `..<n`, `...n`.
func grade(_ score: Int) -> String {
    switch score {
    case 90...: return "A"
    case 80..<90: return "B"
    case ..<60: return "F"
    default: return "C"
    }
}

for score in [95, 85, 72, 50] {
    print(grade(score))
}

let n = 42
switch n {
case ...0: print("non-positive")
case ...100: print("small")
default: print("big")
}
