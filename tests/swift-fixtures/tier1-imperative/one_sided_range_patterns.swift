// expected-no-diagnostics
// Tier 1c — one-sided range patterns in switch: postfix `n...`
// (PartialRangeFrom), prefix `..<n` (PartialRangeUpTo) and `...n`
// (PartialRangeThrough), alongside the two-sided form.

func grade(_ score: Int) -> String {
    switch score {
    case 90...: return "A"
    case 80..<90: return "B"
    case ..<60: return "F"
    default: return "C"
    }
}

func magnitude(_ n: Int) -> String {
    switch n {
    case ...0: return "non-positive"
    case ...100: return "small"
    default: return "big"
    }
}

print(grade(95))
print(magnitude(42))
