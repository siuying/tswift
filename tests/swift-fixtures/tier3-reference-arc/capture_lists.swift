// expected-no-diagnostics
// oracle-gap: C msf does not accept `self` as a condition-binding pattern

// Capture lists with ownership modifiers, `guard let self = self`, and the
// SE-0345 `if let self` shorthand.

class Owner {
    var name = "o"
    var handler: (() -> String)? = nil

    func setup() {
        handler = { [weak self] in
            guard let self = self else { return "gone" }
            return self.name
        }
    }

    func armShorthand() {
        handler = { [weak self] in
            if let self {
                return self.name
            }
            return "gone"
        }
    }

    func armUnowned() {
        handler = { [unowned self] in self.name }
    }
}

// Value captures with initializers alongside ownership captures.
func mixed(base: Owner) -> () -> String {
    let suffix = "!"
    return { [weak base, tag = suffix] in
        (base?.name ?? "gone") + tag
    }
}
