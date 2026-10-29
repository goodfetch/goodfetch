//! `xtask console build` (ADRs 0057 and 0068): Collie's Console build, run
//! through its package's `build` script into a folder of its own, then
//! every file it wrote compressed with brotli into the folder Collie's
//! executable embeds, which holds brotli files only. A build whose inputs
//! did not change since the last one runs nothing.

use std::io::Read as _;

use crate::workspace::Workspace;

/// The folder Collie's executable embeds.
const BUILD: &str = "crates/goodfetch-collie/console-build";

fn decompressed(workspace: &Workspace, relative: &str) -> Vec<u8> {
    let compressed = workspace.read(relative).unwrap();
    let mut text = Vec::new();
    brotli::Decompressor::new(compressed.as_slice(), 4096)
        .read_to_end(&mut text)
        .unwrap();
    text
}

#[test]
fn the_console_build_holds_each_built_file_compressed_with_brotli_and_nothing_else() {
    let workspace = Workspace::from_fixture("console");

    let outcome = workspace.xtask(&["console", "build"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let mut files: Vec<String> = walk(&workspace.path().join(BUILD));
    files.sort();
    assert_eq!(
        files,
        [
            "assets/font-3c4d.woff2.br",
            "assets/index-1a2b.js.br",
            "index.html.br",
        ]
    );
    assert_eq!(
        decompressed(&workspace, &format!("{BUILD}/index.html.br")),
        b"<!doctype html><title>Goodfetch Collie</title>\n"
    );
    assert_eq!(
        decompressed(&workspace, &format!("{BUILD}/assets/font-3c4d.woff2.br")),
        [0, 1, 2, 3, 255, 254, 253]
    );
}

#[test]
fn a_console_build_whose_inputs_did_not_change_runs_nothing() {
    let workspace = Workspace::from_fixture("console");
    assert_eq!(workspace.xtask(&["console", "build"]).code, 0);

    let outcome = workspace.xtask(&["console", "build"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        workspace.read_text("target/console-build-runs.txt"),
        "run\n"
    );
}

#[test]
fn a_changed_input_builds_the_console_again() {
    let workspace = Workspace::from_fixture("console");
    assert_eq!(workspace.xtask(&["console", "build"]).code, 0);
    workspace.append("console/collie/build.mjs", "// changed\n");

    let outcome = workspace.xtask(&["console", "build"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        workspace.read_text("target/console-build-runs.txt"),
        "run\nrun\n"
    );
}

/// The Windows cell's `target` is a junction to its Dev Drive, another
/// volume than the checkout, as a `target` linked to Linux's `/dev/shm`, its
/// own mount, is here: a rename from one to the other fails.
#[cfg(target_os = "linux")]
#[test]
fn a_console_build_whose_target_folder_is_on_another_volume_writes_the_build() {
    use std::os::unix::fs::MetadataExt as _;
    let workspace = Workspace::from_fixture("console");
    let elsewhere = tempfile::tempdir_in("/dev/shm").unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), workspace.path().join("target")).unwrap();
    assert_ne!(
        std::fs::metadata(elsewhere.path()).unwrap().dev(),
        std::fs::metadata(workspace.path()).unwrap().dev(),
        "/dev/shm is no other volume than the workspace's here"
    );

    let outcome = workspace.xtask(&["console", "build"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        decompressed(&workspace, &format!("{BUILD}/index.html.br")),
        b"<!doctype html><title>Goodfetch Collie</title>\n"
    );
}

/// Every file under `folder`, by its path from it.
fn walk(folder: &std::path::Path) -> Vec<String> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            files.extend(walk(&path).into_iter().map(|file| format!("{name}/{file}")));
        } else {
            files.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    files
}
