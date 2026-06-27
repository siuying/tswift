// Custom `Error` types: enum errors with associated values and struct errors,
// caught with value, associated-value, and `as`-cast patterns.
enum NetworkError: Error {
    case timeout
    case badStatus(Int)
}

func fetch(_ ok: Bool) throws -> String {
    if !ok {
        throw NetworkError.badStatus(404)
    }
    return "data"
}

do {
    let r = try fetch(true)
    print(r)
} catch {
    print("unexpected")
}

do {
    _ = try fetch(false)
} catch NetworkError.badStatus(let code) {
    print("status \(code)")
} catch {
    print("other")
}

struct ValidationError: Error {
    let field: String
}

func validate(_ name: String) throws {
    if name.isEmpty {
        throw ValidationError(field: "name")
    }
}

do {
    try validate("")
} catch let e as ValidationError {
    print("invalid: \(e.field)")
} catch {
    print("?")
}

// `as`-cast pattern in a switch over a heterogeneous error.
func classify(_ error: Error) -> String {
    switch error {
    case let n as NetworkError:
        if case .timeout = n { return "net-timeout" }
        return "net-other"
    case is ValidationError:
        return "validation"
    default:
        return "unknown"
    }
}
print(classify(NetworkError.timeout))
print(classify(ValidationError(field: "x")))
