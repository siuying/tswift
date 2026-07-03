import Foundation

let base = "http://127.0.0.1:8765"

func exercise() async {
    do {
        // Plain GET via data(from:)
        let url = URL(string: "\(base)/greeting")!
        let (data, response) = try await URLSession.shared.data(from: url)
        let http = response as! HTTPURLResponse
        print(http.statusCode)
        print(http.mimeType ?? "nil")
        print(String(data: data, encoding: .utf8) ?? "nil")
        print(http.value(forHTTPHeaderField: "X-Request-Id") ?? "nil")

        // POST via data(for:) with body and headers
        var req = URLRequest(url: URL(string: "\(base)/echo")!)
        req.httpMethod = "POST"
        req.httpBody = Data([112, 105, 110, 103])
        req.setValue("text/plain", forHTTPHeaderField: "Content-Type")
        let (body, resp2) = try await URLSession.shared.data(for: req)
        let http2 = resp2 as! HTTPURLResponse
        print(http2.statusCode)
        print(String(data: body, encoding: .utf8) ?? "nil")

        // A 404 is a response, not a thrown error
        let (missing, resp3) = try await URLSession.shared.data(from: URL(string: "\(base)/missing")!)
        print((resp3 as! HTTPURLResponse).statusCode)
        print(missing.count)
    } catch is URLError {
        print("unexpected URLError")
    } catch {
        print("unexpected error")
    }

    // Transport failures throw URLError
    do {
        _ = try await URLSession.shared.data(from: URL(string: "http://127.0.0.1:9/never")!)
        print("unexpected success")
    } catch is URLError {
        print("caught URLError")
    } catch {
        print("caught other")
    }
}

let session = URLSession(configuration: .default)
print(session.configuration.timeoutIntervalForRequest)
print(URLSessionConfiguration.default.timeoutIntervalForResource)

await exercise()
