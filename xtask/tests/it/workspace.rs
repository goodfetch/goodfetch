//! A fixture workspace in its own temporary folder, and `xtask` run on it.
//! Its per-test folder and cleared-environment commands stay here until the
//! Test infrastructure spec fills `goodfetch-test-support` (TESTING.md,
//! Shared helpers), whose interface that spec designs.

use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) struct Workspace {
    folder: tempfile::TempDir,
    /// The Cargo target folder this workspace builds into.
    target: TargetSlot,
}

/// One of the numbered target folders under `fixture-target/`, held through
/// a lock on its lock file for as long as the workspace lives. Copies of one
/// fixture crate share their artifacts' names, so two workspaces never build
/// into one folder at once; a folder keeps its dependencies built for the
/// next test that takes it.
///
/// A workspace takes its slot before it writes a file. Cargo calls a crate
/// fresh when its sources are older than its last build, so sources written
/// before an earlier holder's build would read that holder's test binary.
struct TargetSlot {
    folder: PathBuf,
    _lock: std::fs::File,
}

impl TargetSlot {
    /// The first slot no live workspace holds, in any test process.
    fn take() -> Self {
        let slots = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fixture-target");
        std::fs::create_dir_all(&slots).unwrap();
        for slot in 0.. {
            let lock = std::fs::File::options()
                .create(true)
                .truncate(false)
                .write(true)
                .open(slots.join(format!("{slot}.lock")))
                .unwrap();
            match lock.try_lock() {
                Ok(()) => {
                    return Self {
                        folder: slots.join(slot.to_string()),
                        _lock: lock,
                    };
                }
                Err(std::fs::TryLockError::WouldBlock) => {}
                Err(std::fs::TryLockError::Error(error)) => panic!("locking slot {slot}: {error}"),
            }
        }
        unreachable!("a slot is free once the slots outnumber the live workspaces")
    }
}

/// One problem a check reported: its kind and what it is about, read from an
/// `error[<kind>] <subject>: <message>` line. The message is never compared.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    pub(crate) kind: String,
    pub(crate) subject: String,
}

pub(crate) struct Outcome {
    pub(crate) code: i32,
    pub(crate) stdout: String,
    /// What went wrong when the command could not reach a verdict; shown
    /// when an assertion on the exit code fails.
    pub(crate) stderr: String,
}

impl Outcome {
    pub(crate) fn findings(&self) -> Vec<Finding> {
        reported(&self.stdout, "error[")
    }

    /// The `note[<kind>] <subject>: <message>` lines a check printed beside
    /// its findings.
    pub(crate) fn notes(&self) -> Vec<Finding> {
        reported(&self.stdout, "note[")
    }

    /// The findings of a command whose stdout is its result, such as `xtask
    /// stress select`'s selection: it writes them on stderr.
    pub(crate) fn stderr_findings(&self) -> Vec<Finding> {
        reported(&self.stderr, "error[")
    }

    /// The notes such a command writes on stderr beside its findings.
    pub(crate) fn stderr_notes(&self) -> Vec<Finding> {
        reported(&self.stderr, "note[")
    }
}

fn reported(output: &str, prefix: &str) -> Vec<Finding> {
    output
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix(prefix)?;
            let (kind, rest) = rest.split_once("] ")?;
            let (subject, _message) = rest.split_once(": ")?;
            Some(Finding {
                kind: kind.to_owned(),
                subject: subject.to_owned(),
            })
        })
        .collect()
}

pub(crate) fn finding(kind: &str, subject: &str) -> Finding {
    Finding {
        kind: kind.to_owned(),
        subject: subject.to_owned(),
    }
}

/// Writes `contents` to `path` as an executable, as a stand-in on `PATH` is.
#[cfg(unix)]
pub(crate) fn write_executable(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, contents).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A process a test started in the background, the leader of a process group
/// of its own. However the test ends, passed or failed, the group is told to
/// stop (TERM) and killed if its leader has not ended within 5 seconds, so
/// nothing it started outlives the test.
#[cfg(unix)]
pub(crate) struct Started(std::process::Child);

#[cfg(unix)]
impl Started {
    pub(crate) fn spawn(command: &mut Command) -> Self {
        use std::os::unix::process::CommandExt as _;
        Self(command.process_group(0).spawn().unwrap())
    }

    pub(crate) fn id(&self) -> u32 {
        self.0.id()
    }

    pub(crate) fn wait(&mut self) -> std::process::ExitStatus {
        self.0.wait().unwrap()
    }
}

#[cfg(unix)]
impl Drop for Started {
    fn drop(&mut self) {
        let group = format!("-{}", self.0.id());
        let signal = |name: &str| {
            // A group already gone has nothing left to signal.
            let _ = Command::new("kill")
                .args([name, "--", &group])
                .stderr(std::process::Stdio::null())
                .status();
        };
        signal("-TERM");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while matches!(self.0.try_wait(), Ok(None)) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        signal("-KILL");
        let _ = self.0.wait();
    }
}

/// What `cargo metadata` needs to run under rustup with the pinned toolchain.
const TOOLCHAIN_VARIABLES: [&str; 6] = [
    "PATH",
    "HOME",
    "CARGO",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
];

/// Where an xtask built for coverage writes its profile under `just
/// coverage` (ADR 0072): passed on, so the profile reaches cargo-llvm-cov's
/// report rather than landing in the fixture workspace as a new file.
const COVERAGE_VARIABLE: &str = "LLVM_PROFILE_FILE";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/fixtures")
}

/// Copies a tree, each file written anew rather than cloned (as macOS clones
/// with `std::fs::copy`), so its modification time is the copy's and Cargo
/// rebuilds a fixture crate whose artifacts another copy left.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::write(&target, std::fs::read(entry.path()).unwrap()).unwrap();
        }
    }
}

impl Workspace {
    /// Copies `fixtures/<name>/` into a new temporary folder.
    pub(crate) fn from_fixture(name: &str) -> Self {
        let target = TargetSlot::take();
        let folder = tempfile::tempdir().unwrap();
        copy_tree(&fixtures().join(name), folder.path());
        Self { folder, target }
    }

    pub(crate) fn path(&self) -> &Path {
        self.folder.path()
    }

    /// Copies one fixture file to `relative` inside the workspace.
    pub(crate) fn add_fixture_file(&self, fixture: &str, relative: &str) {
        let target = self.path().join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::copy(fixtures().join(fixture), target).unwrap();
    }

    /// Links the repository's installed npm packages and Python environment
    /// into the workspace, so its runners run there as they do at the root.
    pub(crate) fn link_installed_runners(&self) {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        for folder in ["node_modules", ".venv"] {
            let target = repository.join(folder);
            assert!(
                target.is_dir(),
                "{} is missing: run `pnpm install` and `uv sync`",
                target.display()
            );
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, self.path().join(folder)).unwrap();
            #[cfg(windows)]
            std::os::windows::fs::symlink_dir(&target, self.path().join(folder)).unwrap();
        }
    }

    /// Installs the workspace's npm packages as a pnpm workspace and its
    /// Python environment as a uv workspace, offline. Their runners are the
    /// repository's installed ones: Vitest and Playwright through the
    /// `.toolchain` link, pytest through `.pytoolchain`'s `.pth` file, on
    /// the repository's interpreter that `.python-version` names. All three
    /// and the lockfiles are committed with the workspace, so a checkout of
    /// it installs the same.
    pub(crate) fn install_runners(&self) {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let node_modules = repository.join("node_modules");
        assert!(
            node_modules.is_dir(),
            "{} is missing: run `pnpm install`",
            node_modules.display()
        );
        let venv = repository.join(".venv");
        assert!(
            venv.is_dir(),
            "{} is missing: run `uv sync`",
            venv.display()
        );
        #[cfg(unix)]
        std::os::unix::fs::symlink(&node_modules, self.path().join(".toolchain")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&node_modules, self.path().join(".toolchain")).unwrap();
        let output = Command::new("pnpm")
            .args(["install", "--offline"])
            .current_dir(self.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "pnpm install failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // The `.pth` line and the interpreter, one per line. `addsitedir`
        // reads the repository's own `.pth` files too, so the pytest plugins
        // it installs load.
        let output = Command::new(venv.join("bin/python"))
            .args([
                "-c",
                "import os, sys, sysconfig\n\
                 print(f\"import site; site.addsitedir({sysconfig.get_path('purelib')!r})\")\n\
                 print(os.path.realpath(sys.executable))",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let output = String::from_utf8(output.stdout).unwrap();
        let (pth, interpreter) = output.trim().split_once('\n').unwrap();
        self.write(
            ".pytoolchain/site/repository.pth",
            format!("{pth}\n").as_bytes(),
        );
        self.write(".python-version", format!("{interpreter}\n").as_bytes());
        for args in [
            &["lock", "--offline"][..],
            &["sync", "--frozen", "--offline"],
        ] {
            let output = self.command("uv").args(args).output().unwrap();
            assert!(
                output.status.success(),
                "uv {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    pub(crate) fn remove(&self, relative: &str) {
        std::fs::remove_file(self.path().join(relative)).unwrap();
    }

    /// Removes a folder of the workspace and everything in it.
    pub(crate) fn remove_folder(&self, relative: &str) {
        std::fs::remove_dir_all(self.path().join(relative)).unwrap();
    }

    /// A text file of the workspace, which must exist.
    pub(crate) fn read_text(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path().join(relative)).unwrap()
    }

    /// Replaces the one occurrence of `from` in a file with `to`.
    pub(crate) fn edit(&self, relative: &str, from: &str, to: &str) {
        let contents = self.read_text(relative);
        assert_eq!(contents.matches(from).count(), 1, "{from:?} in {relative}");
        self.write(relative, contents.replace(from, to).as_bytes());
    }

    pub(crate) fn write(&self, relative: &str, contents: &[u8]) {
        let target = self.path().join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, contents).unwrap();
    }

    /// Appends `text`, after a line break, to a file of the workspace.
    pub(crate) fn append(&self, relative: &str, text: &str) {
        let target = self.path().join(relative);
        let mut contents = std::fs::read_to_string(&target).unwrap();
        contents.push('\n');
        contents.push_str(text);
        std::fs::write(target, contents).unwrap();
    }

    pub(crate) fn read(&self, relative: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path().join(relative)).ok()
    }

    /// Writes a file of exactly `bytes` bytes without writing its contents
    /// (sparse where the file system allows), for sizes far over the fixture
    /// limit; only its length is ever read.
    pub(crate) fn write_sized(&self, relative: &str, bytes: u64) {
        let target = self.path().join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::File::create(target)
            .unwrap()
            .set_len(bytes)
            .unwrap();
    }

    /// Commits the workspace in its own Git repository and returns the
    /// commit's tree.
    pub(crate) fn commit(&self) -> String {
        self.commit_workspace("Fixture");
        self.git(&["rev-parse", "HEAD^{tree}"]).trim().to_owned()
    }

    /// Commits the workspace in its own Git repository and returns the new
    /// head commit.
    pub(crate) fn commit_all(&self) -> String {
        self.commit_all_with("Fixture")
    }

    /// Commits the workspace with `message`, whose trailers a check may read,
    /// and returns the new head commit.
    pub(crate) fn commit_all_with(&self, message: &str) -> String {
        self.commit_workspace(message);
        self.git(&["rev-parse", "HEAD"]).trim().to_owned()
    }

    /// Makes the repository on the first call (`git init` again is harmless),
    /// then commits everything in the workspace.
    fn commit_workspace(&self, message: &str) {
        self.git(&["init", "--quiet"]);
        self.git(&["add", "--all"]);
        self.git(&[
            "commit",
            "--quiet",
            "--no-verify",
            "--allow-empty",
            "--message",
            message,
        ]);
    }

    /// Runs `git <args>` in the workspace, with no user or system
    /// configuration and a fixed author, and returns its output; panics if
    /// it fails.
    pub(crate) fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(self.path())
            .env_clear()
            .envs(std::env::var_os("PATH").map(|path| ("PATH", path)))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?} failed");
        String::from_utf8(output.stdout).unwrap()
    }

    /// A command run in the workspace with a cleared environment, but for the
    /// variables that find and run the pinned Cargo toolchain and the
    /// coverage profile's.
    pub(crate) fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let toolchain = TOOLCHAIN_VARIABLES
            .iter()
            .chain([&COVERAGE_VARIABLE])
            .filter_map(|name| std::env::var_os(name).map(|value| (*name, value)));
        let mut command = Command::new(program);
        command.current_dir(self.path()).env_clear().envs(toolchain);
        command
    }

    /// `just`, run in the workspace on the repository's justfile, with the
    /// programs in `shims` found first on the path: stand-ins for those the
    /// recipe under test calls.
    #[cfg(unix)]
    pub(crate) fn just(&self, shims: &Path) -> Command {
        let path = std::env::join_paths(
            std::iter::once(shims.to_owned())
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let mut command = self.command("just");
        command
            .arg("--justfile")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../justfile"))
            .arg("--working-directory")
            .arg(self.path())
            .env("PATH", path);
        command
    }

    /// Runs `xtask <args>` in the workspace. Cargo builds offline, into a
    /// target folder the workspace holds alone while it lives and later
    /// workspaces reuse, so a fixture's dependencies build once per folder
    /// and the tests that build a fixture crate can run at once
    /// (`.config/nextest.toml`, the `fixture-builds` group).
    pub(crate) fn xtask(&self, args: &[&str]) -> Outcome {
        self.xtask_with_env(args, &[])
    }

    /// Runs `cargo <args>` in the workspace, offline and into the target
    /// folder its `xtask` runs build into.
    pub(crate) fn cargo(&self, args: &[&str]) -> Outcome {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let output = self
            .command(cargo)
            .args(args)
            .env("CARGO_TARGET_DIR", &self.target.folder)
            .env("CARGO_NET_OFFLINE", "true")
            .output()
            .unwrap();
        Outcome {
            code: output.status.code().unwrap(),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    }

    /// Runs `xtask <args>` as `xtask` does, with these variables added.
    pub(crate) fn xtask_with_env(
        &self,
        args: &[&str],
        variables: &[(&str, &std::ffi::OsStr)],
    ) -> Outcome {
        let output = self
            .command(env!("CARGO_BIN_EXE_xtask"))
            .args(args)
            .envs(variables.iter().copied())
            .env("CARGO_TARGET_DIR", &self.target.folder)
            .env("CARGO_NET_OFFLINE", "true")
            .output()
            .unwrap();
        Outcome {
            code: output.status.code().unwrap(),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    }
}
