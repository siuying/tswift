//! Combines every native host-service backing (`tswift.defaults.*`,
//! `tswift.fs.*`, …) into the single [`tswift_core::HostCallHandler`] the
//! interpreter's host bridge supports as its one default handler
//! (`Interpreter::set_host_call_handler`; see `tswift_core::host_bridge`).
//!
//! Dispatch routes on the function's namespace via
//! [`tswift_core::HostService::for_function`] rather than a manual prefix
//! match, so a new service only needs a new match arm here, not a new
//! parsing rule.

use tswift_core::{HostCallHandler, HostService};

use crate::defaults::DefaultsHandler;
use crate::fs::FsHandler;

pub struct CliHostHandler {
    defaults: DefaultsHandler,
    fs: FsHandler,
}

impl CliHostHandler {
    pub fn new() -> Self {
        Self {
            defaults: DefaultsHandler::new(),
            fs: FsHandler::new(),
        }
    }
}

impl Default for CliHostHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl HostCallHandler for CliHostHandler {
    fn call(&self, name: &str, args_json: &str) -> Result<String, String> {
        match HostService::for_function(name) {
            Some(HostService::Defaults) => self.defaults.call(name, args_json),
            Some(HostService::FileSystem) => self.fs.call(name, args_json),
            Some(HostService::Database) | None => {
                Err(format!("no native handler for host fn `{name}`"))
            }
        }
    }
}
