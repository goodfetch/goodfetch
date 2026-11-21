use std::path::PathBuf;

use goodfetch_protocol::connection::ProductVersion;

use super::Home;

#[test]
fn the_run_folders_and_the_run_index_sit_in_the_per_major_folder() {
    let home = Home::new(PathBuf::from("home"));
    let version = ProductVersion::parse("1.4.0").unwrap();

    assert_eq!(
        (home.runs(&version), home.run_index(&version)),
        (
            PathBuf::from("home").join("1").join("runs"),
            PathBuf::from("home").join("1").join("run-index.sqlite"),
        )
    );
}

/// Covers: story:3-67
#[test]
fn the_usage_ledger_sits_in_the_home_outside_every_per_major_folder() {
    let home = Home::new(PathBuf::from("home"));

    assert_eq!(
        home.usage_ledger(),
        PathBuf::from("home").join("usage-ledger.sqlite")
    );
}

/// Covers: story:4-69
#[test]
fn the_audit_log_sits_in_the_home_outside_every_per_major_folder() {
    let home = Home::new(PathBuf::from("home"));

    assert_eq!(
        (home.audit_log(), home.audit_log_head()),
        (
            PathBuf::from("home").join("audit-log.sqlite"),
            PathBuf::from("home").join("audit-log-head.json"),
        )
    );
}

/// The known-certificates store sits beside the Usage ledger, outside the
/// per-major folders, so every engine major reads the same pins (ADR 0042).
/// Covers: story:4-43
#[test]
fn the_known_certificates_store_sits_beside_the_usage_ledger() {
    let home = Home::new(PathBuf::from("home"));

    assert_eq!(
        home.known_certificates(),
        PathBuf::from("home").join("known-certificates.sqlite")
    );
}
