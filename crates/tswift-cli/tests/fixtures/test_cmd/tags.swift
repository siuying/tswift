import Testing

@Test(.tags(.fast)) func quick() { #expect(1 + 1 == 2) }
@Test(.tags(.slow)) func lengthy() { #expect(2 + 2 == 4) }
@Test func untagged() { #expect(true) }
