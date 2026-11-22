//! `ci/clippy-excluded.sh`, `just lint`'s `clippy-excluded` step: Clippy,
//! with `-D warnings`, on each first-party crate the root workspace's
//! `exclude` list holds, which the `clippy` step's `--workspace` never reaches
//! (#2980). The forks under `vendor/` stay out. Each crate is linted from its
//! own folder, so rustup takes its own toolchain pin, as one step of its own,
//! so a red one names the crate; one with a lockfile of its own is linted
//! with it, `--locked`, one without reads a copy of the root lockfile,
//! offline, and none is written into the source tree. On a host other than
//! macOS, the Kennel is also linted for macOS, so its macOS-only code is. On
//! Linux without the development files of GTK 3 and libdbus, the Kennel's
//! graphical crate is left out of the host's lint, and without the macOS
//! target the macOS lint is: each is listed as skipped, and CI fails instead
//! (ADR 0061). The script runs on a fixture workspace through stand-ins for
//! `cargo`, which records each call, `pkg-config` and `rustup`. Repository
//! tooling, so the module declares no Feature.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use crate::workspace::Workspace;

/// The root lockfile's text, which a crate without a lockfile reads.
const ROOT_LOCK: &str = "# the root lockfile\nversion = 4\n";

/// The Clippy arguments every crate takes after its own.
const LINT: &str = "--workspace --all-targets --all-features -- -D warnings";

/// A workspace whose root excludes the Kennel (a lockfile of its own), a
/// library without one, and a fork under `vendor/`, with stand-ins for
/// `cargo` and `pkg-config` in `stand-in/`.
fn excluding() -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "Cargo.toml",
        b"[workspace]\n\
          members = []\n\
          exclude = [\n  \"libs/lockless\",\n  \"kennel\",\n  \"vendor/fork\",\n]\n",
    );
    workspace.write("Cargo.lock", ROOT_LOCK.as_bytes());
    workspace.write("kennel/Cargo.toml", b"[workspace]\n");
    workspace.write("kennel/Cargo.lock", b"# the Kennel's lockfile\n");
    workspace.write("libs/lockless/Cargo.toml", b"[workspace]\n");
    workspace.write("vendor/fork/Cargo.toml", b"[package]\n");
    let folder = workspace.path().join("stand-in");
    let cargo = format!(
        r#"#!/bin/sh
echo "$(pwd -P)|$*" >>"{calls}"
for argument in "$@"; do
  case $argument in
    resolver.lockfile-path=*)
      lock=${{argument#resolver.lockfile-path=\"}}
      cp "${{lock%\"}}" "{read_lock}"
      ;;
  esac
done
if [ -f "{failing}" ] && [ "$(pwd -P)" = "$(cat "{failing}")" ]; then
  echo "warning: a planted warning"
  exit 101
fi
"#,
        calls = folder.join("calls").display(),
        read_lock = folder.join("read-lock").display(),
        failing = folder.join("failing").display(),
    );
    let pkg_config = format!(
        "#!/bin/sh\n[ ! -f \"{}\" ]\n",
        folder.join("no-gtk").display()
    );
    // `rustup target list --installed`: the host's target, and the macOS one
    // unless `no-cross` exists.
    let rustup = format!(
        "#!/bin/sh\necho x86_64-unknown-linux-gnu\n[ -f \"{}\" ] || echo aarch64-apple-darwin\n",
        folder.join("no-cross").display()
    );
    for (name, script) in [
        ("cargo", cargo),
        ("pkg-config", pkg_config),
        ("rustup", rustup),
    ] {
        workspace.write(&format!("stand-in/{name}"), script.as_bytes());
        std::fs::set_permissions(folder.join(name), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    workspace
}

struct Run {
    code: Option<i32>,
    /// Each `cargo` call: the folder it ran in, relative to the workspace,
    /// and its arguments.
    calls: Vec<(String, String)>,
    stdout: String,
    stderr: String,
}

/// Runs the repository's `ci/clippy-excluded.sh` in the workspace, with the
/// stand-ins first on `PATH` and these variables added.
fn clippy_excluded(workspace: &Workspace, variables: &[(&str, &str)]) -> Run {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/clippy-excluded.sh");
    let path = std::env::join_paths(
        std::iter::once(workspace.path().join("stand-in"))
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let output = workspace
        .command("bash")
        .arg(script)
        .env("PATH", path)
        .envs(variables.iter().copied())
        .output()
        .unwrap();
    let root = format!("{}/", root(workspace).display());
    let calls = std::fs::read_to_string(workspace.path().join("stand-in/calls"))
        .unwrap_or_default()
        .lines()
        .map(|line| {
            let (folder, arguments) = line.split_once('|').unwrap();
            (
                folder.strip_prefix(&root).unwrap_or(folder).to_owned(),
                arguments.to_owned(),
            )
        })
        .collect();
    Run {
        code: output.status.code(),
        calls,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn root(workspace: &Workspace) -> PathBuf {
    workspace.path().canonicalize().unwrap()
}

/// The Clippy arguments that lint for macOS from another host.
const CROSS: &str = "--target aarch64-apple-darwin ";

/// The arguments of the one call made in `folder` for this host.
fn call_in<'run>(run: &'run Run, folder: &str) -> &'run str {
    one_call(run, folder, false)
}

/// The arguments of the one call made in `folder` for macOS.
#[cfg(not(target_os = "macos"))]
fn cross_call_in<'run>(run: &'run Run, folder: &str) -> &'run str {
    one_call(run, folder, true)
}

fn one_call<'run>(run: &'run Run, folder: &str, cross: bool) -> &'run str {
    let calls: Vec<&str> = run
        .calls
        .iter()
        .filter(|(called, arguments)| called == folder && arguments.contains(CROSS) == cross)
        .map(|(_, arguments)| arguments.as_str())
        .collect();
    assert_eq!(calls.len(), 1, "{:?}\n{}", run.calls, run.stdout);
    calls[0]
}

/// `just lint`, so `just quick` and `just quick-steps`, runs the script.
#[test]
fn just_lint_runs_the_script() {
    let lint = crate::register::steps("lint");

    assert!(
        lint.iter()
            .any(|command| command == "ci/clippy-excluded.sh"),
        "{lint:#?}"
    );
}

#[test]
fn each_first_party_excluded_crate_is_linted_from_its_own_folder_and_no_vendor_fork_is() {
    let workspace = excluding();

    let run = clippy_excluded(&workspace, &[]);

    assert_eq!(run.code, Some(0), "{}\n{}", run.stdout, run.stderr);
    let mut folders: Vec<&str> = run
        .calls
        .iter()
        .map(|(folder, _)| folder.as_str())
        .collect();
    folders.dedup();
    assert_eq!(folders, ["libs/lockless", "kennel"], "{}", run.stdout);
    let target = root(&workspace).join("target");
    assert_eq!(
        call_in(&run, "kennel"),
        format!("clippy --locked --target-dir {} {LINT}", target.display())
    );
    assert!(
        call_in(&run, "libs/lockless")
            .ends_with(&format!("--target-dir {} {LINT}", target.display())),
        "{:?}",
        run.calls
    );
}

/// The Kennel's macOS-only code is linted from the Linux gate too, as its
/// whole workspace for macOS; no other crate is.
#[cfg(not(target_os = "macos"))]
#[test]
fn off_macos_the_kennel_is_also_linted_for_macos_and_no_other_crate_is() {
    let workspace = excluding();

    let run = clippy_excluded(&workspace, &[]);

    assert_eq!(run.code, Some(0), "{}\n{}", run.stdout, run.stderr);
    let target = root(&workspace).join("target");
    assert_eq!(
        cross_call_in(&run, "kennel"),
        format!(
            "clippy --locked {CROSS}--target-dir {} {LINT}",
            target.display()
        )
    );
    assert!(
        !run.calls
            .iter()
            .any(|(folder, arguments)| folder != "kennel" && arguments.contains(CROSS)),
        "{:?}",
        run.calls
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn without_the_macos_target_its_lint_is_listed_as_skipped_and_the_host_lint_still_runs() {
    let workspace = excluding();
    workspace.write("stand-in/no-cross", b"");

    let run = clippy_excluded(&workspace, &[]);

    assert_eq!(run.code, Some(0), "{}\n{}", run.stdout, run.stderr);
    call_in(&run, "kennel");
    assert!(
        !run.calls
            .iter()
            .any(|(_, arguments)| arguments.contains(CROSS)),
        "{:?}",
        run.calls
    );
    let skipped = workspace.read_text("target/skipped.txt");
    assert!(
        skipped.contains("the Rust target aarch64-apple-darwin is not installed"),
        "{skipped}"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn in_ci_without_the_macos_target_the_kennel_fails() {
    let workspace = excluding();
    workspace.write("stand-in/no-cross", b"");

    let run = clippy_excluded(&workspace, &[("CI", "true")]);

    assert_eq!(run.code, Some(1), "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("failed   clippy-kennel: "),
        "{}",
        run.stdout
    );
    assert!(workspace.read("target/skipped.txt").is_none());
}

#[test]
fn a_warning_in_one_crate_fails_the_run_names_its_crate_and_the_others_are_still_linted() {
    let workspace = excluding();
    workspace.write(
        "stand-in/failing",
        root(&workspace)
            .join("libs/lockless")
            .to_str()
            .unwrap()
            .as_bytes(),
    );

    let run = clippy_excluded(&workspace, &[]);

    assert_eq!(run.code, Some(1), "{}\n{}", run.stdout, run.stderr);
    call_in(&run, "kennel");
    assert!(
        run.stdout.contains("warning: a planted warning"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("failed   clippy-lockless: "),
        "{}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("failed   clippy-kennel"),
        "{}",
        run.stdout
    );
}

#[test]
fn a_crate_without_a_lockfile_reads_a_copy_of_the_root_one_offline_and_none_is_left_in_its_folder()
{
    let workspace = excluding();

    let run = clippy_excluded(&workspace, &[]);

    assert_eq!(run.code, Some(0), "{}\n{}", run.stdout, run.stderr);
    let arguments = call_in(&run, "libs/lockless");
    assert!(
        arguments.starts_with("clippy --offline --config resolver.lockfile-path="),
        "{arguments}"
    );
    assert_eq!(workspace.read_text("stand-in/read-lock"), ROOT_LOCK);
    assert!(workspace.read("libs/lockless/Cargo.lock").is_none());
    assert_eq!(workspace.read_text("Cargo.lock"), ROOT_LOCK);
}

#[cfg(target_os = "linux")]
#[test]
fn on_linux_without_gtk_the_kennel_is_linted_without_its_graphical_crate_which_is_listed_as_skipped()
 {
    let workspace = excluding();
    workspace.write("stand-in/no-gtk", b"");

    let run = clippy_excluded(&workspace, &[]);

    assert_eq!(run.code, Some(0), "{}\n{}", run.stdout, run.stderr);
    assert!(
        call_in(&run, "kennel").starts_with("clippy --locked --exclude goodfetch-kennel-app "),
        "{:?}",
        run.calls
    );
    assert!(
        !cross_call_in(&run, "kennel").contains("--exclude"),
        "{:?}",
        run.calls
    );
    let skipped = workspace.read_text("target/skipped.txt");
    assert!(
        skipped.contains("pkg-config finds no gtk+-3.0 dbus-1:"),
        "{skipped}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn in_ci_without_gtk_the_run_fails_rather_than_leave_the_graphical_crate_out() {
    let workspace = excluding();
    workspace.write("stand-in/no-gtk", b"");

    let run = clippy_excluded(&workspace, &[("CI", "true")]);

    assert_eq!(run.code, Some(1), "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("failed   clippy-kennel: "),
        "{}",
        run.stdout
    );
    assert!(workspace.read("target/skipped.txt").is_none());
}
