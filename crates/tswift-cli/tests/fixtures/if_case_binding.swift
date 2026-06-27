// `if case` / `guard case` pattern-match conditions with payload binding.
enum Token { case number(Int); case word(String); case eof }

func describe(_ t: Token) -> String {
    if case .number(let n) = t { return "num \(n)" }
    if case .word(let w) = t { return "word \(w)" }
    return "eof"
}

for t in [Token.number(42), .word("hi"), .eof] {
    print(describe(t))
}

let result: Result<String, Error> = .success("ok")
if case .success(let v) = result { print(v) }

func check(_ t: Token) -> Bool {
    guard case .eof = t else { return false }
    return true
}
print(check(.eof), check(.number(1)))
