// `Task.sleep` in its several spellings — the legacy `nanoseconds:` overload and
// the `Duration`-based `for:` overload (including the `.seconds` shorthand). On
// the cooperative executor sleeping is a no-op, but every spelling must be
// accepted so real concurrency code type-checks and runs.
func run() async {
    do {
        try await Task.sleep(nanoseconds: 1_000)
        try await Task.sleep(for: .seconds(1))
        try await Task.sleep(for: .milliseconds(250))
        print("done")
    } catch {
        print("error")
    }
}
run()
