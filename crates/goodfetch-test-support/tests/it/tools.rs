//! The test tool manifest that `just test` writes (ADR 0094): a test finds
//! the commit's executables through it, and fails when it or an entry is
//! missing, when a source of the tools changed since they were built, or
//! when they were built from another tree.

use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_test_support::{TestEnv, Tool, Tools, ToolsError, tools};
use rstest::rstest;
use sha2::{Digest as _, Sha256};

use crate::TEST_ENV;

#[test]
fn an_unset_manifest_fails() {
    let read = Tools::read(None);

    let error = read.unwrap_err();
    assert!(matches!(error, ToolsError::Unset), "{error}");
}

#[test]
fn a_missing_manifest_fails() {
    let env = TestEnv::new();

    let read = Tools::read(Some(&env.folder().join("tools.json")));

    let error = read.unwrap_err();
    assert!(matches!(error, ToolsError::Unreadable { .. }), "{error}");
}

#[rstest]
#[case::collie(Tool::Collie)]
#[case::foxhound(Tool::Foxhound)]
#[case::proving_ground(Tool::ProvingGround)]
#[case::conformance(Tool::Conformance)]
#[case::stub_gateway(Tool::StubGateway)]
fn a_missing_entry_fails(#[case] tool: Tool) {
    let env = TestEnv::new();
    let manifest = env.folder().join("tools.json");
    std::fs::write(&manifest, r#"{"test_env": "/bin/goodfetch-test-env"}"#).unwrap();
    let tools = Tools::read(Some(&manifest)).unwrap();

    let path = tools.path(tool);

    let error = path.unwrap_err();
    assert!(
        matches!(error, ToolsError::Missing(missing) if missing == tool),
        "{error}"
    );
}

#[test]
fn an_entry_of_the_manifest_is_its_path() {
    let env = TestEnv::new();
    let manifest = env.folder().join("tools.json");
    std::fs::write(
        &manifest,
        r#"{"collie": "/work/target/debug/goodfetch-collie"}"#,
    )
    .unwrap();
    let tools = Tools::read(Some(&manifest)).unwrap();

    let path = tools.path(Tool::Collie);

    assert_eq!(
        path.unwrap(),
        Path::new("/work/target/debug/goodfetch-collie")
    );
}

/// A tree with one source of the tools, and their manifest with its input
/// record as `xtask test-tools build` writes it, the tools just built.
struct BuiltTools {
    tree: PathBuf,
    source: PathBuf,
    manifest: PathBuf,
}

impl BuiltTools {
    fn new(env: &TestEnv) -> Self {
        let tree = env.folder().join("tree");
        let source = tree.join("src/lib.rs");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "pub fn built() {}\n").unwrap();
        let manifest = env.folder().join("tools.json");
        std::fs::write(
            &manifest,
            r#"{"collie": "/work/target/debug/goodfetch-collie"}"#,
        )
        .unwrap();
        let built = Self {
            tree,
            source,
            manifest,
        };
        built.record(&built.tree);
        built
    }

    /// Writes the input record of tools built now from `root`.
    fn record(&self, root: &Path) {
        let modified = std::fs::metadata(&self.source).unwrap().modified().unwrap();
        let nanoseconds = modified.duration_since(UNIX_EPOCH).unwrap().as_nanos();
        self.record_modified(root, u64::try_from(nanoseconds).unwrap());
    }

    /// Writes the input record of tools built from `root` when the source's
    /// modification time read `nanoseconds` since the Unix epoch.
    fn record_modified(&self, root: &Path, nanoseconds: u64) {
        let content = std::fs::read(&self.source).unwrap();
        let digest = data_encoding::HEXLOWER.encode(&Sha256::digest(&content));
        let record = serde_json::json!({
            "root": root,
            "inputs": [["src/lib.rs", content.len(), nanoseconds, digest]],
        });
        std::fs::write(
            self.manifest.with_extension("inputs.json"),
            record.to_string(),
        )
        .unwrap();
    }

    fn read(&self) -> Result<Tools, ToolsError> {
        Tools::read_built_from(Some(&self.manifest), &self.tree)
    }
}

#[test]
fn tools_whose_sources_are_as_they_were_built_from_are_read() {
    let env = TestEnv::new();
    let built = BuiltTools::new(&env);

    let tools = built.read();

    assert_eq!(
        tools.unwrap().path(Tool::Collie).unwrap(),
        Path::new("/work/target/debug/goodfetch-collie")
    );
}

/// `xtask test-tools build` on Linux records a source's time to the
/// nanosecond, and Windows, or Wine running a test built for it, reads it in
/// 100 ns ticks (issue 2767): one time, read through either clock, is the
/// same time.
#[test]
fn tools_whose_sources_time_two_clocks_read_at_different_resolutions_are_read() {
    let env = TestEnv::new();
    let built = BuiltTools::new(&env);
    let source = std::fs::File::options()
        .write(true)
        .open(&built.source)
        .unwrap();
    source
        .set_modified(UNIX_EPOCH + Duration::new(1_800_000_000, 123_456_789))
        .unwrap();
    built.record_modified(&built.tree, 1_800_000_000_123_456_700);

    let tools = built.read();

    assert!(tools.is_ok(), "{:?}", tools.err());
}

/// A watched folder is recorded with no size, since Linux and Windows
/// count a folder's size differently (issue 2767).
#[test]
fn tools_whose_watched_folder_is_as_it_was_built_from_are_read() {
    let env = TestEnv::new();
    let built = BuiltTools::new(&env);
    let folder = built.source.parent().unwrap();
    let modified = std::fs::metadata(folder).unwrap().modified().unwrap();
    let nanoseconds = modified.duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let record = serde_json::json!({
        "root": built.tree,
        "inputs": [["src", 0, u64::try_from(nanoseconds).unwrap(), null]],
    });
    std::fs::write(
        built.manifest.with_extension("inputs.json"),
        record.to_string(),
    )
    .unwrap();

    let tools = built.read();

    assert!(tools.is_ok(), "{:?}", tools.err());
}

/// A folder has no content to compare, so its time alone says it changed.
#[test]
fn tools_whose_watched_folder_changed_time_are_refused() {
    let env = TestEnv::new();
    let built = BuiltTools::new(&env);
    let folder = built.source.parent().unwrap();
    let modified = std::fs::metadata(folder).unwrap().modified().unwrap();
    let before = modified - Duration::from_secs(3600);
    let nanoseconds = before.duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let record = serde_json::json!({
        "root": built.tree,
        "inputs": [["src", 0, u64::try_from(nanoseconds).unwrap(), null]],
    });
    std::fs::write(
        built.manifest.with_extension("inputs.json"),
        record.to_string(),
    )
    .unwrap();

    let read = built.read();

    let error = read.unwrap_err();
    assert!(
        matches!(&error, ToolsError::Stale { input, .. } if input == Path::new("src")),
        "{error}"
    );
}

/// Rewrites the file at `path` with `content` and sets its modification time
/// to `modified`.
fn rewrite(path: &Path, content: &[u8], modified: std::time::SystemTime) {
    std::fs::write(path, content).unwrap();
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(modified).unwrap();
}

/// An hour after the file at `path` was last modified.
fn an_hour_later(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path).unwrap().modified().unwrap() + Duration::from_secs(3600)
}

/// A check that writes a source back with the bytes it had, as
/// `ci/workspace-hack.sh check` once did to `Cargo.lock`, moves its time
/// alone: the tools are still built from it (issue 3236).
#[test]
fn tools_whose_source_was_rewritten_with_its_own_bytes_are_read() {
    let env = TestEnv::new();
    let built = BuiltTools::new(&env);
    let content = std::fs::read(&built.source).unwrap();
    rewrite(&built.source, &content, an_hour_later(&built.source));

    let tools = built.read();

    assert!(tools.is_ok(), "{:?}", tools.err());
}

/// What a merge, a `git switch` or an edit does to a source after the tools
/// were built: its bytes change, at its size or not, or it is gone.
#[rstest]
#[case::its_size(|source: &Path| std::fs::write(source, "pub fn merged() {}\npub fn built() {}\n").unwrap())]
#[case::its_content(|source: &Path| rewrite(source, b"pub fn moved() {}\n", an_hour_later(source)))]
#[case::removed(|source: &Path| std::fs::remove_file(source).unwrap())]
fn tools_older_than_a_source_are_refused_naming_it_and_the_rebuild_command(
    #[case] change: fn(&Path),
) {
    let env = TestEnv::new();
    let built = BuiltTools::new(&env);
    change(&built.source);

    let read = built.read();

    let error = read.unwrap_err();
    assert!(
        matches!(&error, ToolsError::Stale { input, .. } if input == Path::new("src/lib.rs")),
        "{error}"
    );
    // As a test that unwraps `tools()` prints it.
    let message = format!("{error:?}");
    assert!(message.contains("src/lib.rs"), "{message}");
    assert!(message.contains("`just _test-tools`"), "{message}");
}

#[test]
fn tools_built_from_another_tree_are_refused_naming_the_rebuild_command() {
    let env = TestEnv::new();
    let built = BuiltTools::new(&env);
    let other = env.folder().join("other-tree");
    std::fs::create_dir_all(&other).unwrap();
    built.record(&other);

    let read = built.read();

    let error = read.unwrap_err();
    assert!(
        matches!(&error, ToolsError::OtherTree { built_from, .. } if *built_from == other),
        "{error}"
    );
    let message = format!("{error:?}");
    assert!(message.contains("`just _test-tools`"), "{message}");
}

/// The same tree under another spelling of its path, as a link gives it, is
/// this tree.
#[cfg(unix)]
#[test]
fn tools_built_from_this_tree_under_a_link_are_read() {
    let env = TestEnv::new();
    let built = BuiltTools::new(&env);
    let link = env.folder().join("link");
    std::os::unix::fs::symlink(&built.tree, &link).unwrap();
    built.record(&link);

    let tools = built.read();

    assert!(tools.is_ok(), "{}", tools.unwrap_err());
}

#[test]
fn an_input_record_that_is_not_one_fails() {
    let env = TestEnv::new();
    let built = BuiltTools::new(&env);
    std::fs::write(built.manifest.with_extension("inputs.json"), "{").unwrap();

    let read = built.read();

    let error = read.unwrap_err();
    assert!(matches!(error, ToolsError::Invalid { .. }), "{error}");
}

/// `just test` builds the commit's executables with the tests' own profile,
/// so the manifest names the same `goodfetch-test-env` Cargo built for these
/// tests, and its input record says they were built from this tree as it
/// is.
#[test]
fn the_manifest_just_test_writes_lists_this_commits_test_env() {
    let tools = tools().unwrap();

    let path = tools.path(Tool::TestEnv);

    assert_eq!(path.unwrap(), Path::new(TEST_ENV));
}
