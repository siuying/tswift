// expected-no-diagnostics
// Tier 9d — built-in #file/#line/#function/#column and #if branch selection.

func trace(file: String = #file, line: Int = #line, function: String = #function) -> String {
    "\(function) at \(file):\(line)"
}

let column = #column
let where_ = trace()

// The inactive branch must be skipped, so its #error never fires.
#if false
#error("this branch is never compiled")
#endif

// Availability conditions parse and type-check.
if #available(macOS 10.15, *) {
    let _ = "available"
}
if #unavailable(iOS 13) {
    let _ = "legacy"
}

let _ = (column, where_)
