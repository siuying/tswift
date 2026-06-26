// `@unknown default` — a catch-all clause for cases added in the future,
// treated like `default` at runtime.

enum Status { case ok, warn, fail }

func classify(_ s: Status) -> String {
    switch s {
    case .ok: return "ok"
    @unknown default: return "other"
    }
}

print(classify(.ok))
print(classify(.warn))
print(classify(.fail))

func describe(_ n: Int) -> String {
    switch n {
    case 0: return "zero"
    case 1: return "one"
    @unknown default: return "many"
    }
}

print(describe(0))
print(describe(7))
