//! Records the commit the Proving ground is built from, so its version is the
//! workspace version plus the commit (ADR 0063): `GOODFETCH_COMMIT` when the
//! build sets it, else `git rev-parse HEAD`, else `unknown` with a warning.
#![expect(
    clippy::disallowed_methods,
    reason = "a build script is synchronous and reads its own environment and git"
)]

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-env-changed=GOODFETCH_COMMIT");
    let commit = std::env::var("GOODFETCH_COMMIT")
        .ok()
        .filter(|commit| !commit.is_empty())
        .or_else(|| {
            watch_head();
            git(&["rev-parse", "HEAD"])
        })
        .unwrap_or_else(|| {
            println!("cargo::warning=no git commit found; the version ends in `+unknown`");
            "unknown".to_owned()
        });
    println!("cargo::rustc-env=GOODFETCH_PROVING_GROUND_COMMIT={commit}");
}

/// Reruns the script when `HEAD` moves: its file, the branch it names and the
/// packed references.
fn watch_head() {
    let (Some(git_folder), Some(common_folder)) = (
        git(&["rev-parse", "--absolute-git-dir"]),
        git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]),
    ) else {
        return;
    };
    let common_folder = Path::new(&common_folder);
    let branch = git(&["symbolic-ref", "--quiet", "HEAD"]);
    let watched = [
        Some(Path::new(&git_folder).join("HEAD")),
        Some(common_folder.join("packed-refs")),
        branch.map(|branch| common_folder.join(branch)),
    ];
    // A path that does not exist would rerun the script on every build.
    for path in watched.into_iter().flatten().filter(|path| path.exists()) {
        println!("cargo::rerun-if-changed={}", path.display());
    }
}

fn git(arguments: &[&str]) -> Option<String> {
    let output = Command::new("git").args(arguments).output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (output.status.success() && !text.is_empty()).then(|| text.to_owned())
}
