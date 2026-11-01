//! `LazyFs` (ADR 0060, Linux): a folder mounted through the pinned LazyFS
//! build keeps a synced write through `clear_cache` and loses an unsynced
//! one, as a power cut would, and a mount that cannot be made fails the test
//! with LazyFS's own output. Only the Release suite runs these tests, on
//! Linux x64 (`just lazyfs-test`, which builds LazyFS and names it in
//! `GOODFETCH_LAZYFS`); nextest's profiles leave them out elsewhere.

use std::ffi::OsString;
use std::io::Write as _;
use std::os::unix::ffi::OsStringExt as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use goodfetch_test_support::{LAZYFS_DEADLINE, LazyFs, LazyFsError, TestEnv};

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
    let env = TestEnv::for_lazyfs();
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
    let env = TestEnv::for_lazyfs();
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
    let env = TestEnv::for_lazyfs();
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
    let env = TestEnv::for_lazyfs();
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

/// The test's own writes of the user configuration, such as the `log.level`
/// each engine start sets, are synced: a power cut after them leaves the
/// file as written, never empty.
#[test]
fn the_tests_user_configuration_written_on_a_mounted_home_survives_clear_cache() {
    let env = TestEnv::for_lazyfs();
    let file = env.home().join("0.0").join("configuration.toml");
    let mounted = LazyFs::mount(&env, env.home()).unwrap();
    env.log_level("debug");
    env.user_configuration("run_state.durability = \"fast\"\n");
    env.log_level("trace");
    let written = std::fs::read_to_string(&file).unwrap();

    mounted.clear_cache().unwrap();
    drop(mounted);
    let _remounted = LazyFs::mount(&env, env.home()).unwrap();

    assert_eq!(
        (
            std::fs::read_to_string(&file).unwrap(),
            written.contains("version = \"0.1\"") && written.contains("log.level = \"trace\""),
        ),
        (written.clone(), true)
    );
}

#[test]
fn a_mount_that_cannot_be_made_fails_with_lazyfs_output_and_leaves_the_folder_as_it_was() {
    let env = TestEnv::for_lazyfs();
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

/// A mount the operating system refuses, as Ubuntu's AppArmor profile of
/// `fusermount3` does outside the folders it allows, fails naming that
/// cause and where mounts are allowed, not only with LazyFS's output, in
/// the text a test's `unwrap()` prints. A script stands in for LazyFS: it
/// prints what `fusermount3` prints then and exits as LazyFS does.
#[test]
fn a_mount_the_os_refuses_fails_naming_the_refusal_and_where_mounts_are_allowed() {
    let env = TestEnv::for_lazyfs();
    let program = env.folder().join("refused-lazyfs");
    write(
        &program,
        "#!/bin/sh\necho 'fusermount3: mount failed: Permission denied' >&2\nexit 4\n",
        true,
    );
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();

    let error = LazyFs::mount_program(&env, &program, env.home()).unwrap_err();

    let message = format!("{error:?}");
    assert_eq!(
        (
            matches!(error, LazyFsError::MountRefused { .. }),
            message.contains("AppArmor"),
            message.contains("GOODFETCH_LAZYFS_FOLDER"),
            message.contains("fusermount3: mount failed: Permission denied"),
        ),
        (true, true, true, true),
        "{message}"
    );
}

#[test]
fn a_mount_of_a_folder_whose_parent_is_missing_fails() {
    let env = TestEnv::for_lazyfs();

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
    let env = TestEnv::for_lazyfs();
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

/// A process the test started that still runs in the folder at the end,
/// as an engine not yet stopped would, with its working folder and a file
/// open there, fails the end at once, named in the error: the end detaches
/// a mount only when no process uses the folder.
#[test]
fn a_mount_that_a_running_process_still_uses_at_its_end_fails_naming_it() {
    let env = TestEnv::for_lazyfs();
    let mounted = LazyFs::mount(&env, env.home()).unwrap();
    write(&env.home().join("open"), "kept open", true);
    let user = env
        .spawn(
            env.process_command("/bin/sh")
                .args(["-c", "exec 3<open; exec /bin/sleep 600"])
                .current_dir(env.home()),
        )
        .unwrap();
    let pid = user.child().id();
    wait_until("the process to hold the file open", || {
        std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|name| name == "sleep\n")
    });

    let ended = mounted.unmount();

    user.stop().unwrap();
    let Err(LazyFsError::NotUnmounted { users, .. }) = &ended else {
        panic!("{ended:?}");
    };
    assert_eq!(
        (
            users.len(),
            users[0].starts_with(&format!("{pid} sleep: cwd, open "))
        ),
        (1, true),
        "{users:?}"
    );
    assert!(!is_mounted(env.home()));
}

/// A lookup that only passes through the folder, such as `lsof` or `df`
/// stat'ing every mount point on the machine, holds the mount while it
/// lasts without using the folder, and the kernel refuses an unmount then:
/// the end waits for it to be done rather than fail. A reader opening a
/// FIFO in the folder stands in for it: until a writer comes, its open
/// holds the mount with no open file, working folder or mapping in it.
#[test]
fn a_mount_that_a_passing_lookup_holds_at_its_end_ends_once_the_lookup_is_done() {
    let env = TestEnv::for_lazyfs();
    let fifo = env.home().join("pipe");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let mounted = LazyFs::mount(&env, env.home()).unwrap();
    let lookup = env
        .spawn(env.process_command("/bin/cat").arg(&fifo))
        .unwrap();
    let pid = lookup.child().id();
    wait_until("the reader to wait for a writer", || {
        std::fs::read_to_string(format!("/proc/{pid}/wchan"))
            .is_ok_and(|wchan| wchan == "wait_for_partner")
    });
    let home = env.home().to_owned();
    // The lookup is done once the end has let go of the folder.
    let done = std::thread::spawn(move || {
        wait_until("the folder to be unmounted", || !is_mounted(&home));
        Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .unwrap();
    });

    let ended = mounted.unmount();

    done.join().unwrap();
    let _ = lookup.stop();
    assert!(ended.is_ok(), "{ended:?}");
}

/// Waits until `condition` holds, failing the test past [`LAZYFS_DEADLINE`].
fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + LAZYFS_DEADLINE;
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "waited {LAZYFS_DEADLINE:?} for {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Whether `path` is a mount point in this process's mount table, read
/// without looking up the path, which would hold the mount itself.
fn is_mounted(path: &Path) -> bool {
    std::fs::read_to_string("/proc/self/mountinfo")
        .unwrap()
        .lines()
        .filter_map(|line| line.split(' ').nth(4))
        .any(|point| unescape(point) == path)
}

/// A mount table's path, whose space, tab, newline and backslash are
/// octal escapes such as `\040`.
fn unescape(field: &str) -> PathBuf {
    let bytes = field.as_bytes();
    let mut path = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        let octal = bytes
            .get(at + 1..at + 4)
            .filter(|digits| byte == b'\\' && digits.iter().all(u8::is_ascii_digit));
        if let Some(digits) = octal {
            path.push(
                digits
                    .iter()
                    .fold(0, |value, digit| value * 8 + (digit - b'0')),
            );
            at += 4;
        } else {
            path.push(byte);
            at += 1;
        }
    }
    PathBuf::from(OsString::from_vec(path))
}
