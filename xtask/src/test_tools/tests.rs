use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{dep_info_inputs, shipped_target_of, sources, stamps};

#[test]
fn a_dep_info_files_prerequisites_are_its_paths_with_their_spaces() {
    let text = "/work/target/debug/tool: /work/src/main.rs /work/my\\ pages/a.html\n";

    let inputs = dep_info_inputs(text);

    assert_eq!(
        inputs,
        [
            PathBuf::from("/work/src/main.rs"),
            PathBuf::from("/work/my pages/a.html")
        ]
    );
}

/// Git's files change at a commit or at another worktree's fetch, and the
/// target folder's at any build: neither is a source.
#[test]
fn the_sources_are_the_workspaces_own_files_and_the_manifests_of_their_packages() {
    let prerequisites = [
        "/work/crates/engine/src/lib.rs",
        "/work/crates/engine/../../pages/a.toml",
        "/work/target/debug/build/rules/out/table.rs",
        "/work/.git/HEAD",
        "/elsewhere/.git/worktrees/work/HEAD",
        "/elsewhere/registry/serde/src/lib.rs",
    ]
    .map(PathBuf::from);
    let packages = ["/work/crates/engine/Cargo.toml", "/work/xtask/Cargo.toml"].map(PathBuf::from);

    let sources = sources(
        &prerequisites,
        Path::new("/work"),
        Path::new("/work/target"),
        &packages,
    );

    let sources: Vec<&str> = sources.iter().map(|path| path.to_str().unwrap()).collect();
    assert_eq!(
        sources,
        [
            "crates/engine/../../pages/a.toml",
            "crates/engine/Cargo.toml",
            "crates/engine/src/lib.rs"
        ]
    );
}

/// Cargo may have compiled such a source as it was before the change, so a
/// record of it as it is now would pass tools older than it.
#[test]
fn a_source_changed_while_the_tools_were_building_fails_the_build() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("lib.rs"), "").unwrap();
    let sources = BTreeSet::from([PathBuf::from("lib.rs")]);
    let started = SystemTime::now() - Duration::from_secs(60);

    let stamps = stamps(sources, folder.path(), started);

    let error = stamps.unwrap_err().to_string();
    assert!(
        error.contains("lib.rs changed while the test tools were building"),
        "{error}"
    );
}

/// A build that resolves anew writes the lockfile itself, then builds from
/// it.
#[test]
fn a_lockfile_written_while_the_tools_were_building_is_recorded() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("Cargo.lock"), "").unwrap();
    let sources = BTreeSet::from([PathBuf::from("Cargo.lock")]);
    let started = SystemTime::now() - Duration::from_secs(60);

    let stamps = stamps(sources, folder.path(), started);

    assert_eq!(stamps.unwrap().len(), 1);
}

#[test]
fn a_source_as_it_was_when_the_build_began_is_recorded_with_its_size() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("lib.rs"), "four").unwrap();
    let sources = BTreeSet::from([PathBuf::from("lib.rs")]);
    let started = SystemTime::now() + Duration::from_secs(60);

    let stamps = stamps(sources, folder.path(), started).unwrap();

    assert_eq!(stamps.len(), 1);
    assert_eq!((&stamps[0].0, stamps[0].1), (&PathBuf::from("lib.rs"), 4));
}

#[test]
fn linux_ships_the_static_musl_build_of_its_architecture() {
    assert_eq!(
        shipped_target_of("linux", "x86_64").as_deref(),
        Some("x86_64-unknown-linux-musl")
    );
    assert_eq!(
        shipped_target_of("linux", "aarch64").as_deref(),
        Some("aarch64-unknown-linux-musl")
    );
}

#[test]
fn macos_and_windows_ship_the_hosts_own_build() {
    assert_eq!(shipped_target_of("macos", "aarch64"), None);
    assert_eq!(shipped_target_of("windows", "x86_64"), None);
}
