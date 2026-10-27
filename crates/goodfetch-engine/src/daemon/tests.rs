//! The `fix` a refused Client adds to a Daemon's refusal (spec #213's Seam
//! 4, ADR 0098), and the Daemon's files: what names a running Daemon, and
//! its runtime file's mode.
#![cfg_attr(
    unix,
    expect(
        clippy::disallowed_methods,
        reason = "the tests read the Daemon's files themselves"
    )
)]

use std::path::Path;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::console::ConsoleState;
use goodfetch_protocol::daemon::{
    DaemonEndpoint, DaemonRuntimeFile, InstalledService, ServiceManager,
};
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
        console: ConsoleState::Serving("http://127.0.0.1:7801/".to_owned()),
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

/// Each major, and each `0.y` minor during previews, has its own runtime
/// file and socket, which both Products share (ADRs 0030 and 0098).
///
/// Covers: story:3-22
#[rstest]
#[case::a_release("2.0.1", "2")]
#[case::a_later_patch_of_the_major("2.7.0", "2")]
#[case::a_preview_minor("0.4.2", "0.4")]
#[case::the_next_preview_minor("0.5.0", "0.5")]
fn each_major_has_its_own_runtime_file_and_socket(#[case] version: &str, #[case] major: &str) {
    let files = DaemonFiles::of(
        &Home::new("home".into()),
        &ProductVersion::parse(version).unwrap(),
    );

    assert_eq!(
        (files.runtime_file(), files.socket()),
        (
            Path::new("home").join(major).join("daemon.json"),
            Path::new("home").join(major).join("daemon.sock")
        )
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

/// A socket-activated Daemon's persistent token sits in a file of the
/// major's folder that only its owner reads, and a rotation replaces it
/// with another from the OS generator (ADR 0031).
///
/// Covers: story:3-21
#[cfg(unix)]
#[test]
fn the_persistent_token_is_its_owners_alone_and_a_rotation_replaces_it() {
    use std::os::unix::fs::PermissionsExt as _;
    let env = TestEnv::new();
    let files = DaemonFiles::of(
        &Home::new(env.home().to_owned()),
        &ProductVersion::parse("1.5.0").unwrap(),
    );

    let none = files.read_token().unwrap();
    let first = files.persistent_token().unwrap();
    let again = files.persistent_token().unwrap();
    let rotated = files.rotate_token().unwrap();
    let mode = std::fs::metadata(files.token_file())
        .unwrap()
        .permissions()
        .mode();

    assert_eq!(
        (
            none,
            first == again,
            rotated == first,
            files.read_token().unwrap(),
            rotated.len(),
            files.token_file(),
            mode & 0o777,
        ),
        (
            None,
            true,
            false,
            Some(rotated.clone()),
            64,
            env.home().join("1").join("daemon.token"),
            0o600
        )
    );
}

/// The HTTP listener's token is a token of its own, its owner's alone,
/// made once and replaced only by a rotation (spec #608).
///
/// Covers: story:11-214
#[cfg(unix)]
#[test]
fn the_http_listener_token_is_its_owners_alone_and_a_rotation_replaces_it() {
    use std::os::unix::fs::PermissionsExt as _;
    let env = TestEnv::new();
    let files = DaemonFiles::of(
        &Home::new(env.home().to_owned()),
        &ProductVersion::parse("1.5.0").unwrap(),
    );

    let first = files.http_listener_token().unwrap();
    let again = files.http_listener_token().unwrap();
    let persistent = files.persistent_token().unwrap();
    let rotated = files.rotate_http_listener_token().unwrap();
    let mode = std::fs::metadata(files.http_listener_token_file())
        .unwrap()
        .permissions()
        .mode();

    assert_eq!(
        (
            first == again,
            first == persistent,
            rotated == first,
            files.http_listener_token().unwrap() == rotated,
            files.http_listener_token_file(),
            mode & 0o777,
        ),
        (
            true,
            false,
            false,
            true,
            env.home().join("1").join("http-listener.token"),
            0o600
        )
    );
}

/// A service's socket stays where its service manager listens across
/// starts: in the major's folder when its path is short enough, otherwise
/// in a folder of the user's own, named by that folder's path.
///
/// Covers: story:3-21
#[cfg(unix)]
#[test]
fn a_services_socket_is_the_same_at_every_start_and_short_enough() {
    use std::os::unix::fs::PermissionsExt as _;
    let env = TestEnv::new();
    let short = DaemonFiles::of(
        &Home::new(std::path::PathBuf::from("/h")),
        &ProductVersion::parse("1.5.0").unwrap(),
    );
    let long_home = env
        .home()
        .join("a folder whose name is long enough to pass the limit of the OS's socket paths");
    let long = DaemonFiles::of(
        &Home::new(long_home),
        &ProductVersion::parse("1.5.0").unwrap(),
    );

    let first = long.service_socket().unwrap();
    let again = long.service_socket().unwrap();
    let mode = std::fs::metadata(first.parent().unwrap())
        .unwrap()
        .permissions()
        .mode();

    assert_eq!(
        (
            short.service_socket().unwrap(),
            first == again,
            first.as_os_str().len() <= 103,
            mode & 0o777,
        ),
        (
            std::path::PathBuf::from("/h/1/daemon.sock"),
            true,
            true,
            0o700
        )
    );
}

/// The installed service is recorded in the major's folder until it is
/// forgotten.
///
/// Covers: story:3-21
#[test]
fn the_installed_service_is_recorded_until_it_is_forgotten() {
    let env = TestEnv::new();
    let files = DaemonFiles::of(
        &Home::new(env.home().to_owned()),
        &ProductVersion::parse("1.5.0").unwrap(),
    );
    let service = InstalledService {
        manager: ServiceManager::Systemd,
        name: "goodfetch-daemon-1-0123abcd".to_owned(),
        socket: Some(env.home().join("1").join("daemon.sock")),
        console: Some("127.0.0.1:7801".to_owned()),
        executable: env.home().join("copy"),
        files: vec![env.home().join("unit.socket")],
    };

    let before = files.service().unwrap();
    files.record_service(&service).unwrap();
    let recorded = files.service().unwrap();
    files.forget_service().unwrap();

    assert_eq!(
        (before, recorded, files.service().unwrap()),
        (None, Some(service), None)
    );
}
