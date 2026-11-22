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
//! it is sccache, as the `cc` crate does, and directly otherwise. And
//! `ci/compiler-cache/sccache`, the Rust compiler wrapper the justfile sets
//! over sccache, hands sccache each compile of DuckDB's bundled sources from
//! its `OUT_DIR` with paths relative to it, so every profile and target
//! folder keys it alike, and every other call unchanged (issue 3231). sccache
//! is a stand-in that prints the statistics a test gives it, or records the
//! folder and arguments it was run with. The compiler cache is repository
//! tooling, so the module declares no Feature.
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

/// What the stand-in sccache recorded for a call: the folder it ran in and
/// its arguments.
#[derive(Debug, PartialEq)]
struct Call {
    folder: PathBuf,
    arguments: Vec<String>,
}

/// A stand-in sccache that records the folder and arguments of its last
/// call.
fn recording_sccache(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let sccache = folder.join("sccache");
    executable(
        &sccache,
        &format!(
            "#!/bin/sh\n{{ pwd -P; printf '%s\\n' \"$@\"; }} >\"{}\"\n",
            folder.join("call").display()
        ),
    );
    sccache
}

/// Runs `ci/compiler-cache/sccache` from `folder` with `arguments`, the
/// stand-in on `PATH` and these variables, and returns what the stand-in
/// recorded.
fn wrap(
    workspace: &Workspace,
    folder: &Path,
    arguments: &[String],
    variables: &[(&str, &Path)],
) -> Call {
    let sccache = recording_sccache(workspace);
    let stand_ins = sccache.parent().unwrap();
    let status = workspace
        .command(repository_script("compiler-cache/sccache"))
        .current_dir(folder)
        .args(arguments)
        .env("PATH", format!("{}:/usr/bin:/bin", stand_ins.display()))
        .envs(variables.iter().copied())
        .status()
        .unwrap();
    assert!(status.success());
    let recorded = std::fs::read_to_string(stand_ins.join("call")).unwrap();
    let mut lines = recorded.lines();
    Call {
        folder: PathBuf::from(lines.next().unwrap()),
        arguments: lines.map(str::to_owned).collect(),
    }
}

/// A folder of the workspace, created, every link resolved.
fn folder(workspace: &Workspace, relative: &str) -> PathBuf {
    let folder = workspace.path().join(relative);
    std::fs::create_dir_all(&folder).unwrap();
    folder.canonicalize().unwrap()
}

/// The package's folder, which the build script runs from.
const PACKAGE: &str = "registry/libduckdb-sys-1.20000.0";

/// Two build scripts' `OUT_DIR`s: a profile's in a checkout, and another
/// profile's in trybuild's target folder.
const OUT_DIRS: [&str; 2] = [
    "checkout/target/debug/build/libduckdb-sys-265611ae4e72fe17/out",
    "elsewhere/target/tests/trybuild/ci/build/libduckdb-sys-0f1e2d3c4b5a6978/out",
];

/// A compile of one of DuckDB's sources as the `cc` crate makes it, in `out`.
fn duckdb_compile(out: &Path) -> Vec<String> {
    let out = out.display();
    [
        "c++".to_owned(),
        "-O3".to_owned(),
        "--target=x86_64-unknown-linux-gnu".to_owned(),
        "-I".to_owned(),
        format!("{out}/duckdb/src/include"),
        "-I".to_owned(),
        "duckdb".to_owned(),
        format!("-I{out}/duckdb/third_party/re2"),
        "-Iduckdb/third_party".to_owned(),
        "-std=c++17".to_owned(),
        "-DNDEBUG".to_owned(),
        "-o".to_owned(),
        format!("{out}/a5ad43c7d05478b1-ub_src_storage_table.o"),
        "-c".to_owned(),
        format!("{out}/duckdb/src/storage/ub_src_storage_table.cpp"),
    ]
    .into()
}

/// Runs a compile of DuckDB's sources in `out` from the package's folder,
/// as `libduckdb-sys`'s build script does, with `arguments` in place of the
/// `cc` crate's.
fn wrap_duckdb(workspace: &Workspace, out: &Path, arguments: &[String]) -> Call {
    let sccache = recording_sccache(workspace);
    wrap(
        workspace,
        &folder(workspace, PACKAGE),
        arguments,
        &[
            ("CARGO_PKG_NAME", Path::new("libduckdb-sys")),
            ("OUT_DIR", out),
            ("GOODFETCH_SCCACHE", &sccache),
        ],
    )
}

#[test]
fn a_duckdb_compile_reaches_sccache_alike_from_any_target_folder() {
    let workspace = Workspace::from_fixture("empty");
    let package = folder(&workspace, PACKAGE);
    let [first, second] = OUT_DIRS.map(|out| folder(&workspace, out));

    let calls = [&first, &second].map(|out| wrap_duckdb(&workspace, out, &duckdb_compile(out)));

    let relative = [
        "c++",
        "-O3",
        "--target=x86_64-unknown-linux-gnu",
        "-I",
        "duckdb/src/include",
        "-I",
        &format!("{}/duckdb", package.display()),
        "-Iduckdb/third_party/re2",
        &format!("-I{}/duckdb/third_party", package.display()),
        "-std=c++17",
        "-DNDEBUG",
        "-o",
        "a5ad43c7d05478b1-ub_src_storage_table.o",
        "-c",
        "duckdb/src/storage/ub_src_storage_table.cpp",
    ]
    .map(str::to_owned)
    .to_vec();
    assert_eq!(
        calls,
        [
            Call {
                folder: first,
                arguments: relative.clone(),
            },
            Call {
                folder: second,
                arguments: relative,
            },
        ]
    );
}

/// A change of the compiler, the target, the optimization, a define or a
/// flag reaches sccache's key, and nothing else does: the two compiles differ
/// in that argument alone, though they ran in two target folders.
#[rstest]
#[case::compiler("c++", "clang++")]
#[case::target(
    "--target=x86_64-unknown-linux-gnu",
    "--target=x86_64-unknown-linux-musl"
)]
#[case::optimization("-O3", "-O1")]
#[case::define("-DNDEBUG", "-DDEBUG")]
#[case::flag("-std=c++17", "-std=c++20")]
fn a_changed_compile_reaches_sccache_changed(#[case] before: &str, #[case] after: &str) {
    let workspace = Workspace::from_fixture("empty");
    let [first, second] = OUT_DIRS.map(|out| folder(&workspace, out));
    let changed: Vec<String> = duckdb_compile(&second)
        .into_iter()
        .map(|argument| {
            if argument == before {
                after.to_owned()
            } else {
                argument
            }
        })
        .collect();

    let unchanged = wrap_duckdb(&workspace, &first, &duckdb_compile(&first)).arguments;
    let changed = wrap_duckdb(&workspace, &second, &changed).arguments;

    let differing: Vec<(&String, &String)> = unchanged
        .iter()
        .zip(&changed)
        .filter(|(one, other)| one != other)
        .collect();
    assert_eq!(differing, [(&before.to_owned(), &after.to_owned())]);
}

/// Rust compiles, other packages' C and C++ compiles and calls to sccache
/// itself reach sccache as Cargo or the build script made them, from the
/// folder they ran in.
#[rstest]
#[case::the_duckdb_crate_s_rust("libduckdb-sys", "rustc")]
#[case::clippy_on_it("libduckdb-sys", "clippy-driver")]
#[case::another_package_s_cxx("zstd-sys", "c++")]
#[case::no_package("", "--show-stats")]
fn every_other_call_reaches_sccache_unchanged(#[case] package: &str, #[case] program: &str) {
    let workspace = Workspace::from_fixture("empty");
    let out = folder(&workspace, OUT_DIRS[0]);
    let from = folder(&workspace, PACKAGE);
    let sccache = recording_sccache(&workspace);
    let mut arguments = duckdb_compile(&out);
    arguments[0] = format!("/toolchain/bin/{program}");
    if program.starts_with('-') {
        arguments[0] = program.to_owned();
    }

    let call = wrap(
        &workspace,
        &from,
        &arguments,
        &[
            ("CARGO_PKG_NAME", Path::new(package)),
            ("OUT_DIR", &out),
            ("GOODFETCH_SCCACHE", &sccache),
        ],
    );

    assert_eq!(
        call,
        Call {
            folder: from,
            arguments
        }
    );
}

/// A saved sccache that is missing, or that names this wrapper, fails the
/// call, as a missing wrapper would, rather than compiling outside the cache
/// or running itself.
#[rstest]
#[case::missing(false)]
#[case::this_wrapper(true)]
fn a_wrapper_without_its_sccache_fails(#[case] itself: bool) {
    let workspace = Workspace::from_fixture("empty");
    let out = folder(&workspace, OUT_DIRS[0]);
    let sccache = recording_sccache(&workspace);
    let saved = if itself {
        repository_script("compiler-cache/sccache")
    } else {
        workspace.path().join("missing/sccache")
    };

    let status = workspace
        .command(repository_script("compiler-cache/sccache"))
        .current_dir(folder(&workspace, PACKAGE))
        .args(duckdb_compile(&out))
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", sccache.parent().unwrap().display()),
        )
        .env("CARGO_PKG_NAME", "libduckdb-sys")
        .env("OUT_DIR", &out)
        .env("GOODFETCH_SCCACHE", &saved)
        .status()
        .unwrap();

    assert_eq!(
        (status.code(), sccache.with_file_name("call").exists()),
        (Some(2), false)
    );
}

/// The justfile's value of `variable`, with these variables in the
/// environment.
fn evaluate(workspace: &Workspace, variable: &str, variables: &[(&str, &str)]) -> String {
    let shims = folder(workspace, "shims");
    let output = workspace
        .just(&shims)
        .arg("--evaluate")
        .arg(variable)
        .envs(variables.iter().copied())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// Where the environment's Rust compiler wrapper is sccache, the justfile's
/// recipes build through ci/compiler-cache/sccache and save that sccache for
/// it; a nested `just` keeps the one its parent saved.
#[rstest]
#[case::sccache_on_path(&[("RUSTC_WRAPPER", "sccache")], "sccache")]
#[case::sccache_by_path(&[("RUSTC_WRAPPER", "/opt/bin/sccache")], "/opt/bin/sccache")]
#[case::nested(
    &[("RUSTC_WRAPPER", "/checkout/ci/compiler-cache/sccache"), ("GOODFETCH_SCCACHE", "/opt/bin/sccache")],
    "/opt/bin/sccache"
)]
fn recipes_build_through_the_wrapper_where_sccache_wraps_rustc(
    #[case] variables: &[(&str, &str)],
    #[case] saved: &str,
) {
    let workspace = Workspace::from_fixture("empty");

    let rustc_wrapper = evaluate(&workspace, "RUSTC_WRAPPER", variables);
    let sccache = evaluate(&workspace, "GOODFETCH_SCCACHE", variables);

    assert_eq!(
        (Path::new(&rustc_wrapper).canonicalize().unwrap(), sccache),
        (
            repository_script("compiler-cache/sccache")
                .canonicalize()
                .unwrap(),
            saved.to_owned()
        )
    );
}

/// Any other Rust compiler wrapper, or none, stays the environment's, and no
/// sccache is saved.
#[rstest]
#[case::another_wrapper(&[("RUSTC_WRAPPER", "ccache")], "ccache")]
#[case::no_wrapper(&[], "")]
fn recipes_keep_any_other_wrapper(#[case] variables: &[(&str, &str)], #[case] wrapper: &str) {
    let workspace = Workspace::from_fixture("empty");

    let rustc_wrapper = evaluate(&workspace, "RUSTC_WRAPPER", variables);
    let sccache = evaluate(&workspace, "GOODFETCH_SCCACHE", variables);

    assert_eq!((rustc_wrapper.as_str(), sccache.as_str()), (wrapper, ""));
}
