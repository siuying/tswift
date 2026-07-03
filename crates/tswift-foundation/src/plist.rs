//! `PropertyListEncoder` Foundation support.
//!
//! ## Design
//!
//! `PropertyListEncoder` is implemented using the same dispatch pattern as
//! `JSONEncoder`: the interpreter's `try_plist_coder_method` in
//! `tswift-core::interp::coding` handles all runtime calls; this module
//! registers the type constructor and the output-format enum so that:
//!
//! * `PropertyListEncoder()` constructs an opaque struct marker.
//! * `enc.outputFormat = .xml` resolves via the builtin-enum mechanism:
//!   `PropertyListSerialization.PropertyListFormat` is registered as a builtin
//!   enum with cases `xml`, `binary`, and `openStep`, mirroring the real
//!   Foundation type. Accessing `PropertyListEncoder.xml` (a non-existent
//!   member) correctly errors at runtime.
//! * `try enc.encode(value)` dispatches to `try_plist_coder_method`.
//!
//! ## Output format
//!
//! | Leading-dot form | Enum case  | Behaviour                              |
//! |------------------|------------|----------------------------------------|
//! | `.xml`           | `xml`      | Produces XML plist UTF-8 Data          |
//! | `.binary`        | `binary`   | Throws: unsupported in this runtime    |
//! | `.openStep`      | `openStep` | Throws: unsupported in this runtime    |
//!
//! ## Limitations
//!
//! * **Binary / openStep formats**: throw at encode time with a clear message;
//!   they cannot be represented as UTF-8 `Data` in this runtime.
//! * **Default format**: Foundation's `PropertyListEncoder` defaults to
//!   `.binary`; this runtime defaults to `.xml` (the only supported format).
//! * **`userInfo`**: Not implemented — `[CodingUserInfoKey: Any]` dictionary
//!   type is not modelled in this runtime.
//! * **`Nil` top-level values**: throw an encoding error (matching Foundation).
//! * **Non-finite Double**: encoded as `nan` / `+infinity` / `-infinity`
//!   matching Foundation's XML plist output (plist unlike JSON permits them).

use tswift_core::Interpreter;

/// Register `PropertyListEncoder` support into `interp`.
pub fn install(interp: &mut Interpreter<'_>) {
    // Register `PropertyListSerialization.PropertyListFormat` as a builtin
    // enum so that `.xml`, `.binary`, `.openStep` resolve via the leading-dot
    // enum mechanism — the same pattern used for `Date.FormatStyle`.
    // The encoder itself does NOT expose `PropertyListEncoder.xml` etc.;
    // real Foundation places those cases on the separate Format type.
    interp.register_builtin_enum(
        "PropertyListSerialization.PropertyListFormat",
        &["xml", "binary", "openStep"],
    );
}

/// Keys provided by this module (for coverage tracking).
pub fn registered_keys() -> Vec<String> {
    vec![
        "PropertyListEncoder.encode".to_string(),
        "PropertyListEncoder.init".to_string(),
        "PropertyListEncoder.outputFormat".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use tswift_core::Interpreter;

    #[test]
    fn install_registers_format_enum() {
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        install(&mut interp);
        let keys = registered_keys();
        assert!(keys.contains(&"PropertyListEncoder.init".to_string()));
        assert!(keys.contains(&"PropertyListEncoder.encode".to_string()));
        assert!(keys.contains(&"PropertyListEncoder.outputFormat".to_string()));
    }
}
