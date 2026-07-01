// expected-no-diagnostics
// oracle-gap: C msf predates @backDeployed and @_specialize attribute arguments

// @backDeployed on free functions, methods, and accessors — accepted, no
// runtime effect (there is no back-deployment target in the interpreter).
// Realistic Swift 6 shape: public API annotated @available earlier than the
// back-deployment boundary.

@available(iOS 15.0, macOS 12.0, *)
@backDeployed(before: iOS 17, macOS 14)
public func newAPI() -> Int { return 42 }

public struct Counter {
  public var value = 0

  public init() {}

  @available(iOS 15.0, macOS 12.0, watchOS 8.0, *)
  @backDeployed(before: iOS 17.0, macOS 14.0, watchOS 10.0)
  public mutating func bump() { value += 1 }

  @available(iOS 15.0, *)
  @backDeployed(before: iOS 17)
  public var doubled: Int { return value * 2 }
}

// @_specialize is a perf hint — accepted and ignored. Multiple attributes and
// the exported/kind arguments must all parse.

@_specialize(where T == Int)
@_specialize(exported: true, where T == String)
func describe<T>(_ x: T) -> String { return "\(x)" }

@_specialize(exported: false, kind: full, where T == Double)
func identity<T>(_ x: T) -> T { return x }
