import Foundation

// Task.cancel() on the containing async Task → data(from:) throws URLError(.cancelled).
// In the cooperative executor, the Task body runs when awaited; cancelling the task
// before awaiting it sets the cancellation flag, which data(from:) checks before
// starting the transport.
let url = URL(string: "http://127.0.0.1:8765/hello")!

let t = Task { () -> String in
    do {
        let _ = try await URLSession.shared.data(from: url)
        return "unexpected success"
    } catch is URLError {
        return "caught URLError cancelled"
    } catch {
        return "unexpected other error"
    }
}
t.cancel()
print(await t.value)
