import Foundation

// M4 delegate dispatch: delegate cancels the request by returning .cancel
// from the response-received completionHandler.
// The driver should cancel the transport and deliver URLError(.cancelled).

var dispositionCallbackFired = false
var completeErrorCode = ""

class CancelDelegate: URLSessionDataDelegate, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse, completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        dispositionCallbackFired = true
        // Cancel the request from within the delegate.
        completionHandler(.cancel)
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        if let err = error as? URLError {
            completeErrorCode = "\(err.code)"
        }
    }
}

let delegate = CancelDelegate()
var session = URLSession(configuration: URLSessionConfiguration.default, delegate: delegate, delegateQueue: nil)

var handlerError: Error? = nil

var task = session.dataTask(with: URL(string: "http://127.0.0.1:8765/cancel-me")!) { data, response, error in
    handlerError = error
}

task.resume()

// Disposition callback fired
print(dispositionCallbackFired)
// Delegate got didCompleteWithError(.cancelled)
print(completeErrorCode)
// Completion handler received URLError(.cancelled)
if let err = handlerError as? URLError {
    print(err.code == .cancelled)
} else {
    print(false)
}
