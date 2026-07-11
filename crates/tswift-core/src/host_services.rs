//! Host-services vocabulary and install-time capability gating.
//!
//! Some framework builtins are not pure Rust: they reach the embedding through
//! the host-call bridge ([`crate::host_bridge`]) to touch a key-value defaults
//! store, the file system, or a local (e.g. SQL) database. Whether those
//! services exist depends on the *platform* backing the interpreter — a
//! browser page, an iOS app, or the native CLI each back a different subset.
//!
//! This module is the **generic** vocabulary shared across all of that: it
//! names the service categories, owns their stable host-function namespaces,
//! and provides an install-time [`Capabilities`] set that an embedding declares
//! once (at context creation) and threads into each framework's `install`. A
//! framework gates its host-backed APIs on the set so an absent service yields
//! a clean *"unavailable on this platform"* diagnostic ([`CapabilityError`])
//! instead of a runtime host-probe surprise.
//!
//! Deliberately framework-agnostic: core knows the *categories* and their
//! namespaces, never the concrete APIs layered on top (those live in the
//! framework crates that own them).

/// A category of host-backed service reachable through the host-call bridge.
///
/// Each service owns a stable namespace prefix under which its host functions
/// are registered (e.g. `tswift.defaults.set`). The categories are intentionally
/// coarse and framework-agnostic — a capability, not an API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostService {
    /// Key-value persistence (a defaults store), namespace `tswift.defaults`.
    Defaults,
    /// File-system access, namespace `tswift.fs`.
    FileSystem,
    /// A local persistent store / database, namespace `tswift.db`.
    Database,
}

impl HostService {
    /// Every known service, in declaration order.
    pub const ALL: [HostService; 3] = [Self::Defaults, Self::FileSystem, Self::Database];

    /// The stable host-function namespace prefix owned by this service.
    ///
    /// Host functions belonging to the service are registered as
    /// `"{namespace}.{fn}"` (e.g. `tswift.defaults.set`).
    pub const fn namespace(self) -> &'static str {
        match self {
            Self::Defaults => "tswift.defaults",
            Self::FileSystem => "tswift.fs",
            Self::Database => "tswift.db",
        }
    }

    /// The single-bit mask this service occupies in a [`Capabilities`] set.
    const fn bit(self) -> u8 {
        match self {
            Self::Defaults => 1 << 0,
            Self::FileSystem => 1 << 1,
            Self::Database => 1 << 2,
        }
    }

    /// Build a fully-qualified host-function name in this service's namespace:
    /// `service.function("set")` → `"tswift.defaults.set"`.
    pub fn function(self, name: &str) -> String {
        format!("{}.{name}", self.namespace())
    }

    /// Whether `host_fn` is a function in this service's namespace (i.e. it is
    /// exactly `"{namespace}.{something}"`).
    pub fn owns(self, host_fn: &str) -> bool {
        host_fn
            .strip_prefix(self.namespace())
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(|name| !name.is_empty())
    }

    /// Resolve the service whose namespace owns `host_fn`, if any.
    pub fn for_function(host_fn: &str) -> Option<HostService> {
        Self::ALL.into_iter().find(|service| service.owns(host_fn))
    }

    /// Resolve the service that owns the exact `namespace` string, if any.
    ///
    /// This is the vocabulary an embedding uses to turn an *explicit* host
    /// declaration — a list of namespace strings the host promises to back —
    /// into a [`Capabilities`] set (see [`Capabilities::from_namespaces`]).
    pub fn for_namespace(namespace: &str) -> Option<HostService> {
        Self::ALL
            .into_iter()
            .find(|service| service.namespace() == namespace)
    }
}

/// The set of host services an embedding declares its host backs.
///
/// Built once at context creation from what the platform actually provides
/// (native CLI backs everything; a browser page or iOS app back a subset) and
/// threaded into each framework's install, which gates host-backed APIs on it.
///
/// A stored bitset over [`HostService`] — cheap to copy and pass by value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    bits: u8,
}

impl Capabilities {
    /// No host services available — the safe default for a bare interpreter
    /// with no platform behind it.
    pub const fn none() -> Self {
        Self { bits: 0 }
    }

    /// Every known host service available. The native CLI backs all of them.
    pub const fn all() -> Self {
        Self {
            bits: HostService::Defaults.bit()
                | HostService::FileSystem.bit()
                | HostService::Database.bit(),
        }
    }

    /// This set plus `service` (chainable): `Capabilities::none().with(Defaults)`.
    pub const fn with(mut self, service: HostService) -> Self {
        self.bits |= service.bit();
        self
    }

    /// Build a set from an iterator of services.
    pub fn from_services(services: impl IntoIterator<Item = HostService>) -> Self {
        let mut caps = Self::none();
        for service in services {
            caps = caps.with(service);
        }
        caps
    }

    /// Build a set from an *explicit* list of host-service namespace strings the
    /// embedding declares its host backs (e.g. `["tswift.defaults", "tswift.fs"]`).
    ///
    /// Each recognised namespace enables its whole service; unknown strings are
    /// ignored. This is the sound alternative to inferring a service from the
    /// presence of individual registered functions: a service is available iff
    /// the host explicitly declares its namespace.
    pub fn from_namespaces<'a>(namespaces: impl IntoIterator<Item = &'a str>) -> Self {
        let mut caps = Self::none();
        for namespace in namespaces {
            if let Some(service) = HostService::for_namespace(namespace) {
                caps = caps.with(service);
            }
        }
        caps
    }

    /// Whether `service` is available.
    pub const fn contains(self, service: HostService) -> bool {
        self.bits & service.bit() != 0
    }

    /// Gate check for a host-backed API: `Ok(())` when `service` is available,
    /// else a [`CapabilityError`] naming `api` and the missing service's
    /// namespace — the clean *"unavailable on this platform"* diagnostic a
    /// framework raises instead of probing the host at call time and surfacing
    /// a lower-level failure. Any human-readable elaboration beyond the
    /// namespace string belongs to the framework crate that owns `api`, not
    /// core.
    pub fn require(self, service: HostService, api: &str) -> Result<(), CapabilityError> {
        if self.contains(service) {
            Ok(())
        } else {
            Err(CapabilityError {
                service,
                api: api.to_string(),
            })
        }
    }
}

/// A host service required by an API is not backed by the current platform.
///
/// Its [`Display`][std::fmt::Display] is the user-facing *"unavailable on this
/// platform"* message; a framework typically raises it as a Swift-catchable
/// error (or a runtime diagnostic) at the gated call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityError {
    /// The missing service.
    pub service: HostService,
    /// The API that required it (an API name supplied by the caller).
    pub api: String,
}

impl std::fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Core stays framework-agnostic: the diagnostic references only the
        // opaque service namespace, never a framework-specific description.
        write!(
            f,
            "{} is unavailable on this platform: the host does not provide the '{}' service",
            self.api,
            self.service.namespace()
        )
    }
}

impl std::error::Error for CapabilityError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_are_stable() {
        assert_eq!(HostService::Defaults.namespace(), "tswift.defaults");
        assert_eq!(HostService::FileSystem.namespace(), "tswift.fs");
        assert_eq!(HostService::Database.namespace(), "tswift.db");
    }

    #[test]
    fn function_builds_qualified_name() {
        assert_eq!(HostService::Defaults.function("set"), "tswift.defaults.set");
    }

    #[test]
    fn owns_only_own_namespace_functions() {
        assert!(HostService::Defaults.owns("tswift.defaults.set"));
        // Bare namespace with no function part is not owned.
        assert!(!HostService::Defaults.owns("tswift.defaults"));
        assert!(!HostService::Defaults.owns("tswift.defaults."));
        // A different service's namespace is not owned.
        assert!(!HostService::Defaults.owns("tswift.fs.read"));
        // A prefix collision (`tswift.defaultsX`) must not match.
        assert!(!HostService::Defaults.owns("tswift.defaultsX.set"));
    }

    #[test]
    fn for_function_resolves_service() {
        assert_eq!(
            HostService::for_function("tswift.fs.write"),
            Some(HostService::FileSystem)
        );
        assert_eq!(
            HostService::for_function("tswift.db.query"),
            Some(HostService::Database)
        );
        assert_eq!(HostService::for_function("print"), None);
    }

    #[test]
    fn for_namespace_resolves_only_exact_matches() {
        assert_eq!(
            HostService::for_namespace("tswift.defaults"),
            Some(HostService::Defaults)
        );
        assert_eq!(
            HostService::for_namespace("tswift.fs"),
            Some(HostService::FileSystem)
        );
        // A function name is not a namespace.
        assert_eq!(HostService::for_namespace("tswift.defaults.set"), None);
        assert_eq!(HostService::for_namespace("unknown"), None);
    }

    #[test]
    fn all_and_none_bitset() {
        let all = Capabilities::all();
        for service in HostService::ALL {
            assert!(all.contains(service));
        }
        let none = Capabilities::none();
        for service in HostService::ALL {
            assert!(!none.contains(service));
        }
        assert_eq!(Capabilities::default(), Capabilities::none());
    }

    #[test]
    fn with_is_additive_and_independent() {
        let caps = Capabilities::none().with(HostService::Defaults);
        assert!(caps.contains(HostService::Defaults));
        assert!(!caps.contains(HostService::FileSystem));
        assert!(!caps.contains(HostService::Database));
    }

    #[test]
    fn from_services_collects() {
        let caps = Capabilities::from_services([HostService::Defaults, HostService::Database]);
        assert!(caps.contains(HostService::Defaults));
        assert!(!caps.contains(HostService::FileSystem));
        assert!(caps.contains(HostService::Database));
    }

    #[test]
    fn from_namespaces_enables_declared_services() {
        let caps = Capabilities::from_namespaces([
            "tswift.defaults",
            "tswift.fs",
            "tswift.defaults.set", // ignored: a function name, not a namespace
            "unknown",             // ignored: not a known namespace
        ]);
        assert!(caps.contains(HostService::Defaults));
        assert!(caps.contains(HostService::FileSystem));
        assert!(!caps.contains(HostService::Database));
    }

    #[test]
    fn require_passes_when_available() {
        assert!(Capabilities::all()
            .require(HostService::Defaults, "SomeDefaultsAPI")
            .is_ok());
    }

    #[test]
    fn require_fails_with_named_diagnostic() {
        let err = Capabilities::none()
            .require(HostService::Defaults, "SomeDefaultsAPI")
            .unwrap_err();
        assert_eq!(err.service, HostService::Defaults);
        assert_eq!(err.api, "SomeDefaultsAPI");
        let msg = err.to_string();
        assert!(msg.contains("SomeDefaultsAPI"), "{msg}");
        assert!(msg.contains("unavailable on this platform"), "{msg}");
        // Diagnostic references the opaque namespace, not a framework label.
        assert!(msg.contains("tswift.defaults"), "{msg}");
    }
}
