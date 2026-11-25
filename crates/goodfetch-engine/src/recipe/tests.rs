#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::fs;
use std::path::Path;

use rstest::rstest;
use tempfile::TempDir;

use super::{manifest, package};

const PROGRAM: &str = r#"
version = "0.1"
name = "price-watch"
product = "collie"
entry.program = "src/main.ts"
runtime = { language = "node", version = "24" }
"#;

/// A package's folder holding `files`, each a path and its text.
fn folder(files: &[(&str, &str)]) -> TempDir {
    let root = TempDir::new().unwrap();
    for (path, text) in files {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    root
}

fn refused_field(root: &Path) -> (String, String) {
    let refused = package(root).unwrap_err();
    let field = refused.details().unwrap()["field"]
        .as_str()
        .unwrap()
        .to_owned();
    (refused.code().to_owned(), field)
}

/// A package holds its manifest, source and lockfiles, less what its
/// `.goodfetchignore` leaves out and what a package always does: version
/// control, secrets files, dependency folders and build output.
///
/// Covers: format:recipe-manifest
#[test]
fn a_package_holds_its_source_and_lockfiles_and_leaves_out_ignored_files() {
    let root = folder(&[
        ("goodfetch-recipe.toml", PROGRAM),
        ("src/main.ts", ""),
        ("src/lib/parse.ts", ""),
        ("package.json", "{}"),
        ("pnpm-lock.yaml", ""),
        (".github/workflows/check.yml", ""),
        (".goodfetchignore", "notes/\n*.log\n"),
        ("notes/draft.md", ""),
        ("debug.log", ""),
        (".env", "TOKEN=secret"),
        (".env.local", "TOKEN=secret"),
        ("src/.env", "TOKEN=secret"),
        (".git/config", ""),
        ("node_modules/left-pad/index.js", ""),
        (".venv/bin/python", ""),
        ("__pycache__/main.pyc", ""),
        ("target/debug/price-watch", ""),
        ("dist/main.js", ""),
        ("build/main.js", ""),
    ]);

    let package = package(root.path()).unwrap();

    assert_eq!(
        package.files,
        [
            ".github/workflows/check.yml",
            ".goodfetchignore",
            "goodfetch-recipe.toml",
            "package.json",
            "pnpm-lock.yaml",
            "src/lib/parse.ts",
            "src/main.ts",
        ]
    );
    assert_eq!(package.manifest.name, "price-watch");
}

/// An ignore file's negation brings back nothing a package always leaves
/// out.
#[test]
fn an_ignore_file_cannot_bring_back_a_secrets_file() {
    let root = folder(&[
        ("goodfetch-recipe.toml", PROGRAM),
        ("src/main.ts", ""),
        (".goodfetchignore", "!.env\n!node_modules/\n"),
        (".env", "TOKEN=secret"),
        ("node_modules/left-pad/index.js", ""),
    ]);

    assert_eq!(
        package(root.path()).unwrap().files,
        [".goodfetchignore", "goodfetch-recipe.toml", "src/main.ts"]
    );
}

/// Covers: error:recipe_invalid
#[rstest]
#[case::no_manifest(&[("src/main.ts", "")], "goodfetch-recipe.toml")]
#[case::entry_missing(&[("goodfetch-recipe.toml", PROGRAM)], "entry.program")]
#[case::entry_ignored(
    &[("goodfetch-recipe.toml", PROGRAM), ("src/main.ts", ""), (".goodfetchignore", "src/\n")],
    "entry.program"
)]
#[case::manifest_ignored(
    &[("goodfetch-recipe.toml", PROGRAM), ("src/main.ts", ""), (".goodfetchignore", "*.toml\n")],
    ".goodfetchignore"
)]
#[case::not_toml(&[("goodfetch-recipe.toml", "name = ")], "goodfetch-recipe.toml")]
fn a_package_that_is_not_valid_is_refused_naming_its_field(
    #[case] files: &[(&str, &str)],
    #[case] field: &str,
) {
    let root = folder(files);

    assert_eq!(
        refused_field(root.path()),
        ("recipe_invalid".to_owned(), field.to_owned())
    );
}

/// A symbolic link could lead out of the package, so a package holds none.
///
/// Covers: error:recipe_invalid
#[cfg(unix)]
#[test]
fn a_symbolic_link_in_the_package_is_refused_naming_its_path() {
    let root = folder(&[("goodfetch-recipe.toml", PROGRAM), ("src/main.ts", "")]);
    std::os::unix::fs::symlink("/etc/hostname", root.path().join("src/host")).unwrap();

    assert_eq!(
        refused_field(root.path()),
        ("recipe_invalid".to_owned(), "src/host".to_owned())
    );
}

/// A manifest of a newer version is refused for its version under ADR
/// 0030's default rule, before any field this engine does not know.
///
/// Covers: format:recipe-manifest, error:format_version_unsupported
#[rstest]
#[case::newer_minor("0.2")]
#[case::newer_major("1.0")]
fn a_newer_manifest_is_refused_for_its_version(#[case] version: &str) {
    let refused = manifest(&format!(
        "version = \"{version}\"\nname = \"r\"\nproduct = \"collie\"\nentry.steps = \"s.json\"\nsample = {{ pages = 3 }}\n"
    ))
    .unwrap_err();

    assert_eq!(refused.code(), "format_version_unsupported");
    let details = refused.details().unwrap();
    assert_eq!(
        (details["format"].as_str(), details["found"].as_str()),
        (Some("recipe-manifest"), Some(version))
    );
}
