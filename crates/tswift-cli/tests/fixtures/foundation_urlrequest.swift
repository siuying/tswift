import Foundation

// URLRequest basics
let url = URL(string: "https://api.example.com/v1/items?page=2")!
var req = URLRequest(url: url)
print(req.url!.absoluteString)
print(req.httpMethod ?? "nil")
print(req.timeoutInterval)
print(req.httpBody == nil)
print(req.allHTTPHeaderFields == nil)

// Mutating stored properties
req.httpMethod = "POST"
req.httpBody = Data([104, 105])
req.timeoutInterval = 30
print(req.httpMethod ?? "nil")
print(req.httpBody!.count)
print(req.timeoutInterval)

// Header field methods (case-insensitive)
req.setValue("application/json", forHTTPHeaderField: "Content-Type")
req.addValue("gzip", forHTTPHeaderField: "Accept-Encoding")
req.addValue("br", forHTTPHeaderField: "accept-encoding")
print(req.value(forHTTPHeaderField: "content-type") ?? "nil")
print(req.value(forHTTPHeaderField: "Accept-Encoding") ?? "nil")
req.setValue(nil, forHTTPHeaderField: "Content-Type")
print(req.value(forHTTPHeaderField: "Content-Type") ?? "nil")
print(req.allHTTPHeaderFields!.count)

// Custom timeout init
let quick = URLRequest(url: url, timeoutInterval: 5)
print(quick.timeoutInterval)

// URLResponse
let plain = URLResponse(
    url: URL(string: "https://example.com/files/report.pdf")!,
    mimeType: "application/pdf",
    expectedContentLength: 1024,
    textEncodingName: nil
)
print(plain.url!.absoluteString)
print(plain.mimeType ?? "nil")
print(plain.expectedContentLength)
print(plain.textEncodingName ?? "nil")
print(plain.suggestedFilename ?? "nil")

// HTTPURLResponse
let http = HTTPURLResponse(
    url: URL(string: "https://api.example.com/v1/items")!,
    statusCode: 200,
    httpVersion: "HTTP/1.1",
    headerFields: ["Content-Type": "application/json; charset=utf-8", "Content-Length": "128"]
)!
print(http.statusCode)
print(http.mimeType ?? "nil")
print(http.expectedContentLength)
print(http.value(forHTTPHeaderField: "content-type") ?? "nil")
print(http.allHeaderFields.count)
print(HTTPURLResponse.localizedString(forStatusCode: 200))
print(HTTPURLResponse.localizedString(forStatusCode: 404))
print(HTTPURLResponse.localizedString(forStatusCode: 503))

// URLError
let err = URLError(.timedOut)
print(err.code == .timedOut)
print(err.errorCode)
print(URLError(.badURL).errorCode)
print(URLError(.notConnectedToInternet).errorCode)

func fetchOrThrow() throws -> String {
    throw URLError(.cannotFindHost)
}
do {
    _ = try fetchOrThrow()
} catch let error as URLError {
    print("caught \(error.errorCode)")
}
