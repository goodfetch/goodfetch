//! `ci/ignored-sources.sh`, in `just lint`: `.gitignore` reaches no crate's
//! sources (#1761) and ignores the build folder a tool makes in each Cargo
//! package's folder, such as gungraun's under `cargo bench`, which left the
//! gate's tree unclean. Run on a fixture repository with a given
//! `.gitignore`. Repository tooling, so the module declares no Feature.
#![cfg(unix)]

use std::path::Path;

use rstest::rstest;

use crate::workspace::Workspace;

/// A repository with a root package, a package `crates/a` whose module
/// `guard/target.rs` has its tests in `guard/target/tests.rs`, all tracked,
/// and `gitignore` as its `.gitignore`.
fn repository(gitignore: &str) -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    for file in [
        "Cargo.toml",
        "crates/a/Cargo.toml",
        "crates/a/src/lib.rs",
        "crates/a/src/guard/target.rs",
        "crates/a/src/guard/target/tests.rs",
    ] {
        workspace.write(file, b"");
    }
    workspace.git(&["init", "--quiet"]);
    workspace.git(&["add", "--all", "--force"]);
    workspace.write(".gitignore", gitignore.as_bytes());
    workspace
}

fn check(workspace: &Workspace) -> (Option<i32>, String) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/ignored-sources.sh");
    let output = workspace
        .command(script)
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[rstest]
#[case::a_module_folder_named_target(
    "target/\n",
    "error[ignored-sources] crates/a/src/guard/target/tests.rs: .gitignore ignores this source path (.gitignore:1:target/)"
)]
#[case::a_packages_own_build_folder(
    "/target\n",
    "error[unignored-build-folder] crates/a/target/: .gitignore does not ignore this package's build folder"
)]
fn a_gitignore_that_hides_a_source_or_shows_a_build_folder_fails(
    #[case] gitignore: &str,
    #[case] error: &str,
) {
    let workspace = repository(gitignore);

    let (code, stdout) = check(&workspace);

    assert_eq!(code, Some(1), "{stdout}");
    assert!(stdout.contains(error), "{stdout}");
}

/// This repository's own `.gitignore` passes both.
#[test]
fn the_gitignore_of_this_repository_passes() {
    let own = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.gitignore"))
        .unwrap();
    let workspace = repository(&own);

    let (code, stdout) = check(&workspace);

    assert_eq!(code, Some(0), "{stdout}");
}
