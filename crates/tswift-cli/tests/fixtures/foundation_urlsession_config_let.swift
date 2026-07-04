import Foundation

// Headline fidelity: `let`-bound URLSessionConfiguration accepts property
// writes via the interpreter's set_object_field path (Object, not Struct).

// 1. let-binding + mutation.
let config = URLSessionConfiguration.default
config.timeoutIntervalForRequest = 30
print(config.timeoutIntervalForRequest)

// 2. Alias shares the same Object — mutation through alias is visible via config.
let config2 = config
config2.timeoutIntervalForRequest = 45
print(config.timeoutIntervalForRequest)

// 3. URLSession.init COPIES the configuration (Foundation-documented).
//    Post-init mutations to config do NOT affect the session's copy.
let session = URLSession(configuration: config)
config.timeoutIntervalForRequest = 99
print(session.configuration.timeoutIntervalForRequest)

// 4. Each .default access returns a fresh independent Object.
let fresh = URLSessionConfiguration.default
print(fresh.timeoutIntervalForRequest)
