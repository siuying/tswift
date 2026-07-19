func add(_ a: Int, _ b: Int) -> Int { a + b }

@Test func addition() {
  #expect(add(1, 1) == 3)
}
