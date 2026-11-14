//! `just compare <profile>` (issue 2099): the repository's recipe, run in an
//! empty fixture with stand-ins for Cargo, xtask, pnpm and uv on the path,
//! each of which records how it was called and builds nothing (`jq` gives
//! the fixture's target folder). Cargo writes the `dev` profile's builds to
//! `debug/` and every other profile's to a folder of its name, so the recipe
//! hands `xtask compare run` that folder, where the matrix reads the sides'
//! executables, and installs fastCRW's server into it.

use std::path::Path;

use rstest::rstest;

use crate::workspace::{Workspace, write_executable};

/// A stand-in that appends its name and arguments to `calls`.
fn recorder(calls: &Path) -> String {
    format!(
        "#!/bin/sh\nprintf '%s %s\\n' \"$(basename \"$0\")\" \"$*\" >> '{}'\n",
        calls.display()
    )
}

/// Runs `just compare <profile>` on the repository's justfile in the
/// fixture, and returns the recorded calls.
fn compare(profile: &str) -> Vec<String> {
    let workspace = Workspace::from_fixture("empty");
    let root = workspace.path();
    let calls = root.join("calls");
    let shims = root.join("shims");
    std::fs::create_dir_all(&shims).unwrap();
    for program in ["cargo", "pnpm", "uv"] {
        write_executable(&shims.join(program), &recorder(&calls));
    }
    // `cargo metadata`'s target folder, as the recipe's `jq` reads it.
    write_executable(
        &shims.join("jq"),
        "#!/bin/sh\nprintf '%s/target\\n' \"$PWD\"\n",
    );
    std::fs::create_dir_all(root.join("target/debug")).unwrap();
    write_executable(&root.join("target/debug/xtask"), &recorder(&calls));
    std::fs::create_dir_all(root.join("ci/musl")).unwrap();
    write_executable(&root.join("ci/musl/install.sh"), "#!/bin/sh\n");
    let path = std::env::join_paths(
        std::iter::once(shims).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let justfile = Path::new(env!("CARGO_MANIFEST_DIR")).join("../justfile");

    let output = workspace
        .command("just")
        .arg("--justfile")
        .arg(justfile)
        .arg("--working-directory")
        .arg(root)
        .args(["compare", profile])
        .env("PATH", path)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::read_to_string(calls)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The value that follows `option` in the one recorded call that starts
/// with `call`.
fn option_of<'a>(calls: &'a [String], call: &str, option: &str) -> &'a str {
    let [found] = calls
        .iter()
        .filter(|recorded| recorded.starts_with(call))
        .collect::<Vec<_>>()[..]
    else {
        panic!("no one call starts with {call:?}: {calls:#?}");
    };
    let mut words = found.split(' ');
    words
        .by_ref()
        .find(|word| *word == option)
        .unwrap_or_else(|| panic!("{call:?} takes no {option}: {found}"));
    words.next().unwrap()
}

/// `just compare dev` reads the sides' executables from `debug/`, where
/// Cargo wrote them, and installs fastCRW's server there; `release` reads
/// and installs into `release/`, as before.
#[rstest]
#[case::dev("dev", "debug")]
#[case::release("release", "release")]
fn just_compare_reads_each_profiles_builds_from_the_folder_cargo_wrote(
    #[case] profile: &str,
    #[case] folder: &str,
) {
    let calls = compare(profile);

    assert_eq!(
        (
            option_of(
                &calls,
                "cargo --config xtask/compare-fastcrw/server-profile.toml install",
                "--root"
            ),
            option_of(&calls, "xtask compare run", "--spider-profile"),
        ),
        (
            format!("target/compare-spider/fastcrw/{folder}").as_str(),
            folder,
        )
    );
}
