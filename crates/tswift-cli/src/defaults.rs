//! Native CLI backing for `tswift.defaults.*` (Foundation's `UserDefaults`;
//! see `tswift_foundation::user_defaults` for the wire schema and the value
//! model this store persists — JSON-encoded strings keyed by defaults key).
//!
//! ## Persistence
//!
//! In-process by default: values live only for the process's lifetime. That
//! keeps golden-fixture runs (each a fresh `tswift run` invocation, often
//! executed concurrently by `cargo test`) free of cross-test file contention
//! — a shared on-disk store would make fixtures flaky or order-dependent.
//!
//! Setting the `TSWIFT_DEFAULTS_FILE` environment variable to a path opts
//! into real persistence, matching how Foundation's `UserDefaults.standard`
//! actually survives process restarts: the store loads that file's JSON
//! object at startup and rewrites it after every `set`/`remove`. A read or
//! write failure against that file is swallowed (falls back to in-memory
//! behaviour for that call) rather than aborting the program — losing
//! persistence is not a reason to crash a script that only wanted key-value
//! storage.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use tswift_core::json::{self, Json};
use tswift_core::HostCallHandler;

pub struct DefaultsHandler {
    store: Mutex<HashMap<String, String>>,
    registered: Mutex<HashMap<String, String>>,
    file: Option<PathBuf>,
}

impl DefaultsHandler {
    /// The production constructor: reads `TSWIFT_DEFAULTS_FILE` from the
    /// environment (see the module docs).
    pub fn new() -> Self {
        Self::with_file(std::env::var_os("TSWIFT_DEFAULTS_FILE").map(PathBuf::from))
    }

    /// Build a handler backed by an explicit file path (or `None` for
    /// in-memory only), bypassing the environment variable. Exists so tests
    /// can exercise file persistence without mutating global process state.
    fn with_file(file: Option<PathBuf>) -> Self {
        let store = file
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| json::parse(&text).ok())
            .map(|doc| match doc {
                Json::Object(entries) => entries
                    .into_iter()
                    .filter_map(|(k, v)| match v {
                        Json::Str(s) => Some((k, s)),
                        _ => None,
                    })
                    .collect(),
                _ => HashMap::new(),
            })
            .unwrap_or_default();
        Self {
            store: Mutex::new(store),
            registered: Mutex::new(HashMap::new()),
            file,
        }
    }

    fn persist(&self, store: &HashMap<String, String>) {
        let Some(path) = &self.file else { return };
        let entries: Vec<(String, Json)> = store
            .iter()
            .map(|(k, v)| (k.clone(), Json::Str(v.clone())))
            .collect();
        let text = json::to_string(&Json::Object(entries));
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, text);
    }
}

impl Default for DefaultsHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl HostCallHandler for DefaultsHandler {
    fn call(&self, name: &str, args_json: &str) -> Result<String, String> {
        let Json::Array(args) = json::parse(args_json).map_err(|e| e.to_string())? else {
            return Err(format!("{name}: expected an args array"));
        };
        let str_arg = |i: usize| -> Result<String, String> {
            match args.get(i) {
                Some(Json::Str(s)) => Ok(s.clone()),
                _ => Err(format!(
                    "{name}: expected a String argument at position {i}"
                )),
            }
        };
        match name {
            "tswift.defaults.set" => {
                let key = str_arg(0)?;
                let value = str_arg(1)?;
                let mut store = self.store.lock().map_err(|_| "defaults store poisoned")?;
                store.insert(key, value);
                self.persist(&store);
                Ok("null".to_string())
            }
            "tswift.defaults.get" => {
                // The host fn's *declared* return type is `String?` — its
                // reply must be valid JSON for that type: `null`, or a JSON
                // string whose *content* is the stored value's own JSON
                // encoding (which `tswift-foundation` parses again on its
                // side). Returning the stored text unwrapped would hand back
                // e.g. bare `42` for a stored Int, which fails to decode as
                // `String?`.
                let key = str_arg(0)?;
                let store = self.store.lock().map_err(|_| "defaults store poisoned")?;
                let registered = self
                    .registered
                    .lock()
                    .map_err(|_| "defaults store poisoned")?;
                Ok(match store.get(&key).or_else(|| registered.get(&key)) {
                    Some(stored) => json::to_string(&Json::Str(stored.clone())),
                    None => "null".to_string(),
                })
            }
            "tswift.defaults.remove" => {
                let key = str_arg(0)?;
                let mut store = self.store.lock().map_err(|_| "defaults store poisoned")?;
                store.remove(&key);
                self.persist(&store);
                Ok("null".to_string())
            }
            "tswift.defaults.register" => {
                let document = str_arg(0)?;
                let Json::Object(values) = json::parse(&document)
                    .map_err(|e| format!("tswift.defaults.register: invalid defaults JSON: {e}"))?
                else {
                    return Err(
                        "tswift.defaults.register: defaults must be a JSON object".to_string()
                    );
                };
                let mut registered = self
                    .registered
                    .lock()
                    .map_err(|_| "defaults store poisoned")?;
                for (key, value) in values {
                    registered
                        .entry(key)
                        .or_insert_with(|| json::to_string(&value));
                }
                Ok("null".to_string())
            }
            other => Err(format!("unknown host fn `{other}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_remove_round_trip() {
        // `\"v\"` is the *stored* JSON encoding of the Swift string "v"
        // (what `tswift-foundation`'s `set(_:forKey:)` sends as its `value`
        // argument); the `get` reply wraps that stored text in a JSON string
        // again — the host fn's declared return type is `String?`, so its
        // reply must itself decode as one.
        let handler = DefaultsHandler::new();
        assert_eq!(
            handler
                .call("tswift.defaults.set", r#"["k","\"v\""]"#)
                .unwrap(),
            "null"
        );
        assert_eq!(
            handler.call("tswift.defaults.get", r#"["k"]"#).unwrap(),
            json::to_string(&Json::Str("\"v\"".to_string()))
        );
        assert_eq!(
            handler.call("tswift.defaults.remove", r#"["k"]"#).unwrap(),
            "null"
        );
        assert_eq!(
            handler.call("tswift.defaults.get", r#"["k"]"#).unwrap(),
            "null"
        );
    }

    #[test]
    fn missing_key_returns_null() {
        let handler = DefaultsHandler::new();
        assert_eq!(
            handler
                .call("tswift.defaults.get", r#"["absent"]"#)
                .unwrap(),
            "null"
        );
    }

    #[test]
    fn file_backed_persistence_survives_a_new_handler() {
        // Uses the explicit-path constructor (not the `TSWIFT_DEFAULTS_FILE`
        // env var) so this test is race-free under a parallel test runner.
        let dir = std::env::temp_dir().join(format!(
            "tswift-defaults-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let file = dir.join("defaults.json");
        {
            let handler = DefaultsHandler::with_file(Some(file.clone()));
            handler.call("tswift.defaults.set", r#"["k","1"]"#).unwrap();
        }
        {
            let handler = DefaultsHandler::with_file(Some(file.clone()));
            assert_eq!(
                handler.call("tswift.defaults.get", r#"["k"]"#).unwrap(),
                json::to_string(&Json::Str("1".to_string()))
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
