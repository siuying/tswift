import Foundation

// Common URLError.Code cases and their NSURLError raw values.
print(URLError(.badServerResponse).errorCode)
print(URLError(.cannotConnectToHost).errorCode)
print(URLError(.cannotDecodeContentData).errorCode)
print(URLError(.cannotDecodeRawData).errorCode)
print(URLError(.cannotParseResponse).errorCode)
print(URLError(.dataNotAllowed).errorCode)
print(URLError(.dnsLookupFailed).errorCode)
print(URLError(.httpTooManyRedirects).errorCode)
print(URLError(.networkConnectionLost).errorCode)
print(URLError(.resourceUnavailable).errorCode)
print(URLError(.secureConnectionFailed).errorCode)
print(URLError(.unsupportedURL).errorCode)
print(URLError(.userAuthenticationRequired).errorCode)
print(URLError(.userCancelledAuthentication).errorCode)
print(URLError(.zeroByteResource).errorCode)

// Code identity round-trips through the `.code` accessor.
let err = URLError(.dnsLookupFailed)
print(err.code == .dnsLookupFailed)

// `failingURL` is derived from userInfo, which this runtime does not model,
// so it is honestly nil for a code-only URLError.
print(err.failingURL == nil)
