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

@_specialize(where T == Int)
@_specialize(exported: true, where T == String)
func describe<T>(_ x: T) -> String { return "\(x)" }

@_specialize(exported: false, kind: full, where T == Double)
func identity<T>(_ x: T) -> T { return x }

print(newAPI())
var c = Counter()
c.bump()
c.bump()
print(c.value)
print(c.doubled)
print(describe(5))
print(describe("hi"))
print(identity(1.5))
