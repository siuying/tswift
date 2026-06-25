// rust-gap: advanced Tier 0-10 spec syntax not yet modelled by the pure-Rust frontend (tracked in #37)
// expected-no-diagnostics
// Tier 9c — declaration/type attributes the frontend must accept.

@discardableResult
func send(_ message: String) -> Int { message.count }

@available(macOS 12.0, iOS 15.0, *)
func modernFeature() -> String { "new" }

public struct Math {
    public var base: Int
    @inlinable public func doubled() -> Int { base * 2 }
}

func run(_ work: @Sendable () -> Void) { work() }

send("ignored result is fine")

if #available(macOS 12.0, *) {
    let _ = modernFeature()
}

let _ = Math(base: 21).doubled()