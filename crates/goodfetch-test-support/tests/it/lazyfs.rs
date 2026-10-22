//! `LazyFs` (ADR 0060, Linux): a folder mounted through the pinned LazyFS
//! build keeps a synced write through `clear_cache` and loses an unsynced
//! one, as a power cut would, and a mount that cannot be made fails the test
//! with LazyFS's own output. Only the Release suite runs these tests, on
//! Linux x64 (`just lazyfs-test`, which builds LazyFS and names it in
//! `GOODFETCH_LAZYFS`); nextest's profiles leave them out elsewhere.

use std::io::Write as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;

use goodfetch_test_support::{LazyFs, LazyFsError, TestEnv};

/// Writes `text` into a new file at `path`, synced when `synced`.
fn write(path: &Path, text: &str, synced: bool) {
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(text.as_bytes()).unwrap();
    if synced {
        file.sync_all().unwrap();
    }
}

#[test]
fn an_fsynced_write_survives_clear_cache() {
    let env = TestEnv::new();
    let mounted = LazyFs::mount(&env, env.home()).unwrap();
    write(&env.home().join("synced"), "kept", true);

    mounted.clear_cache().unwrap();

    assert_eq!(
        std::fs::read_to_string(env.home().join("synced")).unwrap(),
        "kept"
    );
}

#[test]
fn an_unsynced_write_is_gone_after_clear_cache() {
    let env = TestEnv::new();
    let mounted = LazyFs::mount(&env, env.home()).unwrap();
    write(&env.home().join("unsynced"), "lost", false);

    mounted.clear_cache().unwrap();

    assert_eq!(
        std::fs::read_to_string(env.home().join("unsynced")).unwrap(),
        ""
    );
}

#[test]
fn a_folder_keeps_its_files_through_a_mount_and_after_it_ends() {
    let env = TestEnv::new();
    write(&env.home().join("before"), "before the mount", false);
    let mounted = LazyFs::mount(&env, env.home()).unwrap();
    let seen = std::fs::read_to_string(env.home().join("before")).unwrap();
    write(&env.home().join("during"), "written, never synced", false);

    drop(mounted);

    assert_eq!(
        (
            seen.as_str(),
            std::fs::read_to_string(env.home().join("during"))
                .unwrap()
                .as_str()
        ),
        ("before the mount", "written, never synced")
    );
}

#[test]
fn a_remount_sees_what_was_synced_before_the_cache_was_cleared() {
    let env = TestEnv::new();
    let mounted = LazyFs::mount(&env, env.home()).unwrap();
    write(&env.home().join("synced"), "kept", true);
    write(&env.home().join("unsynced"), "lost", false);
    mounted.clear_cache().unwrap();
    drop(mounted);

    let _remounted = LazyFs::mount(&env, env.home()).unwrap();

    assert_eq!(
        (
            std::fs::read_to_string(env.home().join("synced")).unwrap(),
            std::fs::read_to_string(env.home().join("unsynced")).unwrap(),
        ),
        ("kept".to_owned(), String::new())
    );
}

#[test]
fn a_mount_that_cannot_be_made_fails_with_lazyfs_output_and_leaves_the_folder_as_it_was() {
    let env = TestEnv::new();
    write(&env.home().join("before"), "untouched", false);

    let error = LazyFs::mount_program(&env, Path::new("/bin/false"), env.home()).unwrap_err();

    assert_eq!(
        (
            matches!(error, LazyFsError::NotMounted { .. }),
            std::fs::read_to_string(env.home().join("before")).unwrap(),
        ),
        (true, "untouched".to_owned()),
        "{error}"
    );
}

#[test]
fn a_mount_of_a_folder_whose_parent_is_missing_fails() {
    let env = TestEnv::new();

    let error = LazyFs::mount(&env, env.folder().join("missing").join("folder")).unwrap_err();

    assert!(matches!(error, LazyFsError::Folder { .. }), "{error}");
}

/// Whether `path` is a mount point: its device is not its parent's.
fn is_mount_point(path: &Path) -> bool {
    let device = |path: &Path| std::fs::metadata(path).unwrap().dev();
    device(path) != device(path.parent().unwrap())
}

/// A mount whose end fails, here because a file stays open in it, still
/// stops LazyFS and leaves the folder unmounted with its files back, so a
/// failed end never leaves a mount or a process behind.
#[test]
fn a_mount_whose_end_fails_still_stops_lazyfs_and_unmounts() {
    let env = TestEnv::new();
    let mounted = LazyFs::mount(&env, env.home()).unwrap();
    write(&env.home().join("open"), "kept open", true);
    let busy = std::fs::File::open(env.home().join("open")).unwrap();

    let ended = mounted.unmount();

    drop(busy);
    assert!(
        matches!(&ended, Err(LazyFsError::NotUnmounted { .. })),
        "{ended:?}"
    );
    assert!(!is_mount_point(env.home()));
    assert_eq!(
        std::fs::read_to_string(env.home().join("open")).unwrap(),
        "kept open"
    );
}
