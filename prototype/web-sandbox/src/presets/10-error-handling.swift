// throws, do-catch, try?, try!, Result, defer, rethrows
enum ValidationError: Error {
    case empty
    case tooShort(Int)
    case invalidChar(Character)
}

func validate(_ password: String) throws -> String {
    guard !password.isEmpty else { throw ValidationError.empty }
    guard password.count >= 6  else { throw ValidationError.tooShort(password.count) }
    for ch in password {
        guard ch.isLetter || ch.isNumber else { throw ValidationError.invalidChar(ch) }
    }
    return "✓ valid"
}

let inputs = ["", "hi", "hello!", "secure123"]
for pwd in inputs {
    do {
        let msg = try validate(pwd)
        print("'\(pwd)' → \(msg)")
    } catch ValidationError.empty {
        print("'\(pwd)' → error: empty")
    } catch ValidationError.tooShort(let n) {
        print("'\(pwd)' → error: too short (\(n) chars)")
    } catch ValidationError.invalidChar(let c) {
        print("'\(pwd)' → error: bad char '\(c)'")
    } catch { print("unexpected: \(error)") }
}

// try? and Result
let safe = try? validate("abc123")
print("try? 'abc123' → \(safe ?? "nil")")

let result: Result<String, ValidationError> = .success("ok")
if case .success(let v) = result { print("Result: \(v)") }

// defer
func withCleanup() -> String {
    defer { print("(cleanup ran)") }
    return "done"
}
print(withCleanup())
