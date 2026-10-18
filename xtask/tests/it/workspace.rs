//! A fixture workspace in its own temporary folder, and `xtask` run on it.

use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) struct Workspace {
    folder: tempfile::TempDir,
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
}

impl Outcome {
    pub(crate) fn findings(&self) -> Vec<Finding> {
        self.stdout
            .lines()
            .filter_map(|line| {
                let rest = line.strip_prefix("error[")?;
                let (kind, rest) = rest.split_once("] ")?;
                let (subject, _message) = rest.split_once(": ")?;
                Some(Finding {
                    kind: kind.to_owned(),
                    subject: subject.to_owned(),
                })
            })
            .collect()
    }
}

pub(crate) fn finding(kind: &str, subject: &str) -> Finding {
    Finding {
        kind: kind.to_owned(),
        subject: subject.to_owned(),
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

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/fixtures")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

impl Workspace {
    /// Copies `fixtures/<name>/` into a new temporary folder.
    pub(crate) fn from_fixture(name: &str) -> Self {
        let folder = tempfile::tempdir().unwrap();
        copy_tree(&fixtures().join(name), folder.path());
        Self { folder }
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

    pub(crate) fn remove(&self, relative: &str) {
        std::fs::remove_file(self.path().join(relative)).unwrap();
    }

    pub(crate) fn write(&self, relative: &str, contents: &[u8]) {
        let target = self.path().join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
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
        self.commit_workspace();
        self.git(&["rev-parse", "HEAD^{tree}"]).trim().to_owned()
    }

    /// Commits the workspace in its own Git repository and returns the new
    /// head commit.
    pub(crate) fn commit_all(&self) -> String {
        self.commit_workspace();
        self.git(&["rev-parse", "HEAD"]).trim().to_owned()
    }

    /// Makes the repository on the first call (`git init` again is harmless),
    /// then commits everything in the workspace.
    fn commit_workspace(&self) {
        self.git(&["init", "--quiet"]);
        self.git(&["add", "--all"]);
        self.git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--no-verify",
            "--allow-empty",
            "--message",
            "Fixture",
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
    /// variables that find and run the pinned Cargo toolchain.
    pub(crate) fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let toolchain = TOOLCHAIN_VARIABLES
            .iter()
            .filter_map(|name| std::env::var_os(name).map(|value| (*name, value)));
        let mut command = Command::new(program);
        command.current_dir(self.path()).env_clear().envs(toolchain);
        command
    }

    /// Runs `xtask <args>` in the workspace.
    pub(crate) fn xtask(&self, args: &[&str]) -> Outcome {
        let output = self
            .command(env!("CARGO_BIN_EXE_xtask"))
            .args(args)
            .output()
            .unwrap();
        Outcome {
            code: output.status.code().unwrap(),
            stdout: String::from_utf8(output.stdout).unwrap(),
        }
    }
}
