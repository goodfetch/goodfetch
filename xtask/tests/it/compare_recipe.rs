//! `just compare <profile>` (issue 2099): the repository's recipe, run in an
//! empty fixture with stand-ins for Cargo, xtask, pnpm and uv on the path,
//! each of which records how it was called and builds nothing (`jq` gives
//! the fixture's target folder). Cargo writes the `dev` profile's builds to
//! `debug/` and every other profile's to a folder of its name, so the recipe
//! installs fastCRW's server into that folder, beside fastCRW's side, and
//! hands `xtask compare run` the profile itself, which xtask maps to the
//! same folder and its rerun command names.

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

/// Runs `just compare [profile]` on the repository's justfile in the
/// fixture, and returns the recorded calls.
fn compare(profile: Option<&str>) -> Vec<String> {
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

    let output = workspace
        .just(&shims)
        .arg("compare")
        .args(profile)
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

/// `just compare dev` installs fastCRW's server into `debug/`, where Cargo
/// writes the sides' `dev` builds, and runs xtask with the profile `dev`;
/// `release`, or no profile, installs into `release/` and runs with
/// `release`, as before.
#[rstest]
#[case::dev(Some("dev"), "dev", "debug")]
#[case::release(Some("release"), "release", "release")]
#[case::no_profile(None, "release", "release")]
fn just_compare_installs_into_the_profiles_folder_and_runs_with_the_profile(
    #[case] asked: Option<&str>,
    #[case] profile: &str,
    #[case] folder: &str,
) {
    let calls = compare(asked);

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
            profile,
        )
    );
}
