import Foundation

// M4 golden fixture: delegate dispatch on the async data(from:) path.
// Verifies that delegate callbacks (didReceive response + data + didComplete)
// compose correctly with the async/await call surface — i.e. call_closure
// composes with async frames when invoked from a delegate callback inside
// the async data(from:) driver.
//
// Route: "chunksBase64" with two chunks of "async1" (YXN5bmMx, 6 bytes) and
// "async2" (YXN5bmMy, 6 bytes), total 12 bytes.

var callLog: [String] = []
var totalBytesFromDelegate = 0

class AsyncDelegate: URLSessionDataDelegate, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse, completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        callLog.append("response")
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        totalBytesFromDelegate += data.count
        callLog.append("chunk:\(data.count)")
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        if error == nil {
            callLog.append("complete:ok")
        } else {
            callLog.append("complete:error")
        }
    }
}

let delegate = AsyncDelegate()
let config = URLSessionConfiguration.default
let session = URLSession(configuration: config, delegate: delegate, delegateQueue: nil)

// Use the async data(from:) path — not dataTask(with:completionHandler:).
// Wrap in a Task and await its value so the cooperative executor runs the
// body before the print statements (same pattern as foundation_urlsession_async_cancel).
let t = Task { () -> String in
    do {
        let url = URL(string: "http://127.0.0.1:8765/async-stream")!
        let (data, _) = try await session.data(from: url)
        return String(data: data, encoding: .utf8) ?? "decode-error"
    } catch {
        return "error:\(error)"
    }
}
let asyncDataResult = await t.value

// Callback log: response, two chunks, complete
print(callLog.count)
print(callLog[0])
print(callLog[1])
print(callLog[2])
print(callLog[3])
// Bytes via delegate
print(totalBytesFromDelegate)
// Full body from async return
print(asyncDataResult)
