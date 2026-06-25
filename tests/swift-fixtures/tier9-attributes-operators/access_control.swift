// expected-no-diagnostics
// Tier 9a — access levels open/public/internal/fileprivate/private, package,
// and private(set).

public struct API {
    public let version: Int
    private let secret: String
    internal var cache: [String: Int] = [:]
    fileprivate var hits = 0
    package var token = ""
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
    func debugReveal() -> String { reveal() }
}

open class Service {
    open func handle() -> Int { 0 }
}

var api = API(version: 1, secret: "x")
api.record()

let _ = (api.version, api.requests, api.debugReveal())
