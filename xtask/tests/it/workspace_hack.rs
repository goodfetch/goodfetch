//! `ci/workspace-hack.sh check` (`just lint`), which fails when the
//! workspace-hack is out of date (ADR 0094): it regenerates the crate's
//! manifest away from the tree it checks, so it leaves every file of that
//! tree as it was, content and modification time, and a build or a test run
//! beside it in the same tree reads nothing it wrote (issue 3236). Here the
//! script runs on a small workspace with `cargo` a stand-in that writes as
//! `cargo hakari` does: `generate` writes the workspace-hack's section and
//! writes `Cargo.lock` back with its own bytes, and `manage-deps --dry-run`
//! writes `Cargo.lock` back too. The workspace-hack is repository tooling,
//! so the module declares no Feature.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use crate::workspace::write_executable;

/// The build-side workspace-hack's manifest, from the workspace's folder.
const MANIFEST: &str = "crates/goodfetch-workspace-hack/Cargo.toml";

/// The dev code-side workspace-hack's manifest, from the workspace's folder.
const DEV_MANIFEST: &str = "crates/goodfetch-dev-workspace-hack/Cargo.toml";

/// A member crate's manifest, from the workspace's folder.
const MEMBER_MANIFEST: &str = "crates/goodfetch-sample/Cargo.toml";

/// The excluded protocol crate's manifest, from the workspace's folder.
const PROTOCOL_MANIFEST: &str = "crates/goodfetch-protocol/Cargo.toml";

/// A workspace-hack manifest named `name` with `section` as its hakari section.
fn manifest_with(name: &str, section: &str) -> String {
    format!(
        "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\n\n\
         ### BEGIN HAKARI SECTION\n{section}### END HAKARI SECTION\n"
    )
}

/// What the stand-in's `generate` writes: both sides, of which the check
/// splits the build side into `goodfetch-workspace-hack` and the code side
/// into `goodfetch-dev-workspace-hack`.
const GENERATED: &str = "[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\n\n\
    [build-dependencies]\ncc = { version = \"1\", features = [\"parallel\"] }\n\n\
    [target.'cfg(unix)'.dependencies]\nlibc = { version = \"0.2\" }\n\n";

/// The build side of [`GENERATED`], as the committed build-side manifest holds it.
const BUILD_SIDE: &str =
    "[build-dependencies]\ncc = { version = \"1\", features = [\"parallel\"] }\n\n";

/// The code side of [`GENERATED`], as the committed dev-side manifest holds it.
const CODE_SIDE: &str = "[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\n\n\
    [target.'cfg(unix)'.dependencies]\nlibc = { version = \"0.2\" }\n\n";

/// A member manifest taking `goodfetch-dev-workspace-hack` in `[dev-dependencies]`.
const MEMBER_WITH_DEV_HACK: &str = "[package]\nname = \"goodfetch-sample\"\nversion = \"0.0.0\"\n\n\
    [dependencies]\ngoodfetch-workspace-hack.workspace = true\n\n\
    [dev-dependencies]\ngoodfetch-dev-workspace-hack.workspace = true\n";

/// A workspace holding the repository's `ci/workspace-hack.sh`, its
/// `Cargo.lock`, both workspace-hack manifests and a member crate, all an
/// hour old, and a stand-in `cargo`.
struct HackedWorkspace {
    folder: tempfile::TempDir,
    stand_in: PathBuf,
    dev_manifest_during_generate: PathBuf,
}

impl HackedWorkspace {
    fn new(build_section: &str, dev_section: &str, member_manifest: &str) -> Self {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let files = [
            (
                "Cargo.toml",
                "[workspace]\nmembers = [\"crates/goodfetch-*\"]\n".to_owned(),
            ),
            ("Cargo.lock", "version = 4\n".to_owned()),
            (
                MANIFEST,
                manifest_with("goodfetch-workspace-hack", build_section),
            ),
            ("crates/goodfetch-workspace-hack/src/lib.rs", String::new()),
            (
                DEV_MANIFEST,
                manifest_with("goodfetch-dev-workspace-hack", dev_section),
            ),
            (
                "crates/goodfetch-dev-workspace-hack/src/lib.rs",
                String::new(),
            ),
            (MEMBER_MANIFEST, member_manifest.to_owned()),
            ("crates/goodfetch-sample/src/lib.rs", String::new()),
            (
                PROTOCOL_MANIFEST,
                "[package]\nname = \"goodfetch-protocol\"\nversion = \"0.0.0\"\n".to_owned(),
            ),
            ("crates/goodfetch-protocol/src/lib.rs", String::new()),
        ];
        for (relative, contents) in files {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, contents).unwrap();
        }
        let hour_ago = SystemTime::now() - Duration::from_secs(3600);
        for relative in [
            "Cargo.lock",
            MANIFEST,
            DEV_MANIFEST,
            MEMBER_MANIFEST,
            PROTOCOL_MANIFEST,
        ] {
            std::fs::File::options()
                .write(true)
                .open(root.join(relative))
                .unwrap()
                .set_modified(hour_ago)
                .unwrap();
        }
        std::fs::create_dir_all(root.join("ci")).unwrap();
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/workspace-hack.sh");
        write_executable(
            &root.join("ci/workspace-hack.sh"),
            &std::fs::read_to_string(script).unwrap(),
        );
        let stand_in = root.join("stand-in");
        std::fs::create_dir_all(&stand_in).unwrap();
        std::fs::write(stand_in.join("generated"), GENERATED).unwrap();
        let dev_manifest_during_generate = stand_in.join("dev-manifest-seen");
        // Each write is in place, through a link as `cargo` writes
        // `Cargo.lock`.
        write_executable(
            &stand_in.join("cargo"),
            &format!(
                "#!/bin/sh\n\
                 rewrite() {{ contents=$(cat \"$1\"; echo x); printf '%s' \"${{contents%x}}\" >\"$1\"; }}\n\
                 [ \"$1\" = hakari ] || exit 2\n\
                 rewrite Cargo.lock\n\
                 case \"$2\" in\n\
                 generate)\n\
                   cp {DEV_MANIFEST} \"{seen}\"\n\
                   {{ printf '[package]\\nname = \"goodfetch-workspace-hack\"\\nversion = \"0.0.0\"\\n\\n### BEGIN HAKARI SECTION\\n'\n\
                     cat \"{generated}\"\n\
                     printf '### END HAKARI SECTION\\n'; }} >{MANIFEST}\n\
                   ;;\n\
                 manage-deps) ;;\n\
                 *) exit 2 ;;\n\
                 esac\n",
                generated = stand_in.join("generated").display(),
                seen = dev_manifest_during_generate.display(),
            ),
        );
        Self {
            folder,
            stand_in,
            dev_manifest_during_generate,
        }
    }

    /// The content and modification time of each file a check may not
    /// write.
    fn written(&self) -> Vec<(String, SystemTime)> {
        [
            "Cargo.lock",
            MANIFEST,
            DEV_MANIFEST,
            MEMBER_MANIFEST,
            PROTOCOL_MANIFEST,
        ]
        .iter()
        .map(|relative| {
            let path = self.folder.path().join(relative);
            let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
            (std::fs::read_to_string(&path).unwrap(), modified)
        })
        .collect()
    }

    /// Runs `ci/workspace-hack.sh check`, with the stand-in first on `PATH`.
    fn check(&self) -> std::process::Output {
        let path = std::env::join_paths(
            std::iter::once(self.stand_in.clone())
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        Command::new(self.folder.path().join("ci/workspace-hack.sh"))
            .arg("check")
            .current_dir(self.folder.path())
            .env("PATH", path)
            .output()
            .unwrap()
    }
}

#[test]
fn a_check_of_a_current_workspace_hack_passes_and_writes_nothing_in_the_tree() {
    let workspace = HackedWorkspace::new(BUILD_SIDE, CODE_SIDE, MEMBER_WITH_DEV_HACK);
    let before = workspace.written();

    let check = workspace.check();

    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert_eq!(workspace.written(), before, "the check wrote the tree");
    let seen = std::fs::read_to_string(&workspace.dev_manifest_during_generate).unwrap();
    assert_eq!(
        seen,
        manifest_with("goodfetch-dev-workspace-hack", ""),
        "the check must clear the dev workspace-hack section in the mirror before cargo hakari generate"
    );
}

#[test]
fn a_check_of_an_outdated_workspace_hack_fails_naming_generate_and_writes_nothing_in_the_tree() {
    let workspace = HackedWorkspace::new("", CODE_SIDE, MEMBER_WITH_DEV_HACK);
    let before = workspace.written();

    let check = workspace.check();

    assert_eq!(check.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(
        stderr.contains("run ci/workspace-hack.sh generate"),
        "{stderr}"
    );
    assert_eq!(workspace.written(), before, "the check wrote the tree");
}

#[test]
fn a_check_of_an_outdated_dev_workspace_hack_fails_naming_generate_and_writes_nothing_in_the_tree()
{
    let workspace = HackedWorkspace::new(BUILD_SIDE, "", MEMBER_WITH_DEV_HACK);
    let before = workspace.written();

    let check = workspace.check();

    assert_eq!(check.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(
        stderr.contains("goodfetch-dev-workspace-hack")
            && stderr.contains("run ci/workspace-hack.sh generate"),
        "{stderr}"
    );
    assert_eq!(workspace.written(), before, "the check wrote the tree");
}

#[test]
fn a_check_of_a_member_missing_the_dev_workspace_hack_fails_naming_generate() {
    let missing = "[package]\nname = \"goodfetch-sample\"\nversion = \"0.0.0\"\n\n\
        [dependencies]\ngoodfetch-workspace-hack.workspace = true\n";
    let workspace = HackedWorkspace::new(BUILD_SIDE, CODE_SIDE, missing);
    let before = workspace.written();

    let check = workspace.check();

    assert_eq!(check.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(
        stderr.contains("goodfetch-dev-workspace-hack")
            && stderr.contains("run ci/workspace-hack.sh generate"),
        "{stderr}"
    );
    assert_eq!(workspace.written(), before, "the check wrote the tree");
}

#[test]
fn a_check_of_a_member_taking_the_dev_workspace_hack_in_dependencies_fails() {
    let wrong_section = "[package]\nname = \"goodfetch-sample\"\nversion = \"0.0.0\"\n\n\
        [dependencies]\ngoodfetch-workspace-hack.workspace = true\n\
        goodfetch-dev-workspace-hack.workspace = true\n\n\
        [dev-dependencies]\ngoodfetch-dev-workspace-hack.workspace = true\n";
    let workspace = HackedWorkspace::new(BUILD_SIDE, CODE_SIDE, wrong_section);
    let before = workspace.written();

    let check = workspace.check();

    assert_eq!(check.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(stderr.contains("goodfetch-dev-workspace-hack"), "{stderr}");
    assert_eq!(workspace.written(), before, "the check wrote the tree");
}
