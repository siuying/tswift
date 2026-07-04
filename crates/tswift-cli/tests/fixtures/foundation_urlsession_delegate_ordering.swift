import Foundation

// M4 delegate dispatch: callback ordering with chunked body.
// Verifies: didReceive(response), multiple didReceive(data) calls (one per
// chunk), and didCompleteWithError(nil) all fire in order.
// "chunk1" = "Y2h1bmsx" (6 bytes), "chunk2" = "Y2h1bmsy" (6 bytes).

var callLog: [String] = []
var totalDataReceived = 0

class ChunkDelegate: URLSessionDataDelegate, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse, completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        callLog.append("response")
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        totalDataReceived += data.count
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

let delegate = ChunkDelegate()
let config = URLSessionConfiguration.default
var session = URLSession(configuration: config, delegate: delegate, delegateQueue: nil)

var receivedData: Data? = nil
var task = session.dataTask(with: URL(string: "http://127.0.0.1:8765/stream")!) { data, response, error in
    receivedData = data
}

task.resume()

// Callback ordering: response, then chunks, then complete
print(callLog.count)
print(callLog[0])
print(callLog[1])
print(callLog[2])
print(callLog[3])
// Total bytes: 12
print(totalDataReceived)
// Completion handler also received full data
if let d = receivedData {
    print(String(data: d, encoding: .utf8)!)
}
