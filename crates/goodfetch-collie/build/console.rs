//! The table of a Console build (ADR 0057), shared by the build scripts of
//! Collie's and Foxhound's executables: the brotli files `xtask console
//! build` put in a folder, which the executable includes and serves as they
//! are, and the routes of its screens, `routes.json` beside them, at which
//! it serves the page. Without that build the table is a compile error
//! naming the command, so building the executable fails and no executable
//! ships without its Console; the library, which an SDK crate links,
//! includes no table and builds.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The routes of the build's screens, in its folder.
const ROUTES: &str = "routes.json";

/// Writes `console_build.rs` into `OUT_DIR`: the table of the build in
/// `build`, a folder relative to the package's manifest, as the static
/// `CONSOLE`, or a compile error that names `build` and the command.
pub(crate) fn embed(build: &str) {
    let manifest = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("invariant: Cargo sets CARGO_MANIFEST_DIR"),
    );
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("invariant: Cargo sets OUT_DIR"));
    let folder = manifest.join(build);
    // A folder that does not exist reruns the script on every build, until
    // the Console is built.
    println!("cargo::rerun-if-changed={build}");
    let mut files = Vec::new();
    if folder.is_dir() {
        walk(&folder, &mut files);
    }
    files.retain(|file| file.extension().is_some_and(|extension| extension == "br"));
    files.sort();
    let table = if files.iter().any(|file| file == Path::new("index.html.br")) {
        table(&folder, &files, &routes(&folder))
    } else {
        format!(
            "compile_error!(\"The Console build is missing from {}: run `cargo run -p xtask -- console build` (xtask console build) before building the executable (ADR 0057).\");\n",
            folder.display()
        )
    };
    std::fs::write(out.join("console_build.rs"), table)
        .expect("invariant: a build script can write its OUT_DIR");
}

/// The routes `routes.json` in `folder` lists, `{ "routes": ["/settings", …] }`
/// as the Console's package commits it; none when the build has no such
/// file.
fn routes(folder: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(folder.join(ROUTES)) else {
        return Vec::new();
    };
    let listed: serde_json::Value = serde_json::from_str(&text)
        .expect("invariant: `xtask console build` copies the package's routes.json as it is");
    listed["routes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|route| route.as_str().map(str::to_owned))
        .collect()
}

/// The table of `files`, each a path under `folder` ending in `.br`, served
/// at its path without that suffix, and of `routes`.
fn table(folder: &Path, files: &[PathBuf], routes: &[String]) -> String {
    let mut table = format!(
        "static FILES: [goodfetch_engine::ConsoleFile; {}] = [\n",
        files.len()
    );
    for file in files {
        let served = file.to_string_lossy().replace('\\', "/");
        let served = served.strip_suffix(".br").unwrap_or(&served);
        let _ = writeln!(
            table,
            "    goodfetch_engine::ConsoleFile::new({:?}, include_bytes!({:?})),",
            format!("/{served}"),
            folder.join(file).to_string_lossy(),
        );
    }
    let _ = writeln!(
        table,
        "];\n\nstatic ROUTES: [&str; {}] = {routes:?};\n",
        routes.len()
    );
    table.push_str("/// The Console build, as `xtask console build` wrote it.\npub(crate) static CONSOLE: goodfetch_engine::ConsoleBuild =\n    goodfetch_engine::ConsoleBuild::new(&FILES, &ROUTES);\n");
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
