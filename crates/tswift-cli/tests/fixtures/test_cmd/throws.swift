struct Boom: Error {}
struct Other: Error {}

func explode() throws { throw Boom() }
func quiet() throws {}

@Test func catchesExpectedType() {
  #expect(throws: Boom.self) { try explode() }
}

@Test func neverThrows() {
  #expect(throws: Never.self) { try quiet() }
}

@Test func reportsWrongType() {
  #expect(throws: Boom.self) { throw Other() }
}
