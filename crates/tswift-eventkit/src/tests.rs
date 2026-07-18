//! Crate tests, including the coverage-tooling registry dump.

use crate::registered_keys;

#[test]
fn every_key_is_type_qualified() {
    for key in registered_keys() {
        assert!(key.contains('.'), "coverage key {key:?} is not Type.member");
    }
}

#[test]
fn enum_cases_are_registered() {
    let keys = registered_keys();
    assert!(keys.iter().any(|k| k == "EKSpan.thisEvent"));
    assert!(keys.iter().any(|k| k == "EKAuthorizationStatus.fullAccess"));
    assert!(keys.iter().any(|k| k == "EKEntityType.event"));
    assert!(keys.iter().any(|k| k == "EKSpan.init"));
}

/// Regenerate `frameworks/eventkit/registered_keys.txt` from the live registry,
/// mirroring `tswift-swiftdata`'s `dump_registered_keys`. Cannot drift: the file
/// is derived from [`registered_keys`].
#[test]
fn dump_registered_keys() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = root.join("frameworks/eventkit/registered_keys.txt");
    let body = registered_keys().join("\n") + "\n";
    std::fs::write(&path, body).expect("write registered_keys.txt");
}
