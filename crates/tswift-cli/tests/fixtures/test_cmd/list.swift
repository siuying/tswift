@Test("adds two numbers")
func addition() { #expect(1 + 1 == 2) }

@Test(arguments: [2, 4, 6]) func even(x: Int) { #expect(x % 2 == 0) }

@Test(.disabled("under maintenance")) func skipMe() { #expect(false) }

struct MathSuite {
  @Test func inSuite() { #expect(true) }
}
