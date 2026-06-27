// Access levels (open/public/internal/fileprivate/private/package) and
// private(set) are accepted and run. A custom init applies stored-property
// defaults before its body, so members untouched by the init keep their
// defaults.
public struct API {
    public let version: Int
    private let secret: String
    internal var cache: [String: Int] = [:]
    fileprivate var hits = 0
    package var token = "t0"
    public private(set) var requests = 0

    public init(version: Int, secret: String) {
        self.version = version
        self.secret = secret
    }

    public mutating func record() {
        requests += 1
        hits += 1
    }

    private func reveal() -> String { secret }
    func describe() -> String {
        "v\(version) reqs=\(requests) hits=\(hits) token=\(token) secret=\(reveal())"
    }
}

var api = API(version: 1, secret: "shh")
api.record()
api.record()
print(api.describe())
print(api.requests)

open class Service {
    open func handle() -> Int { 0 }
}

final class EchoService: Service {
    override func handle() -> Int { 7 }
}

let svc: Service = EchoService()
print(svc.handle())
