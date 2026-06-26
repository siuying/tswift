// expected-no-diagnostics
// Tier 11 / Web demo — Error handling: throws, do-catch, try?, Result, defer.

enum ValidationError: Error {
    case empty
    case tooShort(Int)
    case containsSpace
}

func validate(_ password: String) throws -> String {
    guard !password.isEmpty else { throw ValidationError.empty }
    guard password.count >= 6 else { throw ValidationError.tooShort(password.count) }
    guard !password.contains(" ") else { throw ValidationError.containsSpace }
    return "✓ valid"
}

let inputs = ["", "hi", "has space", "secure123"]
for pwd in inputs {
    do {
        let msg = try validate(pwd)
        print("'\(pwd)' → \(msg)")
    } catch ValidationError.empty {
        print("'\(pwd)' → error: empty")
    } catch ValidationError.tooShort(let n) {
        print("'\(pwd)' → error: too short (\(n) chars)")
    } catch ValidationError.containsSpace {
        print("'\(pwd)' → error: contains space")
    } catch {
        print("unexpected: \(error)")
    }
}

// try? and Result
let safe = try? validate("abc123")
print("try? 'abc123' → \(safe ?? "nil")")

let result: Result<String, ValidationError> = .success("ok")
if case .success(let v) = result { print("Result: \(v)") }

// defer: guaranteed cleanup
func withCleanup() -> String {
    defer { print("(cleanup ran)") }
    return "done"
}
print(withCleanup())
