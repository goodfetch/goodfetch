//! The `fix` a refused Client adds to a Daemon's refusal (spec #213's Seam
//! 4, ADR 0098), and the Daemon's files: what names a running Daemon, and
//! its runtime file's mode.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the Daemon's files themselves"
)]

use std::path::Path;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::daemon::{DaemonEndpoint, DaemonRuntimeFile};
use goodfetch_test_support::TestEnv;
use rstest::rstest;

use super::files::DaemonFiles;
use super::{RefusedClient, RefusedExecutable, refusal_fix};
use crate::home::Home;
use crate::product::Channel;

/// Covers: story:3-5
#[rstest]
#[case::its_own_executable_is_the_pick(
    Product::Collie,
    RefusedExecutable::Path(Path::new("/project/node_modules/.bin/goodfetch-collie")),
    true,
    "Run `/project/node_modules/.bin/goodfetch-collie daemon restart` to restart the Daemon on its major's pick."
)]
#[case::a_path_with_a_space(
    Product::Foxhound,
    RefusedExecutable::Path(Path::new("/Users/Zoë Doe/.venv/bin/goodfetch-foxhound")),
    true,
    "Run `\"/Users/Zoë Doe/.venv/bin/goodfetch-foxhound\" daemon restart` to restart the Daemon on its major's pick."
)]
#[case::an_older_foxhound_stays_the_pick(
    Product::Collie,
    RefusedExecutable::Path(Path::new("/opt/goodfetch-collie")),
    false,
    "Run `/opt/goodfetch-collie daemon restart` to restart the Daemon on its major's pick. Collie's executable would still not run the Daemon while a Foxhound of the major is installed: install Foxhound 1.5.0 or newer (`cargo install goodfetch-foxhound --locked --features executable`) and run its `daemon restart`."
)]
#[case::a_rust_client_installs_its_executable_first(
    Product::Collie,
    RefusedExecutable::Linked,
    true,
    "Install Collie's executable at 1.5.0 (`cargo install goodfetch-collie --locked --features executable`), then run its `daemon restart` to restart the Daemon on its major's pick."
)]
fn a_refused_clients_fix_names_its_own_executables_daemon_restart(
    #[case] product: Product,
    #[case] executable: RefusedExecutable<'_>,
    #[case] would_be_pick: bool,
    #[case] fix: &str,
) {
    let version = ProductVersion::parse("1.5.0").unwrap();

    let found = refusal_fix(&RefusedClient {
        product,
        version: &version,
        executable,
        channel: Channel::CratesIo,
        would_be_pick,
    });

    assert_eq!(found, fix);
}

fn runtime_file(pid: u32) -> DaemonRuntimeFile {
    DaemonRuntimeFile {
        endpoint: DaemonEndpoint::Unix("daemon.sock".into()),
        token: "token".to_owned(),
        version: ProductVersion::parse("1.5.0").unwrap(),
        pid,
    }
}

/// A runtime file whose Daemon holds no serving lock, as one that died
/// leaves it, names no running Daemon, so a Client replaces it.
///
/// Covers: story:3-11
#[test]
fn a_runtime_file_names_a_running_daemon_only_while_its_serving_lock_is_held() {
    let env = TestEnv::new();
    let files = DaemonFiles::of(
        &Home::new(env.home().to_owned()),
        &ProductVersion::parse("1.5.0").unwrap(),
    );
    files.publish(&runtime_file(7)).unwrap();

    let dead = files.running().unwrap();
    let held = files.serving_lock().unwrap();
    let running = files.running().unwrap();
    drop(held);

    assert_eq!((dead, running), (None, Some(runtime_file(7))));
}

/// Covers: story:3-11, story:3-22
#[cfg(unix)]
#[test]
fn the_runtime_file_is_its_owners_alone_in_its_majors_folder() {
    use std::os::unix::fs::PermissionsExt as _;
    let env = TestEnv::new();
    let files = DaemonFiles::of(
        &Home::new(env.home().to_owned()),
        &ProductVersion::parse("2.0.1").unwrap(),
    );

    files.publish(&runtime_file(7)).unwrap();

    let mode = std::fs::metadata(files.runtime_file())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(
        (files.runtime_file(), mode & 0o777),
        (env.home().join("2").join("daemon.json"), 0o600)
    );
}

/// Unpublishing leaves a runtime file another Daemon wrote since.
///
/// Covers: story:3-138
#[test]
fn a_daemon_unpublishes_only_its_own_runtime_file() {
    let env = TestEnv::new();
    let files = DaemonFiles::of(
        &Home::new(env.home().to_owned()),
        &ProductVersion::parse("1.5.0").unwrap(),
    );
    files.publish(&runtime_file(8)).unwrap();

    files.unpublish(7).unwrap();
    let kept = files.read().unwrap();
    files.unpublish(8).unwrap();

    assert_eq!((kept, files.read().unwrap()), (Some(runtime_file(8)), None));
}
