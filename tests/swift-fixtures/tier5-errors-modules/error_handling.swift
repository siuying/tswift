// expected-no-diagnostics
// Tier 5 — Error types, throws/throw, do-catch (+ pattern catch), try/try?/try!,
// rethrows, defer, Result.

enum NetworkError: Error {
    case timeout
    case status(code: Int)
}

func fetch(_ url: String) throws -> String {
    guard !url.isEmpty else { throw NetworkError.timeout }
    return "body of \(url)"
}

func process() -> String {
    do {
        return try fetch("https://example.com")
    } catch NetworkError.status(let code) {
        return "status \(code)"
    } catch {
        return "error: \(error)"
    }
}

func withCleanup() -> Int {
    var steps = 0
    defer { steps += 1 }
    steps += 10
    return steps
}

func retry(_ work: () throws -> Int) rethrows -> Int {
    try work()
}

let maybeBody = try? fetch("")
let forcedBody = try! fetch("ok")
let outcome: Result<Int, NetworkError> = .success(200)

let _ = (process(), withCleanup(), try? retry({ 1 }), maybeBody, forcedBody, outcome)
