import Testing

@Test(.bug("https://example.com/issues/7"))
func stillBroken() {
    withKnownIssue("not fixed yet") {
        #expect(1 == 2)
    }
}

@Test func healthy() {
    #expect(true)
}
