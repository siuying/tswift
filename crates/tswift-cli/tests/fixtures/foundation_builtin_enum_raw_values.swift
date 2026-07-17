import Foundation

print(URLRequest.CachePolicy(rawValue: 4)?.rawValue ?? -1)
print(URLRequest.CachePolicy(rawValue: 99) == nil)
print(URLError.Code(rawValue: -1001)?.rawValue ?? 0)
print(URLError.Code(rawValue: -1001) == .timedOut)
