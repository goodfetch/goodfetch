//! `xtask sweep-target` removes a Cargo target folder's dead units, those
//! whose fingerprint no build has read for `--unused-hours` or since the day
//! before `--read-since`, with every file
//! carrying their hash, and keeps whole each unit a build read, so the next
//! build reuses it (TESTING.md, Commands). The fixture target is written by
//! hand, with its files' access and modification times set back.

use std::fs::{File, FileTimes};
use std::time::{Duration, SystemTime};

use crate::workspace::{Outcome, Workspace};

const DEAD: &str = "0123456789abcdef";
const LIVE: &str = "fedcba9876543210";
const ORPHAN: &str = "00112233aabbccdd";

/// Sets a file's or a folder's access and modification times `hours` back.
fn age(workspace: &Workspace, relative: &str, hours: u64) {
    let when = SystemTime::now() - Duration::from_hours(hours);
    File::open(workspace.path().join(relative))
        .unwrap()
        .set_times(FileTimes::new().set_accessed(when).set_modified(when))
        .unwrap();
}

/// A library unit of `profile`, its fingerprint last read `hours` ago, with
/// its `deps/` files; returns those files' paths.
fn library(
    workspace: &Workspace,
    profile: &str,
    name: &str,
    hash: &str,
    hours: u64,
) -> Vec<String> {
    let fingerprint = format!("{profile}/.fingerprint/{name}-{hash}");
    let files = vec![
        format!("{fingerprint}/lib-{name}"),
        format!("{fingerprint}/lib-{name}.json"),
        format!("{profile}/deps/lib{name}-{hash}.rlib"),
        format!("{profile}/deps/lib{name}-{hash}.rmeta"),
        format!("{profile}/deps/{name}-{hash}.d"),
    ];
    for file in &files {
        workspace.write(file, b"unit");
        age(workspace, file, hours);
    }
    age(workspace, &fingerprint, hours);
    files
}

fn sweep(workspace: &Workspace, args: &[&str]) -> Outcome {
    let mut all = vec!["sweep-target", "--target", "target"];
    all.extend_from_slice(args);
    workspace.xtask(&all)
}

fn exists(workspace: &Workspace, relative: &str) -> bool {
    workspace.path().join(relative).symlink_metadata().is_ok()
}

#[test]
fn a_unit_no_build_read_goes_with_its_files_and_a_unit_a_build_read_stays_whole() {
    let workspace = Workspace::from_fixture("empty");
    let dead = library(&workspace, "target/debug", "old", DEAD, 72);
    let live = library(&workspace, "target/debug", "new", LIVE, 1);
    // A read unit keeps its test executable, build-script output and
    // example, however old the files themselves are.
    let kept = [
        format!("target/debug/deps/new-{LIVE}"),
        format!("target/debug/build/new-{LIVE}/out/generated.rs"),
        format!("target/debug/examples/demo-{LIVE}"),
    ];
    for file in &kept {
        workspace.write(file, b"kept");
        age(&workspace, file, 500);
    }
    // A hashed file whose unit has no fingerprint is never fresh to Cargo;
    // a file without a hash, such as an uplifted executable, is no unit's.
    workspace.write(
        &format!("target/debug/deps/liborphan-{ORPHAN}.rlib"),
        b"orphan",
    );
    workspace.write("target/debug/xtask", b"uplifted");
    // An incremental folder no compile wrote within the hours goes; a fresh one stays.
    workspace.write(
        "target/debug/incremental/old-1abc/s-old/query-cache.bin",
        b"i",
    );
    age(
        &workspace,
        "target/debug/incremental/old-1abc/s-old/query-cache.bin",
        72,
    );
    let stale = workspace.path().join("target/debug/incremental/old-1abc");
    for folder in [stale.join("s-old"), stale.clone()] {
        File::open(&folder)
            .unwrap()
            .set_times(FileTimes::new().set_modified(SystemTime::now() - Duration::from_hours(72)))
            .unwrap();
    }
    workspace.write(
        "target/debug/incremental/new-2def/s-new/query-cache.bin",
        b"i",
    );
    // A target triple's profile is swept the same way.
    let musl = library(
        &workspace,
        "target/x86_64-unknown-linux-musl/debug",
        "old",
        DEAD,
        72,
    );

    let outcome = sweep(&workspace, &[]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    for file in dead.iter().chain(&musl) {
        assert!(!exists(&workspace, file), "{file} was kept");
    }
    assert!(!exists(
        &workspace,
        &format!("target/debug/.fingerprint/old-{DEAD}")
    ));
    assert!(!exists(
        &workspace,
        &format!("target/debug/deps/liborphan-{ORPHAN}.rlib")
    ));
    assert!(!exists(&workspace, "target/debug/incremental/old-1abc"));
    for file in live.iter().chain(&kept) {
        assert!(exists(&workspace, file), "{file} was removed");
    }
    assert!(exists(&workspace, "target/debug/xtask"));
    assert!(exists(&workspace, "target/debug/incremental/new-2def"));
    assert!(
        outcome.stdout.contains("2 units no build read for 48 h")
            && outcome.stdout.contains("kept 1 units"),
        "{}",
        outcome.stdout
    );
}

#[test]
fn unused_hours_set_how_long_unread_a_unit_is_dead() {
    let workspace = Workspace::from_fixture("empty");
    let recent = library(&workspace, "target/debug", "recent", LIVE, 29);
    let older = library(&workspace, "target/debug", "older", DEAD, 31);

    let outcome = sweep(&workspace, &["--unused-hours", "30"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(recent.iter().all(|file| exists(&workspace, file)));
    assert!(older.iter().all(|file| !exists(&workspace, file)));
}

/// `--read-since` keeps every unit read since that Unix time, less the day
/// `relatime` may lag: the gate's sweep passes the gate's start.
#[test]
fn read_since_keeps_what_was_read_since_the_day_before_it() {
    let workspace = Workspace::from_fixture("empty");
    let since = SystemTime::now() - Duration::from_hours(6);
    let recent = library(&workspace, "target/debug", "recent", LIVE, 29);
    let older = library(&workspace, "target/debug", "older", DEAD, 31);
    let seconds = since
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();

    let outcome = sweep(&workspace, &["--read-since", &seconds]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(recent.iter().all(|file| exists(&workspace, file)));
    assert!(older.iter().all(|file| !exists(&workspace, file)));
}

/// A time to come would put the cutoff inside the day `relatime` lags.
#[test]
fn a_read_since_after_now_is_refused() {
    let workspace = Workspace::from_fixture("empty");
    let unit = library(&workspace, "target/debug", "old", DEAD, 72);
    let later = (SystemTime::now() + Duration::from_hours(48))
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();

    let outcome = sweep(&workspace, &["--read-since", &later]);

    assert_ne!(outcome.code, 0, "{}", outcome.stdout);
    assert!(
        outcome
            .stderr
            .contains(&format!("--read-since {later} is after now")),
        "{}",
        outcome.stderr
    );
    assert!(unit.iter().all(|file| exists(&workspace, file)));
}

/// Under `relatime` a read moves the access time only once it is a day old,
/// so a window of a day or less could take a unit read today for dead.
#[test]
fn a_window_of_a_day_or_less_is_refused() {
    let workspace = Workspace::from_fixture("empty");
    let unit = library(&workspace, "target/debug", "old", DEAD, 72);

    let outcome = sweep(&workspace, &["--unused-hours", "24"]);

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
    assert!(
        outcome
            .stderr
            .contains("invalid value '24' for '--unused-hours"),
        "{}",
        outcome.stderr
    );
    assert!(unit.iter().all(|file| exists(&workspace, file)));
}

#[test]
fn a_dry_run_lists_what_would_go_and_removes_nothing() {
    let workspace = Workspace::from_fixture("empty");
    let dead = library(&workspace, "target/debug", "old", DEAD, 72);

    let outcome = sweep(&workspace, &["--dry-run"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(dead.iter().all(|file| exists(&workspace, file)));
    assert!(outcome.stdout.contains("would free"), "{}", outcome.stdout);
}

/// A build running beside the sweep holds Cargo's lock of its profile, which
/// the sweep then leaves whole.
#[test]
fn a_profile_a_cargo_process_has_locked_is_left_whole() {
    let workspace = Workspace::from_fixture("empty");
    let dead = library(&workspace, "target/debug", "old", DEAD, 72);
    workspace.write("target/debug/.cargo-lock", b"");
    let lock = File::options()
        .write(true)
        .open(workspace.path().join("target/debug/.cargo-lock"))
        .unwrap();
    lock.lock().unwrap();

    let outcome = sweep(&workspace, &[]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(dead.iter().all(|file| exists(&workspace, file)));
    assert!(
        outcome
            .stdout
            .contains("left whole, a Cargo process holds its .cargo-lock"),
        "{}",
        outcome.stdout
    );
}

/// The stress run's base build (`just stress`) is never entered, nor is a
/// linked folder followed out of the target folder.
#[cfg(unix)]
#[test]
fn the_stress_base_build_and_linked_folders_are_never_entered() {
    let workspace = Workspace::from_fixture("empty");
    let base = library(&workspace, "target/stress-base/debug", "old", DEAD, 72);
    let elsewhere = library(&workspace, "elsewhere/debug", "old", DEAD, 72);
    std::os::unix::fs::symlink(
        workspace.path().join("elsewhere"),
        workspace.path().join("target/linked"),
    )
    .unwrap();

    let outcome = sweep(&workspace, &[]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(
        base.iter()
            .chain(&elsewhere)
            .all(|file| exists(&workspace, file))
    );
}

/// `--leave` names a folder the sweep never enters, wherever it is under the
/// target folder: the backstop's sweep leaves the weekly constrained
/// test-health run's `ci-test` profile out of its two-day window
/// (`ci/gate-backstop.sh`).
#[test]
fn a_folder_the_sweep_is_told_to_leave_is_never_entered() {
    let workspace = Workspace::from_fixture("empty");
    let left = library(&workspace, "target/ci-test", "old", DEAD, 72);
    let under_a_triple = library(
        &workspace,
        "target/x86_64-unknown-linux-gnu/ci-test",
        "old",
        DEAD,
        72,
    );
    let swept = library(&workspace, "target/debug", "old", DEAD, 72);

    let outcome = sweep(&workspace, &["--leave", "ci-test"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(
        left.iter()
            .chain(&under_a_triple)
            .all(|file| exists(&workspace, file))
    );
    assert!(swept.iter().all(|file| !exists(&workspace, file)));
}

/// A profile folder swept as the target itself takes that sweep's own
/// window, as the backstop gives `ci-test` a longer one.
#[test]
fn a_profile_folder_is_swept_as_the_target_with_a_window_of_its_own() {
    let workspace = Workspace::from_fixture("empty");
    let last_week = library(&workspace, "target/ci-test", "recent", LIVE, 200);
    let long_ago = library(&workspace, "target/ci-test", "older", DEAD, 400);
    let elsewhere = library(&workspace, "target/debug", "older", DEAD, 400);

    let outcome = workspace.xtask(&[
        "sweep-target",
        "--target",
        "target/ci-test",
        "--unused-hours",
        "384",
    ]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(
        last_week
            .iter()
            .chain(&elsewhere)
            .all(|file| exists(&workspace, file))
    );
    assert!(long_ago.iter().all(|file| !exists(&workspace, file)));
}

/// Cargo makes a unit's fingerprint folder before it compiles the unit and
/// writes the fingerprint after: a folder made within the hours, empty or
/// not, is a unit a build reached.
#[test]
fn a_unit_whose_fingerprint_a_build_has_not_written_yet_stays() {
    let workspace = Workspace::from_fixture("empty");
    std::fs::create_dir_all(
        workspace
            .path()
            .join(format!("target/debug/.fingerprint/new-{LIVE}")),
    )
    .unwrap();
    let output = format!("target/debug/deps/libnew-{LIVE}.rmeta");
    workspace.write(&output, b"unit");

    let outcome = sweep(&workspace, &[]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(exists(&workspace, &output));
}
