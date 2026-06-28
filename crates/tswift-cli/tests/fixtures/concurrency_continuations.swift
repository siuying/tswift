// Bridging callbacks into `async` with continuations: `withCheckedContinuation`
// resumed inline, `withUnsafeContinuation` resumed from a spawned `Task`, and
// `withCheckedThrowingContinuation` propagating a thrown error via
// `resume(throwing:)` / `resume(with:)`.
struct Boom: Error {}

func inlineValue() async -> Int {
    await withCheckedContinuation { continuation in
        continuation.resume(returning: 42)
    }
}

func deferredValue() async -> Int {
    await withUnsafeContinuation { continuation in
        Task { continuation.resume(returning: 7) }
    }
}

func throwingValue(_ fail: Bool) async throws -> Int {
    try await withCheckedThrowingContinuation { continuation in
        if fail {
            continuation.resume(throwing: Boom())
        } else {
            continuation.resume(with: .success(99))
        }
    }
}

func run() async {
    print("inline \(await inlineValue())")
    print("deferred \(await deferredValue())")
    do {
        let ok = try await throwingValue(false)
        print("ok \(ok)")
        _ = try await throwingValue(true)
    } catch {
        print("caught error")
    }
}
run()
