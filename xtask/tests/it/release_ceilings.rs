//! Tests for release-profile debug symbols, size-manifest generation, and
//! pre-publish Size budget ceilings for both Products (ADRs 0057, 0074, 0087
//! and 0094; issue 203).

use std::fs::File;
use std::path::Path;

use crate::workspace::Workspace;

const MB: u64 = 1_000_000;
const MIB: u64 = 1_048_576;

fn write_sized(root: &Path, relative: &str, bytes: u64) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create fixture parent");
    }
    let file = File::create(&path).expect("create fixture file");
    file.set_len(bytes).expect("set fixture file length");
}

fn seed_release_outputs(workspace: &Workspace, collie_archive: u64, foxhound_archive: u64) {
    let root = workspace.path();
    // Both Products' release executables and archives for Linux x64 musl.
    write_sized(
        root,
        "bins/x86_64-unknown-linux-musl/goodfetch-collie",
        40 * MB,
    );
    write_sized(
        root,
        "bins/x86_64-unknown-linux-musl/goodfetch-foxhound",
        55 * MB,
    );
    write_sized(
        root,
        "packaged/assets/goodfetch-collie-x86_64-unknown-linux-musl.tar.xz",
        collie_archive,
    );
    write_sized(
        root,
        "packaged/assets/goodfetch-foxhound-x86_64-unknown-linux-musl.tar.xz",
        foxhound_archive,
    );
    // Split debug symbols and Goodfetch Chromium archive — deliberately huge to
    // prove neither is measured against any release size ceiling (ADRs 0074, 0094).
    write_sized(
        root,
        "packaged/assets/goodfetch-collie-x86_64-unknown-linux-musl-debug.tar.xz",
        250 * MB,
    );
    write_sized(
        root,
        "packaged/assets/goodfetch-foxhound-x86_64-unknown-linux-musl-debug.tar.xz",
        250 * MB,
    );
    write_sized(
        root,
        "packaged/assets/goodfetch-chromium-linux-x64.tar.xz",
        300 * MB,
    );
    // Platform wheels and crate tarballs within ceilings.
    write_sized(
        root,
        "packaged/wheels/goodfetch_collie-0.1.0-py3-none-manylinux_2_17_x86_64.musllinux_1_2_x86_64.whl",
        20 * MB,
    );
    write_sized(
        root,
        "packaged/wheels/goodfetch_foxhound-0.1.0-py3-none-manylinux_2_17_x86_64.musllinux_1_2_x86_64.whl",
        25 * MB,
    );
    write_sized(
        root,
        "packaged/crates/goodfetch-collie-0.1.0.crate",
        2 * MIB,
    );
    write_sized(
        root,
        "packaged/crates/goodfetch-foxhound-0.1.0.crate",
        3 * MIB,
    );
}

/// Covers: story:1-17
/// Release-profile ceilings pass when both Products fit their own ceilings,
/// excluding Goodfetch Chromium and split debug symbols, and render a Markdown
/// size table naming both Products' files for release notes.
#[test]
fn ceilings_pass_within_budget_and_exclude_chromium_and_debug_symbols() {
    let workspace = Workspace::from_fixture("release");
    seed_release_outputs(&workspace, 49 * MB, 59 * MB);

    let manifest = workspace.xtask(&[
        "release",
        "size-manifest",
        "--binaries",
        "bins",
        "--packaged",
        "packaged",
        "--out",
        "size-files.toml",
    ]);
    assert_eq!(manifest.code, 0, "{}", manifest.stderr);

    let manifest_text = workspace.read_text("size-files.toml");
    assert!(
        !manifest_text.contains("goodfetch-chromium"),
        "Goodfetch Chromium must never enter ceilings manifest:\n{manifest_text}"
    );
    assert!(
        !manifest_text.contains("-debug.tar.xz"),
        "split debug symbols must never enter ceilings manifest:\n{manifest_text}"
    );

    let ceilings = workspace.xtask(&["size", "ceilings", "--files", "size-files.toml"]);
    assert_eq!(
        ceilings.code, 0,
        "stdout:\n{}\nstderr:\n{}",
        ceilings.stdout, ceilings.stderr
    );
    assert!(
        ceilings.stdout.contains("| Collie |"),
        "size table must list Collie:\n{}",
        ceilings.stdout
    );
    assert!(
        ceilings.stdout.contains("| Foxhound |"),
        "size table must list Foxhound:\n{}",
        ceilings.stdout
    );
    assert!(
        ceilings
            .stdout
            .contains("goodfetch-collie-x86_64-unknown-linux-musl.tar.xz")
            && ceilings
                .stdout
                .contains("goodfetch-foxhound-x86_64-unknown-linux-musl.tar.xz"),
        "size table must list both Products' release archives:\n{}",
        ceilings.stdout
    );
}

/// Covers: story:1-17
/// A file of either Product over its own ceiling (Collie > 50 MB compressed,
/// Foxhound > 60 MB compressed, or a crate > 8 MiB) fails `xtask size ceilings`
/// so neither Product publishes.
#[test]
fn broken_ceiling_of_either_product_stops_release_before_publish() {
    // 1. Collie compressed archive over 50 MB (51 MB), Foxhound well within 60 MB.
    let collie_over = Workspace::from_fixture("release");
    seed_release_outputs(&collie_over, 51 * MB, 40 * MB);
    assert_eq!(
        collie_over
            .xtask(&[
                "release",
                "size-manifest",
                "--binaries",
                "bins",
                "--packaged",
                "packaged",
                "--out",
                "size-files.toml",
            ])
            .code,
        0
    );
    let collie_check = collie_over.xtask(&["size", "ceilings", "--files", "size-files.toml"]);
    assert_ne!(
        collie_check.code, 0,
        "Collie over 50 MB must fail ceilings:\n{}",
        collie_check.stdout
    );
    assert!(
        collie_check.stdout.contains("over its ceiling"),
        "{}",
        collie_check.stdout
    );

    // 2. Foxhound compressed archive over 60 MB (61 MB), Collie well within 50 MB.
    let foxhound_over = Workspace::from_fixture("release");
    seed_release_outputs(&foxhound_over, 30 * MB, 61 * MB);
    assert_eq!(
        foxhound_over
            .xtask(&[
                "release",
                "size-manifest",
                "--binaries",
                "bins",
                "--packaged",
                "packaged",
                "--out",
                "size-files.toml",
            ])
            .code,
        0
    );
    let foxhound_check = foxhound_over.xtask(&["size", "ceilings", "--files", "size-files.toml"]);
    assert_ne!(
        foxhound_check.code, 0,
        "Foxhound over 60 MB must fail ceilings:\n{}",
        foxhound_check.stdout
    );
    assert!(
        foxhound_check.stdout.contains("over its ceiling"),
        "{}",
        foxhound_check.stdout
    );

    // 3. Published crate tarball over 8 MiB fails ceilings.
    let crate_over = Workspace::from_fixture("release");
    seed_release_outputs(&crate_over, 30 * MB, 40 * MB);
    write_sized(
        crate_over.path(),
        "packaged/crates/goodfetch-collie-0.1.0.crate",
        9 * MIB,
    );
    assert_eq!(
        crate_over
            .xtask(&[
                "release",
                "size-manifest",
                "--binaries",
                "bins",
                "--packaged",
                "packaged",
                "--out",
                "size-files.toml",
            ])
            .code,
        0
    );
    let crate_check = crate_over.xtask(&["size", "ceilings", "--files", "size-files.toml"]);
    assert_ne!(
        crate_check.code, 0,
        "crate over 8 MiB must fail ceilings:\n{}",
        crate_check.stdout
    );
}

/// Covers: story:1-18
/// Split debug symbols for both executables are signed and attested in
/// rehearsal, tampering breaks verification, and the nightly workflow attests
/// debug symbols, runs ceilings before any publish job, and puts the size table
/// into the pre-release notes.
#[test]
fn debug_symbols_are_attested_in_rehearsal_and_nightly_publishes_size_table() {
    let workspace = Workspace::from_fixture("release");
    workspace.write(
        "packaged/assets/goodfetch-collie-x86_64-unknown-linux-musl-debug.tar.xz",
        b"fake-collie-dwp-archive",
    );
    workspace.write(
        "packaged/assets/goodfetch-foxhound-x86_64-unknown-linux-musl-debug.tar.xz",
        b"fake-foxhound-dwp-archive",
    );

    for product in ["collie", "foxhound"] {
        let file =
            format!("packaged/assets/goodfetch-{product}-x86_64-unknown-linux-musl-debug.tar.xz");
        let signed = workspace.xtask(&[
            "release",
            "sign-rehearsal",
            "--file",
            &file,
            "--key-dir",
            "keys",
        ]);
        assert_eq!(signed.code, 0, "{}", signed.stderr);

        let verified = workspace.xtask(&[
            "release",
            "verify-rehearsal",
            "--file",
            &file,
            "--signature",
            &format!("{file}.sig"),
            "--attestation",
            &format!("{file}.sigstore.jsonl"),
            "--public-key",
            "keys/rehearsal.pub",
        ]);
        assert_eq!(verified.code, 0, "{}", verified.stderr);
    }

    // Tampering with debug symbols fails rehearsal attestation verification.
    workspace.write(
        "packaged/assets/goodfetch-collie-x86_64-unknown-linux-musl-debug.tar.xz",
        b"tampered-collie-dwp-archive",
    );
    let tampered = workspace.xtask(&[
        "release",
        "verify-rehearsal",
        "--file",
        "packaged/assets/goodfetch-collie-x86_64-unknown-linux-musl-debug.tar.xz",
        "--signature",
        "packaged/assets/goodfetch-collie-x86_64-unknown-linux-musl-debug.tar.xz.sig",
        "--attestation",
        "packaged/assets/goodfetch-collie-x86_64-unknown-linux-musl-debug.tar.xz.sigstore.jsonl",
        "--public-key",
        "keys/rehearsal.pub",
    ]);
    assert_ne!(tampered.code, 0, "tampered debug symbols must fail verify");

    // Verify nightly workflow & local rehearsal script wire ceilings before publish
    // and attach the size table to pre-release notes.
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let nightly = std::fs::read_to_string(repo_root.join(".github/workflows/release-nightly.yml"))
        .expect("read release-nightly.yml");
    let attest_pos = nightly.find("attest-and-sign:").expect("attest-and-sign");
    let ceilings_pos = nightly.find("size ceilings").expect("size ceilings");
    let publish_npm_pos = nightly.find("publish-npm:").expect("publish-npm");
    let publish_oci_pos = nightly.find("publish-oci:").expect("publish-oci");
    let publish_gh_pos = nightly
        .find("publish-github-prerelease:")
        .expect("publish-github-prerelease");
    assert!(
        attest_pos < ceilings_pos
            && ceilings_pos < publish_npm_pos
            && ceilings_pos < publish_oci_pos
            && ceilings_pos < publish_gh_pos,
        "ceilings must run after attest-and-sign and before every publish job"
    );
    assert!(
        nightly.contains("release-size-table.md")
            && nightly.contains("--notes-file target/release-notes.md"),
        "nightly pre-release must carry the size table in its notes"
    );

    let rehearse = std::fs::read_to_string(repo_root.join("ci/release/rehearse.sh"))
        .expect("read rehearse.sh");
    assert!(
        rehearse.contains("-debug.tar.xz") && rehearse.contains("size ceilings"),
        "local rehearsal must verify debug symbols and enforce ceilings before install/publish"
    );
}
