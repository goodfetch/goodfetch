//! A size-capped filesystem for one test (TESTING.md, the Storage
//! conformance cases and the Daemon log cases; ADR 0122): a tmpfs mounted
//! over a folder of the test's, in a user and mount namespace of the
//! process the test starts there, which `ci/os-services.sh setup`'s
//! `unshare` allows unprivileged. The test's own process stays outside: it
//! reaches the filesystem through the started process's `/proc/<pid>/root`,
//! and the tmpfs goes when that process exits, never filling the machine's
//! own disk.
#![expect(
    clippy::disallowed_methods,
    reason = "a test fills and frees the filesystem it mounted, outside any async worker"
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::env::TestEnv;

/// The test-only copy of util-linux's `unshare` that `ci/os-services.sh
/// setup` installs, root-owned, with an `AppArmor` profile that gives it user
/// namespaces (TESTING.md, Storage, network and OS services).
const TEST_UNSHARE: &str = "/usr/local/libexec/goodfetch-test-tools/unshare";

/// Mounts the tmpfs over the folder `$1` of `$2` bytes, its files copied in
/// first, keeping its subfolder `$3`, unless empty, on the folder's own
/// filesystem through a bind mount moved back over it, then runs the rest of
/// the arguments.
const MOUNT: &str = "set -e; folder=\"$1\"; size=\"$2\"; kept=\"$3\"; shift 3; \
    if [ -n \"$kept\" ]; then mkdir -p \"$folder/$kept\" \"$folder.kept\"; \
    mount --bind \"$folder/$kept\" \"$folder.kept\"; fi; \
    cp -a \"$folder\" \"$folder.seed\"; \
    mount -t tmpfs -o \"size=$size\" tmpfs \"$folder\"; \
    cp -a \"$folder.seed/.\" \"$folder\"; rm -rf \"$folder.seed\"; \
    if [ -n \"$kept\" ]; then mkdir -p \"$folder/$kept\"; \
    mount --move \"$folder.kept\" \"$folder/$kept\"; rmdir \"$folder.kept\"; fi; \
    exec \"$@\"";

/// The Goodfetch home's folder of the engine's JSON log, which the test's
/// teardown copies into its exercise record (TESTING.md, Feature coverage).
const LOG_FOLDER: &str = "logs";

/// A command that runs `program` with a tmpfs of `size` bytes over
/// `folder`, which holds what `folder` held, in a user and mount namespace
/// of its own.
///
/// # Panics
///
/// At once, naming the restriction, when that `unshare` cannot map the
/// test's user to root.
pub fn tmpfs_mounted(env: &TestEnv, folder: &Path, size: u64, program: &str) -> Command {
    mounted_keeping(env, folder, size, program, "")
}

/// A command that runs `program` with a tmpfs of `size` bytes over the
/// test's Goodfetch home, as [`tmpfs_mounted`] does, but its `logs` folder
/// kept on the test's own disk: the engine's JSON log outlives the tmpfs
/// and is never refused by the full filesystem, so the test's exercise
/// record shows what the engine dispatched (ADR 0072).
///
/// # Panics
///
/// As [`tmpfs_mounted`].
pub fn tmpfs_mounted_home(env: &TestEnv, size: u64, program: &str) -> Command {
    mounted_keeping(env, env.home(), size, program, LOG_FOLDER)
}

fn mounted_keeping(env: &TestEnv, folder: &Path, size: u64, program: &str, kept: &str) -> Command {
    let unshare = if Path::new(TEST_UNSHARE).exists() {
        TEST_UNSHARE
    } else {
        "unshare"
    };
    let probe = env
        .process_command(unshare)
        .args(["--user", "--map-root-user", "true"])
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|error| panic!("`{unshare}` did not run: {error}"));
    assert!(
        probe.status.success(),
        "`{unshare}` cannot map the test's user to root in a user namespace of its own ({}): run \
         `ci/os-services.sh setup`, which installs `{TEST_UNSHARE}` with an AppArmor profile that \
         allows it",
        String::from_utf8_lossy(&probe.stderr).trim()
    );
    std::fs::create_dir_all(folder).expect("the folder to mount over is made");
    let mut command = env.process_command(unshare);
    command
        .args([
            "--user",
            "--map-root-user",
            "--mount",
            "sh",
            "-c",
            MOUNT,
            "sh",
        ])
        .arg(folder)
        .arg(size.to_string())
        .arg(kept)
        .arg(program);
    command
}

/// `path` as the process `pid` started by [`tmpfs_mounted`] sees it,
/// inside its mount namespace.
pub fn inside_mount(pid: u32, path: &Path) -> PathBuf {
    let relative = path.strip_prefix("/").unwrap_or(path);
    Path::new("/proc")
        .join(pid.to_string())
        .join("root")
        .join(relative)
}

/// Fills the filesystem of `folder`, as `pid` sees it, with a file of
/// zeros, keeping `room` bytes free, or until no byte is left when `room`
/// is 0; answers the file, which [`free_filesystem`] removes.
///
/// With some `room`, the file never takes more than what is free beyond
/// it: a filesystem full for a moment on the way is one the engine's
/// storage sample or a commit of run state may meet, and either stops the
/// Run as at `critical` (ADR 0122).
///
/// # Panics
///
/// When the file cannot be made or synced.
pub fn fill_filesystem(pid: u32, folder: &Path, room: u64) -> PathBuf {
    let filler = inside_mount(pid, &folder.join("filler"));
    let mut file = std::fs::File::create(&filler).expect("the filler file is made");
    let block = vec![0_u8; 64 * 1024];
    if room == 0 {
        while file.write_all(&block).is_ok() {}
        let small = [0_u8; 512];
        while file.write_all(&small).is_ok() {}
    } else {
        let mut left = available(&filler).saturating_sub(room);
        while left > 0 {
            let size = block.len().min(usize::try_from(left).unwrap_or(usize::MAX));
            if file.write_all(&block[..size]).is_err() {
                break;
            }
            left -= u64::try_from(size).unwrap_or(u64::MAX).min(left);
        }
    }
    file.sync_all().expect("the filler file syncs");
    filler
}

/// The bytes left to an unprivileged user on the filesystem holding
/// `path`, as `stat --file-system` reads them.
fn available(path: &Path) -> u64 {
    let output = Command::new("stat")
        .args(["--file-system", "--format", "%a %S"])
        .arg(path)
        .output()
        .expect("`stat --file-system` runs");
    assert!(
        output.status.success(),
        "`stat --file-system` failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let read = String::from_utf8(output.stdout).expect("`stat` prints UTF-8");
    let (blocks, size) = read
        .trim()
        .split_once(' ')
        .expect("`stat` prints blocks and a block size");
    let number = |text: &str| text.parse::<u64>().expect("`stat` prints numbers");
    number(blocks) * number(size)
}

/// Frees what [`fill_filesystem`] took.
///
/// # Panics
///
/// When the file cannot be removed.
pub fn free_filesystem(filler: &Path) {
    std::fs::remove_file(filler).expect("the filler file is removed");
}
