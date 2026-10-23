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

use goodfetch_test_support::TestEnv;

/// The test-only copy of util-linux's `unshare` that `ci/os-services.sh
/// setup` installs, root-owned, with an `AppArmor` profile that gives it user
/// namespaces (TESTING.md, Storage, network and OS services).
const TEST_UNSHARE: &str = "/usr/local/libexec/goodfetch-test-tools/unshare";

/// Mounts the tmpfs over the folder `$1` of `$2` bytes, its files copied in
/// first, then runs the rest of the arguments.
const MOUNT: &str = "set -e; folder=\"$1\"; size=\"$2\"; shift 2; \
    cp -a \"$folder\" \"$folder.seed\"; \
    mount -t tmpfs -o \"size=$size\" tmpfs \"$folder\"; \
    cp -a \"$folder.seed/.\" \"$folder\"; rm -rf \"$folder.seed\"; \
    exec \"$@\"";

/// A command that runs `program` with a tmpfs of `size` bytes over
/// `folder`, which holds what `folder` held, in a user and mount namespace
/// of its own. It fails the test at once, naming the restriction, when
/// that `unshare` cannot map the test's user to root.
pub(crate) fn mounted(env: &TestEnv, folder: &Path, size: u64, program: &str) -> Command {
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
    std::fs::create_dir_all(folder).unwrap();
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
        .arg(program);
    command
}

/// `path` as the process `pid` started by [`mounted`] sees it, inside its
/// mount namespace.
pub(crate) fn inside(pid: u32, path: &Path) -> PathBuf {
    let relative = path.strip_prefix("/").unwrap_or(path);
    Path::new("/proc")
        .join(pid.to_string())
        .join("root")
        .join(relative)
}

/// Fills the filesystem of `folder`, as `pid` sees it, with a file of
/// zeros until no byte is left, keeping `room` bytes free; answers the
/// file, which [`free`] removes.
pub(crate) fn fill(pid: u32, folder: &Path, room: u64) -> PathBuf {
    let filler = inside(pid, &folder.join("filler"));
    let mut file = std::fs::File::create(&filler).unwrap();
    let block = vec![0_u8; 64 * 1024];
    while file.write_all(&block).is_ok() {}
    let small = [0_u8; 512];
    while file.write_all(&small).is_ok() {}
    let full = file.metadata().unwrap().len();
    file.set_len(full.saturating_sub(room)).unwrap();
    file.sync_all().unwrap();
    filler
}

/// Frees what [`fill`] took.
pub(crate) fn free(filler: &Path) {
    std::fs::remove_file(filler).unwrap();
}
