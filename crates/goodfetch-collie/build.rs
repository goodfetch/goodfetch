//! Embeds Collie's Console build (ADR 0057): writes the table of the
//! brotli files `xtask console build` put in `console-build/`, which
//! Collie's executable includes and serves as they are. Without that
//! build the table is a compile error naming the command, so building the
//! executable fails and no executable ships without its Console; the
//! library, which an SDK crate links, includes no table and builds.
#![expect(
    clippy::disallowed_methods,
    reason = "a build script is synchronous and reads its own environment and files"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The folder `xtask console build` writes, beside this script.
const BUILD: &str = "console-build";

fn main() {
    let manifest = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("invariant: Cargo sets CARGO_MANIFEST_DIR"),
    );
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("invariant: Cargo sets OUT_DIR"));
    let folder = manifest.join(BUILD);
    // A folder that does not exist reruns the script on every build, until
    // the Console is built.
    println!("cargo::rerun-if-changed={BUILD}");
    let mut files = Vec::new();
    if folder.is_dir() {
        walk(&folder, &mut files);
    }
    files.sort();
    let table = if files.iter().any(|file| file == Path::new("index.html.br")) {
        table(&folder, &files)
    } else {
        "compile_error!(\"Collie's Console build is missing from crates/goodfetch-collie/console-build/: run `cargo run -p xtask -- console build` (xtask console build) before building Collie's executable (ADR 0057).\");\n".to_owned()
    };
    std::fs::write(out.join("console_build.rs"), table)
        .expect("invariant: a build script can write its OUT_DIR");
}

/// The table of `files`, each a path under `folder` ending in `.br`, served
/// at its path without that suffix.
fn table(folder: &Path, files: &[PathBuf]) -> String {
    let mut table = format!(
        "static FILES: [goodfetch_engine::console::ConsoleFile; {}] = [\n",
        files.len()
    );
    for file in files {
        let served = file.to_string_lossy().replace('\\', "/");
        let served = served.strip_suffix(".br").unwrap_or(&served);
        let _ = writeln!(
            table,
            "    goodfetch_engine::console::ConsoleFile::new({:?}, include_bytes!({:?})),",
            format!("/{served}"),
            folder.join(file).to_string_lossy(),
        );
    }
    table.push_str("];\n\n/// Collie's Console build, as `xtask console build` wrote it.\npub(crate) static CONSOLE: goodfetch_engine::console::ConsoleBuild =\n    goodfetch_engine::console::ConsoleBuild::new(&FILES);\n");
    table
}

/// Every file under `folder`, by its path from it.
fn walk(folder: &Path, files: &mut Vec<PathBuf>) {
    walk_from(folder, Path::new(""), files);
}

fn walk_from(folder: &Path, relative: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = relative.join(entry.file_name());
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            walk_from(&entry.path(), &path, files);
        } else {
            files.push(path);
        }
    }
}
