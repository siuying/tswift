import Foundation

// Pre-flight cancel: cancel() before resume() completes with URLError(.cancelled)
// without touching the transport.
let url = URL(string: "http://127.0.0.1:8765/hello")!

var caughtCancel = false
var receivedData: Data? = nil

var task = URLSession.shared.dataTask(with: url) { data, response, error in
    receivedData = data
    if let e = error as? URLError, e.code == .cancelled {
        caughtCancel = true
    }
}

task.cancel()
task.resume()

print(caughtCancel)
print(receivedData == nil)
print(task.state == .completed)
