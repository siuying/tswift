import Foundation

// M4 delegate dispatch: mid-stream scripted failure fires didCompleteWithError
// with a URLError (not nil).  failAfterChunks:1 means: deliver one chunk then
// inject a Failed event.

var gotError = false
var errorCode = ""
var chunkCount = 0

class FailDelegate: URLSessionDataDelegate, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse, completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        chunkCount += 1
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        if let err = error as? URLError {
            gotError = true
            errorCode = "\(err.code)"
        }
    }
}

let delegate = FailDelegate()
var session = URLSession(configuration: URLSessionConfiguration.default, delegate: delegate, delegateQueue: nil)

var handlerCalled = false
var handlerError: Error? = nil

var task = session.dataTask(with: URL(string: "http://127.0.0.1:8765/failing")!) { data, response, error in
    handlerCalled = true
    handlerError = error
}

task.resume()

// Delegate received one chunk before failure
print(chunkCount)
// Delegate got error callback
print(gotError)
// Completion handler also received the error
print(handlerCalled)
print(handlerError != nil)
