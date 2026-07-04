import Foundation

// Completion-handler happy path: dataTask(with:completionHandler:) + resume()
let url = URL(string: "http://127.0.0.1:8765/hello")!

var receivedData: Data? = nil
var receivedStatus: Int = 0

var task = URLSession.shared.dataTask(with: url) { data, response, error in
    if let data = data, let resp = response as? HTTPURLResponse {
        receivedData = data
        receivedStatus = resp.statusCode
    }
}

// Before resume, state is .suspended
print(task.state == .suspended)
task.resume()

// After resume, state is .completed
print(task.state == .completed)
print(receivedStatus)
print(String(data: receivedData!, encoding: .utf8)!)
print(task.countOfBytesReceived)
print(task.countOfBytesExpectedToReceive)
