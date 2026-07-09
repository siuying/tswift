import Foundation

// Newly-added URLError.Code cases: raw values (NSURLError*).
print(URLError(.unknown).errorCode)
print(URLError(.redirectToNonExistentLocation).errorCode)
print(URLError(.internationalRoamingOff).errorCode)
print(URLError(.callIsActive).errorCode)
print(URLError(.requestBodyStreamExhausted).errorCode)
print(URLError(.appTransportSecurityRequiresSecureConnection).errorCode)
print(URLError(.fileDoesNotExist).errorCode)
print(URLError(.fileIsDirectory).errorCode)
print(URLError(.noPermissionsToReadFile).errorCode)
print(URLError(.dataLengthExceedsMaximum).errorCode)
print(URLError(.serverCertificateHasBadDate).errorCode)
print(URLError(.serverCertificateUntrusted).errorCode)
print(URLError(.serverCertificateHasUnknownRoot).errorCode)
print(URLError(.serverCertificateNotYetValid).errorCode)
print(URLError(.clientCertificateRejected).errorCode)
print(URLError(.clientCertificateRequired).errorCode)
print(URLError(.cannotLoadFromNetwork).errorCode)
print(URLError(.cannotCreateFile).errorCode)
print(URLError(.cannotOpenFile).errorCode)
print(URLError(.cannotCloseFile).errorCode)
print(URLError(.cannotWriteToFile).errorCode)
print(URLError(.cannotRemoveFile).errorCode)
print(URLError(.cannotMoveFile).errorCode)
print(URLError(.downloadDecodingFailedMidStream).errorCode)
print(URLError(.downloadDecodingFailedToComplete).errorCode)
print(URLError(.backgroundSessionRequiresSharedContainer).errorCode)
print(URLError(.backgroundSessionInUseByAnotherProcess).errorCode)
print(URLError(.backgroundSessionWasDisconnected).errorCode)

// Code identity + static case comparison.
let err = URLError(.fileDoesNotExist)
print(err.code == .fileDoesNotExist)

// errorDomain is a static constant on the type, not per-instance.
print(URLError.errorDomain)

// Honest-nil properties: not modeled by this runtime (no userInfo/TLS/
// reachability/background-session machinery).
print(err.failureURLString == nil)
print(err.failureURLPeerTrust == nil)
print(err.networkUnavailableReason == nil)
print(err.backgroundTaskCancelledReason == nil)
print(err.downloadTaskResumeData == nil)
print(err.uploadTaskResumeData == nil)
