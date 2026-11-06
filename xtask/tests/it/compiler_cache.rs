//! The compiler cache's coverage (`just compiler-cache`, `ci/compiler-cache.sh`),
//! which `just quick` and `just merge` run before any build: where the build's
//! Rust compiler wrapper is sccache, the checkout must be one of sccache's
//! base directories and `CMake` must compile C and C++ through sccache, or the
//! C/C++ objects other checkouts compiled (DuckDB, BoringSSL) miss here
//! (TESTING.md, Compiler cache). A host that opts in with
//! `COMPILER_CACHE_REQUIRED=1`, as the shared build server and `just merge`
//! do, fails on a gap; any other host is told the fix and passes. Where no
//! build uses sccache, the check is skipped. And `ci/compiler-launcher.sh`, the `CMake` compiler launcher
//! `.cargo/config.toml` sets, compiles through the Rust compiler wrapper when
//! it is sccache, as the `cc` crate does, and directly otherwise. sccache is a
//! stand-in that prints the statistics a test gives it. The compiler cache is
//! repository tooling, so the module declares no Feature.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use rstest::rstest;

use crate::workspace::{Workspace, finding};

/// The `[env]` lines of the repository's `.cargo/config.toml`.
const CONFIG: &str = "[env]\n\
    CMAKE_C_COMPILER_LAUNCHER = { value = \"ci/compiler-launcher.sh\", relative = true }\n\
    CMAKE_CXX_COMPILER_LAUNCHER = { value = \"ci/compiler-launcher.sh\", relative = true }\n";

fn repository_script(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ci")
        .join(name)
}

fn executable(path: &Path, script: &str) {
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A folder holding a stand-in `sccache` that prints `stats` as its
/// `--show-stats --stats-format json` and records each call's arguments.
fn stand_in_sccache(workspace: &Workspace, stats: &str) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("stats.json"), stats).unwrap();
    executable(
        &folder.join("sccache"),
        &format!(
            "#!/bin/sh\necho \"$*\" >>\"{calls}\"\ncat \"{stats}\"\n",
            calls = folder.join("calls").display(),
            stats = folder.join("stats.json").display(),
        ),
    );
    folder
}

/// sccache's statistics with these base directories, each ending in `/` as
/// sccache 0.18 lists them.
fn stats_with(basedirs: &[&Path]) -> String {
    let listed: Vec<String> = basedirs
        .iter()
        .map(|folder| format!("\"{}/\"", folder.display()))
        .collect();
    format!(
        "{{\"stats\":{{}},\"version\":\"0.18.0\",\"basedirs\":[{}]}}",
        listed.join(",")
    )
}

/// The checkout's folder as the build sees it, every link resolved.
fn checkout(workspace: &Workspace) -> PathBuf {
    workspace.path().canonicalize().unwrap()
}

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Run {
    fn findings(&self) -> Vec<crate::workspace::Finding> {
        crate::workspace::Outcome {
            code: self.code.unwrap_or(-1),
            stdout: self.stdout.clone(),
            stderr: String::new(),
        }
        .findings()
    }

    fn notes(&self) -> Vec<crate::workspace::Finding> {
        crate::workspace::Outcome {
            code: self.code.unwrap_or(-1),
            stdout: self.stdout.clone(),
            stderr: String::new(),
        }
        .notes()
    }
}

/// Runs `ci/compiler-cache.sh` in the workspace with `PATH` = `folder`, then
/// the system's folders, and these variables.
fn compiler_cache(workspace: &Workspace, folder: &Path, variables: &[(&str, &str)]) -> Run {
    let output = workspace
        .command(repository_script("compiler-cache.sh"))
        .env("PATH", format!("{}:/usr/bin:/bin", folder.display()))
        .envs(variables.iter().copied())
        .output()
        .unwrap();
    Run {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

const SCCACHE: (&str, &str) = ("RUSTC_WRAPPER", "sccache");

/// The opt-in that turns a gap into a failure.
const REQUIRED: (&str, &str) = ("COMPILER_CACHE_REQUIRED", "1");

#[rstest]
#[case::the_checkout_alone(false)]
#[case::beside_its_parent_folder(true)]
fn a_listed_checkout_with_the_repository_launcher_passes(#[case] with_parent: bool) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(".cargo/config.toml", CONFIG.as_bytes());
    let root = checkout(&workspace);
    let parent = root.parent().unwrap().to_owned();
    let listed: Vec<&Path> = if with_parent {
        vec![&parent, &root]
    } else {
        vec![&root]
    };
    let folder = stand_in_sccache(&workspace, &stats_with(&listed));

    let run = compiler_cache(&workspace, &folder, &[SCCACHE]);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert_eq!(run.findings(), vec![]);
}

/// A checkout outside the list, or under a listed folder that is not itself
/// listed, keeps its own path in every C/C++ key: nothing another checkout
/// compiled hits.
#[rstest]
#[case::no_base_directory(false)]
#[case::only_its_parent_folder(true)]
fn a_checkout_sccache_does_not_list_fails(#[case] with_parent: bool) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(".cargo/config.toml", CONFIG.as_bytes());
    let root = checkout(&workspace);
    let parent = root.parent().unwrap().to_owned();
    let listed: Vec<&Path> = if with_parent { vec![&parent] } else { vec![] };
    let folder = stand_in_sccache(&workspace, &stats_with(&listed));

    let run = compiler_cache(&workspace, &folder, &[SCCACHE, REQUIRED]);

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert_eq!(
        run.findings(),
        vec![finding("compiler-cache", &root.display().to_string())]
    );
}

#[test]
fn an_sccache_without_base_directories_fails() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(".cargo/config.toml", CONFIG.as_bytes());
    let folder = stand_in_sccache(&workspace, "{\"stats\":{},\"version\":\"0.7.0\"}");

    let run = compiler_cache(&workspace, &folder, &[SCCACHE, REQUIRED]);

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert_eq!(
        run.findings(),
        vec![finding(
            "compiler-cache",
            &checkout(&workspace).display().to_string()
        )]
    );
}

/// Without the launcher, `CMake` compiles BoringSSL without sccache: neither
/// `.cargo/config.toml` nor the environment sets it, or the environment,
/// which Cargo's `[env]` leaves alone, empties it or names another tool.
#[rstest]
#[case::unset_everywhere(false, None)]
#[case::emptied_by_the_environment(true, Some(""))]
#[case::another_tool_in_the_environment(true, Some("ccache"))]
fn a_launcher_that_bypasses_sccache_fails(#[case] config: bool, #[case] value: Option<&str>) {
    let workspace = Workspace::from_fixture("empty");
    if config {
        workspace.write(".cargo/config.toml", CONFIG.as_bytes());
    }
    let root = checkout(&workspace);
    let folder = stand_in_sccache(&workspace, &stats_with(&[&root]));
    let mut variables = vec![SCCACHE, REQUIRED];
    if let Some(value) = value {
        variables.push(("CMAKE_C_COMPILER_LAUNCHER", value));
        variables.push(("CMAKE_CXX_COMPILER_LAUNCHER", value));
    }

    let run = compiler_cache(&workspace, &folder, &variables);

    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert_eq!(
        run.findings(),
        vec![
            finding("compiler-cache", "CMAKE_C_COMPILER_LAUNCHER"),
            finding("compiler-cache", "CMAKE_CXX_COMPILER_LAUNCHER"),
        ]
    );
}

/// A contributor's machine, such as one with sccache but no base
/// directories, has not opted in: each gap is a note with its fix, and the
/// check passes, so `just quick` goes on.
#[rstest]
#[case::checkout_not_listed(true, &[], &[])]
#[case::launcher_bypasses_sccache(false, &["checkout"], &[("CMAKE_C_COMPILER_LAUNCHER", "ccache")])]
fn a_host_that_has_not_opted_in_is_told_the_fix_and_passes(
    #[case] config: bool,
    #[case] listed: &[&str],
    #[case] extra: &[(&str, &str)],
) {
    let workspace = Workspace::from_fixture("empty");
    if config {
        workspace.write(".cargo/config.toml", CONFIG.as_bytes());
    }
    let root = checkout(&workspace);
    let basedirs: Vec<&Path> = listed.iter().map(|_| root.as_path()).collect();
    let folder = stand_in_sccache(&workspace, &stats_with(&basedirs));
    let mut variables = vec![SCCACHE];
    variables.extend_from_slice(extra);

    let run = compiler_cache(&workspace, &folder, &variables);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert_eq!(run.findings(), vec![]);
    let subject = if config {
        root.display().to_string()
    } else {
        "CMAKE_C_COMPILER_LAUNCHER".to_owned()
    };
    assert!(
        run.notes().contains(&finding("compiler-cache", &subject)),
        "{}",
        run.stdout
    );
}

#[test]
fn sccache_named_in_the_environment_is_a_launcher_too() {
    let workspace = Workspace::from_fixture("empty");
    let root = checkout(&workspace);
    let folder = stand_in_sccache(&workspace, &stats_with(&[&root]));

    let run = compiler_cache(
        &workspace,
        &folder,
        &[
            SCCACHE,
            ("CMAKE_C_COMPILER_LAUNCHER", "/opt/bin/sccache"),
            ("CMAKE_CXX_COMPILER_LAUNCHER", "sccache"),
        ],
    );

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert_eq!(run.findings(), vec![]);
}

/// A host whose builds use no sccache, such as a contributor's without it
/// or a hosted CI runner, has no cache to cover: the check says so and
/// passes, reading nothing from sccache.
#[rstest]
#[case::no_rust_compiler_wrapper(&[], true)]
#[case::another_wrapper(&[("RUSTC_WRAPPER", "ccache")], true)]
#[case::sccache_not_installed(&[SCCACHE], false)]
fn a_host_without_sccache_skips_the_check(
    #[case] variables: &[(&str, &str)],
    #[case] installed: bool,
) {
    let workspace = Workspace::from_fixture("empty");
    let folder = stand_in_sccache(&workspace, &stats_with(&[]));
    let path = if installed {
        folder.clone()
    } else {
        workspace.path().join("no-tools")
    };

    let run = compiler_cache(&workspace, &path, variables);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("skipped"), "{}", run.stdout);
    assert!(!folder.join("calls").exists());
}

/// Runs `ci/compiler-launcher.sh` as `CMake` runs its compiler launcher, with
/// a stand-in compiler that records its arguments, and returns what the
/// stand-in sccache and the compiler recorded.
fn launch(workspace: &Workspace, wrapper: Option<&str>) -> (String, String) {
    let folder = stand_in_sccache(workspace, "");
    let compiler = folder.join("c++");
    executable(
        &compiler,
        &format!(
            "#!/bin/sh\necho \"$*\" >>\"{}\"\n",
            folder.join("compiled").display()
        ),
    );
    let mut command = workspace.command(repository_script("compiler-launcher.sh"));
    command
        .arg(&compiler)
        .args(["-c", "a b.cc", "-o", "a.o"])
        .env("PATH", format!("{}:/usr/bin:/bin", folder.display()));
    if let Some(wrapper) = wrapper {
        command.env("RUSTC_WRAPPER", wrapper);
    }
    let status = command.status().unwrap();
    assert!(status.success());
    let read = |name: &str| std::fs::read_to_string(folder.join(name)).unwrap_or_default();
    (read("calls"), read("compiled"))
}

#[test]
fn the_launcher_compiles_through_sccache_when_it_wraps_rustc() {
    let workspace = Workspace::from_fixture("empty");
    let sccache = workspace.path().join("stand-in/sccache");

    let (wrapped, _) = launch(&workspace, Some(&sccache.display().to_string()));

    let compiler = workspace.path().join("stand-in/c++");
    assert_eq!(
        wrapped,
        format!("{} -c a b.cc -o a.o\n", compiler.display())
    );
}

#[rstest]
#[case::no_wrapper(None)]
#[case::another_wrapper(Some("ccache"))]
fn the_launcher_runs_the_compiler_itself_otherwise(#[case] wrapper: Option<&str>) {
    let workspace = Workspace::from_fixture("empty");

    let (wrapped, compiled) = launch(&workspace, wrapper);

    assert_eq!(wrapped, "");
    assert_eq!(compiled, "-c a b.cc -o a.o\n");
}
