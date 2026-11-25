#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::fs;
use std::path::Path;

use rstest::rstest;
use serde_json::Value;
use tempfile::TempDir;

use super::{digest, manifest, package};
use crate::pinned_download::Digest;

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

/// The digest's test vectors, which the conformance suite publishes for a
/// host that recomputes a Recipe's digest.
const VECTORS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../conformance/recipe-digest/vectors.json"
));

/// A vector: its name, its files, each a path and its text, and the digest
/// it gives or the path it is refused for.
type Vector = (String, Vec<(String, String)>, Result<String, String>);

/// Each vector.
fn vectors() -> Vec<Vector> {
    let vectors: Value = serde_json::from_str(VECTORS).unwrap();
    let text = |value: &Value| value.as_str().unwrap().to_owned();
    vectors["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|vector| {
            let files = vector["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|file| (text(&file["path"]), text(&file["text"])))
                .collect();
            let expected = match vector.get("digest") {
                Some(found) => Ok(text(found)),
                None => Err(text(&vector["refused"])),
            };
            (text(&vector["name"]), files, expected)
        })
        .collect()
}

/// The digest of a package on disk holding `files`, each a path and its
/// text.
fn digest_on_disk(files: &[(&str, &str)]) -> String {
    package(folder(files).path()).unwrap().digest.to_string()
}

/// Every published vector gives its digest, or is refused naming its path,
/// from the files alone, on every operating system.
///
/// Covers: error:recipe_invalid
#[test]
fn each_test_vector_gives_its_digest_or_is_refused_naming_its_path() {
    let vectors = vectors();

    let found: Vec<(String, Result<String, String>)> = vectors
        .iter()
        .map(|(name, files, _)| {
            let files = files
                .iter()
                .map(|(path, text)| (path.clone(), Digest::of(text.as_bytes())));
            let outcome = digest(files)
                .map(|digest| digest.to_string())
                .map_err(|refused| {
                    assert_eq!(refused.code(), "recipe_invalid", "{name}");
                    refused.details().unwrap()["field"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                });
            (name.clone(), outcome)
        })
        .collect();

    let expected: Vec<(String, Result<String, String>)> = vectors
        .into_iter()
        .map(|(name, _, expected)| (name, expected))
        .collect();
    assert_eq!(found, expected);
}

/// A package read from disk has the digest of its vector.
#[test]
fn a_package_on_disk_has_its_vectors_digest() {
    let vectors: Vec<_> = vectors()
        .into_iter()
        .filter_map(|(name, files, expected)| Some((name, files, expected.ok()?)))
        .collect();

    let found: Vec<String> = vectors
        .iter()
        .map(|(_, files, _)| {
            let files: Vec<(&str, &str)> = files
                .iter()
                .map(|(path, text)| (path.as_str(), text.as_str()))
                .collect();
            digest_on_disk(&files)
        })
        .collect();

    let expected: Vec<String> = vectors.into_iter().map(|(_, _, digest)| digest).collect();
    assert_eq!(found, expected);
}

const STEPS: &str = r#"
version = "0.1"
name = "price-watch"
product = "collie"
entry.steps = "steps.json"
"#;

/// The package each case changes.
const PACKAGE: &[(&str, &str)] = &[
    ("goodfetch-recipe.toml", STEPS),
    ("steps.json", "[]\n"),
    ("notes/a.md", ""),
    (".goodfetchignore", "*.log\n"),
];

/// Any change to what the package holds changes its digest; a file its
/// ignore rules leave out changes nothing.
#[rstest]
#[case::a_byte(&[("steps.json", "[ ]\n")], &[], false)]
#[case::a_file_added(&[("notes/b.md", "")], &[], false)]
#[case::a_file_renamed(&[("notes/b.md", "")], &["notes/a.md"], false)]
#[case::the_manifest(&[("goodfetch-recipe.toml", &STEPS[..STEPS.len() - 1])], &[], false)]
#[case::an_ignored_file(&[("debug.log", "trace")], &[], true)]
#[case::a_secrets_file(&[(".env", "TOKEN=secret"), ("node_modules/a/index.js", "")], &[], true)]
fn a_change_to_the_package_changes_its_digest_and_an_ignored_file_does_not(
    #[case] written: &[(&str, &str)],
    #[case] removed: &[&str],
    #[case] same: bool,
) {
    let before = digest_on_disk(PACKAGE);
    let mut changed: Vec<(&str, &str)> = PACKAGE
        .iter()
        .copied()
        .filter(|(path, _)| !removed.contains(path) && written.iter().all(|(new, _)| new != path))
        .collect();
    changed.extend(written.iter().map(|(path, text)| (*path, *text)));

    assert_eq!(digest_on_disk(&changed) == before, same);
}

/// A package read from disk with no portable digest is refused naming the
/// path: a backslash in a name, and two names differing only in case,
/// which a case-sensitive file system alone can hold.
///
/// Covers: error:recipe_invalid
#[cfg(target_os = "linux")]
#[rstest]
#[case::a_backslash(&[("src\\main.ts", "")], "src\\main.ts")]
#[case::case_only(&[("Notes.md", ""), ("notes.md", "")], "notes.md")]
fn a_package_on_disk_with_no_portable_digest_is_refused_naming_the_path(
    #[case] files: &[(&str, &str)],
    #[case] field: &str,
) {
    let mut files = files.to_vec();
    files.extend([("goodfetch-recipe.toml", STEPS), ("steps.json", "[]\n")]);
    let root = folder(&files);

    assert_eq!(
        refused_field(root.path()),
        ("recipe_invalid".to_owned(), field.to_owned())
    );
}
