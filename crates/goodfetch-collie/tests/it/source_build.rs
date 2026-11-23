//! A clean container builds the crate from source with the tools its docs
//! list, and without any one of them the build fails (story 11-280; ADRs
//! 0094 and 0124). The container runs `ci/source-build/Containerfile`'s
//! image, Rust on Debian with exactly those tools, which
//! `ci/source-build/fetch.sh` builds before ADR 0043's deny, through Docker
//! or Podman. Nothing is downloaded: Cargo reads the workspace's locked
//! dependencies from a copy `cargo vendor` makes from Cargo's own cache,
//! and builds the crate with its executable from the workspace, read-only,
//! as `cargo install --features executable` builds it from crates.io.
//!
//! The builds share one target folder, a volume of the engine's that the
//! test removes: each build without a tool runs first, in a container of
//! its own whose packages of that tool are removed, and stops at the build
//! script that needs it, so the next reuses what it compiled; the build
//! with every tool runs last. It takes most of a build of the crate, so it
//! runs in the Release suite alone, on its Linux x86-64 glibc cell, with
//! every test thread (`.config/nextest.toml`); elsewhere it says so and
//! skips. `CARGO_BUILD_JOBS`, when set, reaches the container's Cargo.
#![expect(
    clippy::disallowed_methods,
    reason = "the test runs the container engine and Cargo, and reads the Containerfile, outside any async worker"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use goodfetch_test_support::{Suite, TestEnv, Variables};
use sha2::{Digest, Sha256};

use crate::install_docs::{CONTAINERFILE, PREREQUISITES, Prerequisite};

/// Cargo's exit status when a build fails.
const BUILD_FAILED: i32 = 101;

/// The workspace, which the container mounts read-only.
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace is there")
}

/// rust-toolchain.toml's channel, the image's toolchain.
fn toolchain() -> String {
    let file: toml::Table = include_str!("../../../../rust-toolchain.toml")
        .parse()
        .expect("rust-toolchain.toml is TOML");
    let channel = file["toolchain"]["channel"]
        .as_str()
        .expect("rust-toolchain.toml names its channel");
    assert!(
        CONTAINERFILE.contains(&format!("FROM docker.io/library/rust:{channel}-")),
        "ci/source-build/Containerfile's image holds Rust {channel}"
    );
    channel.to_owned()
}

/// Whether the test skips here, outside the Release suite, saying so on
/// stderr. Asked before the test's [`TestEnv`], so a skipped test leaves no
/// exercise record.
fn skipped_outside_the_release_suite() -> bool {
    let suite = Variables::from_process()
        .suite()
        .expect("GOODFETCH_TEST_SUITE names a suite");
    if suite == Suite::Release {
        return false;
    }
    #[expect(
        clippy::print_stderr,
        reason = "a skipped test says why in its own output, which the test harness keeps"
    )]
    {
        eprintln!("skipped outside the Release suite: a build of the crate in a container");
    }
    true
}

/// The container engine that answers on this host, Docker or Podman, as
/// `ci/container.sh` picks it.
fn engine() -> &'static str {
    ["docker", "podman"]
        .into_iter()
        .find(|engine| {
            Command::new(engine)
                .arg("info")
                .output()
                .is_ok_and(|output| output.status.success())
        })
        .expect("the Release suite has Docker or Podman for the source build")
}

/// The clean container's image, named by its Containerfile's digest as
/// `ci/source-build/fetch.sh` tags it.
fn image(engine: &str) -> String {
    let digest = data_encoding::HEXLOWER.encode(&Sha256::digest(CONTAINERFILE));
    let image = format!("goodfetch-source-build:{}", &digest[..16]);
    let found = Command::new(engine)
        .args(["image", "inspect", &image])
        .output()
        .expect("the engine starts");
    assert!(
        found.status.success(),
        "{image} is missing: `ci/source-build/fetch.sh` builds it before the deny"
    );
    image
}

/// The workspace's locked dependencies, copied from Cargo's cache into the
/// test's folder.
fn vendor(env: &TestEnv) -> PathBuf {
    let vendor = env.folder().join("vendor");
    let output = Command::new("cargo")
        .arg("vendor")
        .arg("--manifest-path")
        .arg(workspace().join("Cargo.toml"))
        .args(["--locked", "--offline", "--versioned-dirs"])
        .arg(&vendor)
        .output()
        .expect("Cargo starts");
    assert!(output.status.success(), "cargo vendor: {}", tail(&output));
    vendor
}

/// The engine's volume the builds share as their target folder, removed
/// when the test ends.
struct Volume<'a> {
    engine: &'a str,
    name: String,
}

impl<'a> Volume<'a> {
    fn create(engine: &'a str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is past 1970")
            .as_nanos();
        let name = format!("goodfetch-source-build-{}-{nanos}", std::process::id());
        let created = Command::new(engine)
            .args(["volume", "create", &name])
            .output()
            .expect("the engine starts");
        assert!(
            created.status.success(),
            "volume create: {}",
            tail(&created)
        );
        Self { engine, name }
    }
}

impl Drop for Volume<'_> {
    fn drop(&mut self) {
        let _ = Command::new(self.engine)
            .args(["volume", "rm", "--force", &self.name])
            .output();
    }
}

/// In the container: removes the installed packages `REMOVE` matches, if
/// it is set, then builds the crate with its executable, offline, from the
/// vendored dependencies.
const BUILD: &str = r#"set -euo pipefail
if [[ -n "${REMOVE:-}" ]]; then
  packages=$(dpkg-query -W -f '${db:Status-Abbrev} ${binary:Package}\n' "$REMOVE" | awk '$1 == "ii" { print $2 }')
  [[ -n "$packages" ]] || { echo "no installed package matches $REMOVE" >&2; exit 3; }
  dpkg --purge --force-depends $packages >/dev/null
fi
exec cargo build --locked --offline -p goodfetch-collie --features executable \
  --config 'source.crates-io.replace-with="vendored"' \
  --config 'source.vendored.directory="/vendor"'
"#;

/// The crate's build in a fresh container, without `missing`'s packages
/// when it names a tool.
fn build(
    engine: &str,
    image: &str,
    vendor: &Path,
    volume: &Volume<'_>,
    missing: Option<&Prerequisite>,
) -> Output {
    let mut command = Command::new(engine);
    command
        .args(["run", "--rm", "--network", "none", "--workdir", "/src"])
        .arg("--volume")
        .arg(format!("{}:/src:ro", workspace().display()))
        .arg("--volume")
        .arg(format!("{}:/vendor:ro", vendor.display()))
        .arg("--volume")
        .arg(format!("{}:/build", volume.name))
        // The image's toolchain, rust-toolchain.toml's, without the
        // components that file adds, which would be downloaded.
        .arg("--env")
        .arg(format!("RUSTUP_TOOLCHAIN={}", toolchain()))
        .args(["--env", "CARGO_TARGET_DIR=/build"])
        // No debug information, as `cargo install`'s release profile: the
        // target folder takes a third of the room.
        .args(["--env", "CARGO_PROFILE_DEV_DEBUG=false"]);
    if let Some(tool) = missing {
        command.args(["--env", &format!("REMOVE={}", tool.installed_as)]);
    }
    if let Ok(jobs) = std::env::var("CARGO_BUILD_JOBS") {
        command.args(["--env", &format!("CARGO_BUILD_JOBS={jobs}")]);
    }
    command
        .args([image, "bash", "-c", BUILD])
        .output()
        .expect("the engine starts")
}

/// The last lines of a command's stderr.
fn tail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    lines[lines.len().saturating_sub(40)..].join("\n")
}

/// With every tool its docs list, a clean container builds the crate and
/// its executable from source; without any one of them, a build script
/// fails the build.
///
/// Covers: story:11-280
#[test]
fn a_clean_container_builds_the_crate_with_the_listed_tools_and_fails_without_any_one() {
    if skipped_outside_the_release_suite() {
        return;
    }
    let env = TestEnv::new();
    let engine = engine();
    let image = image(engine);
    let vendor = vendor(&env);
    let volume = Volume::create(engine);

    for tool in &PREREQUISITES {
        let without = build(engine, &image, &vendor, &volume, Some(tool));
        let stderr = String::from_utf8_lossy(&without.stderr);
        assert!(
            without.status.code() == Some(BUILD_FAILED)
                && stderr.contains("error: failed to run custom build command"),
            "without {}, the build ended {} and not on a build script: {}",
            tool.name,
            without.status,
            tail(&without)
        );
    }

    let with_every_tool = build(engine, &image, &vendor, &volume, None);
    assert!(
        with_every_tool.status.success(),
        "with every listed tool, the build failed: {}",
        tail(&with_every_tool)
    );
}
