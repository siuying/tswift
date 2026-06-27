// Generic static methods declared inside a type push their own type bindings.
struct ZeroBox {
    let value: Int
    static func zero() -> ZeroBox { ZeroBox(value: 0) }
}

struct Maker {
    static func make<T: AdditiveArithmetic>(_ seed: T) -> T {
        return T.zero()
    }
}

let z = Maker.make(ZeroBox(value: 9))
print(z.value)
