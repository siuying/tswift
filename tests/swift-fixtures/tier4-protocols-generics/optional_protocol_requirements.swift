// expected-no-diagnostics
// oracle-gap: C msf does not accept @objc optional requirements end-to-end

@objc protocol Delegate {
    func required1()
    @objc optional func willStart() -> Int
    @objc optional var rowCount: Int { get }
    @objc optional func title(for row: Int) -> String
}

class Handler: Delegate {
    func required1() { }
}

func poke(_ d: Delegate) {
    d.required1()
    let _ = d.willStart?()
    let _ = d.rowCount
    let _ = d.title?(for: 1)
}
