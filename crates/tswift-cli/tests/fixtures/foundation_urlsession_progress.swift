import Foundation

// Progress counters with a chunked mock route.
// "chunk1" = base64 "Y2h1bmsx" (6 bytes), "chunk2" = "Y2h1bmsy" (6 bytes).
// Content-Length: 12.
let url = URL(string: "http://127.0.0.1:8765/stream")!

var receivedData: Data? = nil

var task = URLSession.shared.dataTask(with: url) { data, response, error in
    receivedData = data
}

task.resume()

if let data = receivedData {
    print(String(data: data, encoding: .utf8)!)
}
print(task.countOfBytesReceived)
print(task.countOfBytesExpectedToReceive)
print(task.progress.fractionCompleted == 1.0)
