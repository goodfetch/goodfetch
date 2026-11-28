//! Each language's `browser` starter as a person starts it (story:11-275):
//! `init` writes it into a folder whose name holds a space and a non-ASCII
//! letter, the language's tools install and build it, its own test passes,
//! and its program prints the title of a page of the local Proving ground,
//! every engine Collie's executable. Collie's packages are not published
//! yet, so the test points the manifest's Collie dependency at the
//! repository's own package, pins the other dependencies to the versions
//! the repository locks, and installs offline from the package store, uv
//! cache and Cargo home the repository's own installs filled. The Rust
//! project builds Collie's crate from source, so it runs in the Release
//! suite alone.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests rewrite the project's manifest and run its tools with the running test's PATH and the user's package caches"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, ProvingGround, TestEnv, browser_cache, cargo_command, cli,
};

use super::{COLLIE, PROJECT};
use crate::release_suite::skipped_outside_the_release_suite;

const SEED: u64 = 1422;

/// The repository's root folder.
const REPOSITORY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// The Proving ground page each program reads: its host, path and title.
const PAGE: (&str, &str, &str) = ("codecs.test", "/", "Codecs");

/// pnpm's command, a batch file on Windows.
const PNPM: &str = if cfg!(windows) { "pnpm.cmd" } else { "pnpm" };

/// A starter `init` wrote into the test's [`PROJECT`] folder, with the
/// Proving ground its program reads and the Browser cache its engines
/// launch from. The Proving ground stops before the test's teardown, which
/// fails on a process still alive.
struct Project {
    proving_ground: ProvingGround,
    env: TestEnv,
    browsers: PathBuf,
}

impl Project {
    /// The `browser` starter in `language`, written by Collie's executable,
    /// with a user configuration that reaches the Proving ground's hosts.
    fn written(language: &str) -> Self {
        let env = TestEnv::new();
        let output = cli(&env, COLLIE)
            .args(["init", PROJECT, "--starter", "browser", "--lang", language])
            .run()
            .unwrap();
        assert_eq!(output.exit, Some(0), "{output}");
        let proving_ground = ProvingGround::start(&env, SEED).unwrap();
        env.user_configuration(&proving_ground.description().reaching());
        let browsers = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
        Self {
            proving_ground,
            env,
            browsers,
        }
    }

    /// The project's folder.
    fn folder(&self) -> PathBuf {
        self.env.folder().join(PROJECT)
    }

    /// The page the program reads.
    fn page(&self) -> String {
        let (host, path, _) = PAGE;
        self.proving_ground.url(host, path)
    }

    /// `program`, run in the project's folder with the test's map, the
    /// running test's `PATH`, Collie's executable as every SDK's engine and
    /// the test's Browser cache.
    fn command(&self, program: &str) -> Command {
        let mut command = self.env.process_command(program);
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        command
            .current_dir(self.folder())
            .env("GOODFETCH_COLLIE_ENGINE_PATH", COLLIE)
            .env(BROWSERS_PATH_VARIABLE, &self.browsers);
        command
    }

    /// The text of the project's file at `path`.
    fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.folder().join(path)).unwrap()
    }

    /// Writes `text` to the project's file at `path`.
    fn write(&self, path: &str, text: &str) {
        std::fs::write(self.folder().join(path), text).unwrap();
    }
}

/// The repository's folder at `path` from its root, as an absolute path.
fn repository(path: &str) -> String {
    Path::new(REPOSITORY)
        .join(path)
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// `text` as a TOML string, which a JSON string is.
fn toml_string(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

/// What `command` printed, once it exited successfully.
fn succeeded(command: &mut Command) -> String {
    let output = command.output().unwrap();
    assert!(output.status.success(), "{command:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// What `program` prints for `args` in the repository's folder, in the
/// running test's own environment: the user's package caches and the
/// repository's lock.
fn the_users(program: &str, args: &[&str]) -> String {
    succeeded(Command::new(program).args(args).current_dir(REPOSITORY))
        .trim()
        .to_owned()
}

/// The TypeScript starter installs with pnpm, type-checks, passes its own
/// test and prints the page's title.
///
/// Covers: story:11-275, command:init
#[test]
fn the_typescript_starter_builds_tests_itself_and_reads_a_page() {
    let project = Project::written("typescript");
    let manifest = project.read("package.json");
    let mut package: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    package["dependencies"]["@goodfetch/collie"] =
        format!("link:{}", repository("sdks/typescript/collie")).into();
    project.write("package.json", &package.to_string());
    let store = the_users(PNPM, &["store", "path"]);
    let cache = the_users(PNPM, &["cache", "path"]);
    let pnpm = |args: &[&str]| {
        let mut command = project.command(PNPM);
        command
            .args(args)
            .env("pnpm_config_store_dir", &store)
            .env("pnpm_config_cache_dir", &cache);
        command
    };

    for step in [
        &["install", "--offline"][..],
        &["run", "typecheck"],
        &["test"],
    ] {
        succeeded(&mut pnpm(step));
    }
    let printed = succeeded(&mut pnpm(&["--silent", "start", &project.page()]));

    assert_eq!(printed.trim_end(), PAGE.2);
}

/// The Python starter installs with uv, passes its own test and prints
/// the page's title.
///
/// Covers: story:11-275, command:init
#[test]
fn the_python_starter_builds_tests_itself_and_reads_a_page() {
    let project = Project::written("python");
    let locked: Vec<String> = the_users(
        "uv",
        &[
            "export",
            "--frozen",
            "--no-hashes",
            "--all-packages",
            "--all-groups",
            "--no-emit-workspace",
            "--no-header",
            "--no-annotate",
        ],
    )
    .lines()
    .map(str::to_owned)
    .collect();
    let manifest = project.read("pyproject.toml");
    let constrained = manifest.replacen(
        "package = false\n",
        &format!(
            "package = false\nconstraint-dependencies = {}\n",
            serde_json::to_string(&locked).unwrap()
        ),
        1,
    );
    assert_ne!(constrained, manifest, "the manifest has a [tool.uv] table");
    let sources: Vec<String> = [
        ("goodfetch-collie", "sdks/python/collie"),
        ("goodfetch-collie-client", "sdks/python/collie-client"),
    ]
    .iter()
    .map(|(package, path)| {
        format!(
            "{package} = {{ path = {}, editable = true }}",
            toml_string(&repository(path))
        )
    })
    .collect();
    project.write(
        "pyproject.toml",
        &format!("{constrained}\n[tool.uv.sources]\n{}\n", sources.join("\n")),
    );
    let cache = the_users("uv", &["cache", "dir"]);
    let pythons = the_users("uv", &["python", "dir"]);
    let uv = |args: &[&str]| {
        let mut command = project.command("uv");
        command
            .args(args)
            .env("UV_CACHE_DIR", &cache)
            .env("UV_PYTHON_INSTALL_DIR", &pythons)
            .env("UV_PYTHON_DOWNLOADS", "never");
        command
    };

    succeeded(&mut uv(&["sync", "--offline"]));
    succeeded(&mut uv(&["run", "--offline", "pytest"]));
    let printed = succeeded(&mut uv(&["run", "--offline", "main.py", &project.page()]));

    assert_eq!(printed.trim_end(), PAGE.2);
}

/// The Rust starter builds with Cargo, passes its own test and prints the
/// page's title. It builds Collie's crate and engine from source, so it
/// runs in the Release suite alone, into a target folder kept between
/// runs.
///
/// Covers: story:11-275, command:init
#[test]
fn the_rust_starter_builds_tests_itself_and_reads_a_page() {
    if skipped_outside_the_release_suite("a build of Collie's crate from source") {
        return;
    }
    let project = Project::written("rust");
    let workspace: toml::Table = std::fs::read_to_string(Path::new(REPOSITORY).join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let mut patches = vec![format!(
        "goodfetch-collie = {{ path = {} }}",
        toml_string(&repository("crates/goodfetch-collie"))
    )];
    let vendored = workspace["patch"]["crates-io"].as_table().unwrap();
    for (package, patch) in vendored {
        let path = patch["path"].as_str().unwrap();
        patches.push(format!(
            "{package} = {{ path = {} }}",
            toml_string(&repository(path))
        ));
    }
    let manifest = project.read("Cargo.toml");
    project.write(
        "Cargo.toml",
        &format!("{manifest}\n[patch.crates-io]\n{}\n", patches.join("\n")),
    );
    std::fs::copy(
        Path::new(REPOSITORY).join("Cargo.lock"),
        project.folder().join("Cargo.lock"),
    )
    .unwrap();
    let cargo = |args: &[&str]| {
        let mut command = cargo_command(&project.env);
        command
            .args(args)
            .current_dir(project.folder())
            .env("GOODFETCH_COLLIE_ENGINE_PATH", COLLIE)
            .env(BROWSERS_PATH_VARIABLE, &project.browsers)
            .env(
                "CARGO_TARGET_DIR",
                Path::new(env!("CARGO_TARGET_TMPDIR")).join("rust-starter"),
            )
            .env("CARGO_PROFILE_DEV_DEBUG", "false")
            .env("CARGO_INCREMENTAL", "0");
        command
    };

    succeeded(&mut cargo(&["test", "--offline"]));
    let printed = succeeded(&mut cargo(&[
        "run",
        "--offline",
        "--quiet",
        "--",
        &project.page(),
    ]));

    assert_eq!(printed.trim_end(), PAGE.2);
}
