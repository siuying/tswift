import Foundation
import TSwiftFFI

/// The host side of the script `URLSession` seam.
///
/// Scripts run through a `TSwiftContext` see `URLSession` as unavailable
/// until the host registers an HTTP handler. The common case is one line:
///
/// ```swift
/// let context = TSwiftContext()
/// context.installURLSessionHTTPHandler()   // scripts now use real URLSession
/// let result = TSwiftCore.run(script, in: context)
/// ```
///
/// The native seam is synchronous (the interpreter is a cooperative
/// single-threaded executor), so the handler blocks the interpreting thread
/// for the duration of each request; the default implementation waits on a
/// semaphore around a real `URLSession` data task.

/// One script-initiated HTTP request, decoded from the FFI request JSON.
public struct TSwiftHTTPRequest: Sendable {
    /// Absolute URL string.
    public let url: String
    /// HTTP method (`GET`, `POST`, ...).
    public let method: String
    /// Header fields in insertion order.
    public let headers: [(name: String, value: String)]
    /// Request body, if any.
    public let body: Data?
    /// Request timeout in seconds.
    public let timeoutSeconds: Double

    /// The request as a Foundation `URLRequest`, or `nil` for an invalid URL.
    public var urlRequest: URLRequest? {
        guard let url = URL(string: url) else { return nil }
        var request = URLRequest(url: url, timeoutInterval: timeoutSeconds)
        request.httpMethod = method
        request.httpBody = body
        for (name, value) in headers {
            request.setValue(value, forHTTPHeaderField: name)
        }
        return request
    }
}

/// The host's answer to a script HTTP request.
public enum TSwiftHTTPResult: Sendable {
    /// A completed exchange (any status code, including 4xx/5xx).
    case response(status: Int, headers: [(name: String, value: String)], body: Data)
    /// A transport-level failure; `code` is a `URLError.Code` case name
    /// (`"timedOut"`, `"cannotFindHost"`, ...) thrown to the script as
    /// `URLError`.
    case failure(code: String, message: String)
}

extension TSwiftContext {
    /// Route script `URLSession` requests to `handler`. The handler runs on
    /// the interpreting thread and must return synchronously (block as
    /// needed). Replaces any previous handler.
    public func setHTTPHandler(_ handler: @escaping (TSwiftHTTPRequest) -> TSwiftHTTPResult) {
        let box = HTTPHandlerBox(handler)
        httpHandlerBox = box
        tswift_set_http_handler(
            handle,
            { userdata, requestJSON, call in
                guard let userdata, let requestJSON else { return }
                let box = Unmanaged<HTTPHandlerBox>.fromOpaque(userdata).takeUnretainedValue()
                let response = box.respond(to: String(cString: requestJSON))
                response.withCString { tswift_http_respond(call, $0) }
            },
            Unmanaged.passUnretained(box).toOpaque()
        )
    }

    /// Remove the registered handler; scripts see `URLSession` as unavailable.
    public func removeHTTPHandler() {
        tswift_set_http_handler(handle, nil, nil)
        httpHandlerBox = nil
    }

    /// Back script `URLSession` requests with a real Foundation `URLSession`
    /// (the platform default transport: system proxies, TLS trust, cookies,
    /// HTTP/2+ — everything the host process gets).
    public func installURLSessionHTTPHandler(session: URLSession = .shared) {
        setHTTPHandler { request in
            guard let urlRequest = request.urlRequest else {
                return .failure(code: "badURL", message: "invalid URL: \(request.url)")
            }
            let semaphore = DispatchSemaphore(value: 0)
            nonisolated(unsafe) var outcome: TSwiftHTTPResult = .failure(
                code: "unknown", message: "no completion"
            )
            session.dataTask(with: urlRequest) { data, response, error in
                if let error {
                    outcome = .failure(
                        code: Self.urlErrorCaseName(for: error),
                        message: error.localizedDescription
                    )
                } else if let http = response as? HTTPURLResponse {
                    var headers: [(String, String)] = []
                    for (name, value) in http.allHeaderFields {
                        headers.append(("\(name)", "\(value)"))
                    }
                    outcome = .response(
                        status: http.statusCode, headers: headers, body: data ?? Data()
                    )
                } else {
                    outcome = .failure(
                        code: "badServerResponse", message: "non-HTTP response"
                    )
                }
                semaphore.signal()
            }.resume()
            semaphore.wait()
            return outcome
        }
    }

    /// The `URLError.Code` case name for `error`, mirroring the runtime's
    /// `URLError` vocabulary (unknown codes degrade to `cannotConnectToHost`).
    static func urlErrorCaseName(for error: Error) -> String {
        guard let urlError = error as? URLError else { return "badServerResponse" }
        let names: [URLError.Code: String] = [
            .cancelled: "cancelled",
            .badURL: "badURL",
            .timedOut: "timedOut",
            .unsupportedURL: "unsupportedURL",
            .cannotFindHost: "cannotFindHost",
            .cannotConnectToHost: "cannotConnectToHost",
            .networkConnectionLost: "networkConnectionLost",
            .dnsLookupFailed: "dnsLookupFailed",
            .httpTooManyRedirects: "httpTooManyRedirects",
            .resourceUnavailable: "resourceUnavailable",
            .notConnectedToInternet: "notConnectedToInternet",
            .badServerResponse: "badServerResponse",
            .userCancelledAuthentication: "userCancelledAuthentication",
            .userAuthenticationRequired: "userAuthenticationRequired",
            .zeroByteResource: "zeroByteResource",
            .cannotDecodeRawData: "cannotDecodeRawData",
            .cannotDecodeContentData: "cannotDecodeContentData",
            .cannotParseResponse: "cannotParseResponse",
            .dataNotAllowed: "dataNotAllowed",
            .secureConnectionFailed: "secureConnectionFailed",
        ]
        return names[urlError.code] ?? "cannotConnectToHost"
    }
}

/// Retained bridge between the C callback and a Swift closure: decodes the
/// request JSON, invokes the handler, encodes the response JSON.
final class HTTPHandlerBox {
    private let handler: (TSwiftHTTPRequest) -> TSwiftHTTPResult

    init(_ handler: @escaping (TSwiftHTTPRequest) -> TSwiftHTTPResult) {
        self.handler = handler
    }

    /// Decode `requestJSON`, run the handler, and encode its result.
    func respond(to requestJSON: String) -> String {
        guard let request = Self.decodeRequest(requestJSON) else {
            return Self.encodeFailure(
                code: "badURL", message: "malformed request JSON from runtime"
            )
        }
        switch handler(request) {
        case let .response(status, headers, body):
            return Self.encodeResponse(status: status, headers: headers, body: body)
        case let .failure(code, message):
            return Self.encodeFailure(code: code, message: message)
        }
    }

    private struct WireRequest: Decodable {
        let url: String
        let method: String
        let timeoutSeconds: Double
        let headers: [[String]]
        let bodyBase64: String?
    }

    private static func decodeRequest(_ json: String) -> TSwiftHTTPRequest? {
        guard let wire = try? JSONDecoder().decode(WireRequest.self, from: Data(json.utf8))
        else { return nil }
        let body = wire.bodyBase64.flatMap { Data(base64Encoded: $0) }
        let headers = wire.headers.compactMap { pair -> (String, String)? in
            pair.count == 2 ? (pair[0], pair[1]) : nil
        }
        return TSwiftHTTPRequest(
            url: wire.url,
            method: wire.method,
            headers: headers,
            body: body,
            timeoutSeconds: wire.timeoutSeconds
        )
    }

    private static func encodeResponse(
        status: Int, headers: [(name: String, value: String)], body: Data
    ) -> String {
        let object: [String: Any] = [
            "status": status,
            "headers": headers.map { [$0.name, $0.value] },
            "bodyBase64": body.base64EncodedString(),
        ]
        return encodeJSON(object)
    }

    private static func encodeFailure(code: String, message: String) -> String {
        encodeJSON(["error": code, "message": message])
    }

    private static func encodeJSON(_ object: [String: Any]) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: object) else {
            return #"{"error":"badServerResponse","message":"response encoding failed"}"#
        }
        return String(decoding: data, as: UTF8.self)
    }
}
