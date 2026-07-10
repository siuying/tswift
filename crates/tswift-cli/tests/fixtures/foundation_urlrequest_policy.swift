import Foundation

// URLRequest policy/attribution/access-control stored properties.
let url = URL(string: "https://api.example.com/v1/items")!
var req = URLRequest(url: url)

// Defaults match Darwin.
print(req.cachePolicy == .useProtocolCachePolicy)
print(req.networkServiceType == .default)
print(req.attribution == .developer)
print(req.allowsCellularAccess)
print(req.allowsConstrainedNetworkAccess)
print(req.allowsExpensiveNetworkAccess)
print(req.httpShouldHandleCookies)
print(req.httpShouldUsePipelining)
print(req.assumesHTTP3Capable)
print(req.requiresDNSSECValidation)
print(req.allowsPersistentDNS)
print(req.allowsUltraConstrainedNetworkAccess)
print(req.mainDocumentURL == nil)
print(req.cookiePartitionIdentifier ?? "nil")

// Constructing with an explicit cachePolicy:.
let reloading = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 30)
print(reloading.cachePolicy == .reloadIgnoringLocalCacheData)

// Mutating each of the new stored properties.
req.cachePolicy = .returnCacheDataElseLoad
req.networkServiceType = .video
req.attribution = .user
req.allowsCellularAccess = false
req.allowsConstrainedNetworkAccess = false
req.allowsExpensiveNetworkAccess = false
req.httpShouldHandleCookies = false
req.httpShouldUsePipelining = true
req.assumesHTTP3Capable = true
req.requiresDNSSECValidation = true
req.allowsPersistentDNS = false
req.allowsUltraConstrainedNetworkAccess = false
req.mainDocumentURL = URL(string: "https://api.example.com/")!
req.cookiePartitionIdentifier = "partition-1"

print(req.cachePolicy == .returnCacheDataElseLoad)
print(req.networkServiceType == .video)
print(req.attribution == .user)
print(req.allowsCellularAccess)
print(req.allowsConstrainedNetworkAccess)
print(req.allowsExpensiveNetworkAccess)
print(req.httpShouldHandleCookies)
print(req.httpShouldUsePipelining)
print(req.assumesHTTP3Capable)
print(req.requiresDNSSECValidation)
print(req.allowsPersistentDNS)
print(req.allowsUltraConstrainedNetworkAccess)
print(req.mainDocumentURL?.absoluteString ?? "nil")
print(req.cookiePartitionIdentifier ?? "nil")

// Equatable: two otherwise-identical requests compare equal; a differing
// field (cachePolicy) makes them unequal.
var a = URLRequest(url: url)
var b = URLRequest(url: url)
print(a == b)
b.cachePolicy = .reloadIgnoringLocalCacheData
print(a == b)
a.cachePolicy = .reloadIgnoringLocalCacheData
print(a == b)

// httpBodyStream is not modelled in this runtime (Tier B — no InputStream);
// intentionally not exercised here.
