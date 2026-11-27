use rstest::rstest;
use serde_json::{Value, json};

use super::{DigestMismatch, LockedPlugin, LockfileError, PluginLockfile};
use crate::catalog::ProtocolVersion;

const KETTLE: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const OTHER: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

fn locked(name: &str, digest: &str) -> LockedPlugin {
    LockedPlugin {
        name: name.to_owned(),
        version: "1.0.0".to_owned(),
        digest: digest.to_owned(),
        signer: None,
        approvals: None,
    }
}

/// A lockfile reads back as it was written, its entries in name order
/// whatever order they were pinned in.
/// Covers: story:7-5
#[test]
fn a_lockfile_reads_back_its_pins_in_name_order() {
    let mut lockfile = PluginLockfile::empty();
    lockfile.lock(locked("kettle", KETTLE)).unwrap();
    lockfile.lock(locked("archive", OTHER)).unwrap();

    let read = PluginLockfile::read(serde_json::to_value(&lockfile).unwrap()).unwrap();

    assert_eq!(
        read.plugins
            .iter()
            .map(|plugin| plugin.name.as_str())
            .collect::<Vec<_>>(),
        ["archive", "kettle"]
    );
}

/// The same pin again keeps the lockfile as it was; another digest for a
/// name it pins is refused, naming both.
/// Covers: story:7-5
#[rstest]
#[case::the_same_digest(KETTLE, Ok(()))]
#[case::another_digest(OTHER, Err(DigestMismatch {
    name: "kettle".to_owned(),
    locked: KETTLE.to_owned(),
    found: OTHER.to_owned(),
}))]
fn a_name_the_lockfile_pins_keeps_its_digest(
    #[case] digest: &str,
    #[case] expected: Result<(), DigestMismatch>,
) {
    let mut lockfile = PluginLockfile::empty();
    lockfile.lock(locked("kettle", KETTLE)).unwrap();

    let outcome = lockfile.lock(locked("kettle", digest));

    assert_eq!(
        (
            outcome,
            lockfile.plugins.len(),
            lockfile
                .locked("kettle")
                .map(|plugin| plugin.digest.as_str())
        ),
        (expected, 1, Some(KETTLE))
    );
}

/// The version is read before the rest, so a newer lockfile with fields
/// this reader does not know still names its version.
#[test]
fn a_newer_lockfile_names_its_version_before_its_fields_are_read() {
    let document = json!({ "version": "0.2", "plugin": [{ "name": "kettle", "mirror": "x" }] });

    assert_eq!(
        PluginLockfile::version_of(&document).unwrap(),
        ProtocolVersion { major: 0, minor: 2 }
    );
}

#[rstest]
#[case::twice(json!({ "version": "0.1", "plugin": [
    { "name": "kettle", "version": "1.0.0", "digest": KETTLE },
    { "name": "kettle", "version": "1.0.1", "digest": OTHER },
] }), "it pins `kettle` twice")]
#[case::unknown_field(json!({ "version": "0.1", "mirror": "x" }), "unknown field `mirror`")]
fn a_lockfile_that_does_not_read_is_refused(#[case] document: Value, #[case] reason: &str) {
    let refused = PluginLockfile::read(document).unwrap_err();

    assert!(
        matches!(
            refused,
            LockfileError::Duplicate(_) | LockfileError::Malformed(_)
        ) && refused.to_string().contains(reason),
        "{refused}"
    );
}
