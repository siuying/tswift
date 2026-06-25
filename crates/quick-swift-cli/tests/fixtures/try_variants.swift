enum E: Error { case fail }
func risky(_ ok: Bool) throws -> Int {
    if ok { return 42 }
    throw E.fail
}
print((try? risky(true)) ?? -1)
print((try? risky(false)) ?? -1)
print(try! risky(true))
