use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use super::{InputRecord, Workspace};

/// A workspace in a temporary folder whose one tool Cargo built from
/// `prerequisites`, paths from its folder, each written here with `files`.
struct Built {
    folder: tempfile::TempDir,
    manifests: Vec<PathBuf>,
}

impl Built {
    fn new(files: &[&str], prerequisites: &[&str]) -> Self {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        for file in files.iter().chain(["target/debug/tool"].iter()) {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "four").unwrap();
        }
        let named: Vec<String> = prerequisites
            .iter()
            .map(|path| root.join(path).display().to_string().replace(' ', "\\ "))
            .collect();
        let dep_info = format!(
            "{}: {}\n",
            root.join("target/debug/tool").display(),
            named.join(" ")
        );
        std::fs::write(root.join("target/debug/tool.d"), dep_info).unwrap();
        let manifests = files
            .iter()
            .filter(|file| file.ends_with("/Cargo.toml"))
            .map(|file| root.join(file))
            .collect();
        Self { folder, manifests }
    }

    /// The record of the tool, built from `started` on, as its file holds it.
    fn record(&self, started: SystemTime) -> anyhow::Result<Value> {
        let root = self.folder.path();
        let target_directory = root.join("target");
        let workspace = Workspace {
            folder: root,
            target_directory: &target_directory,
            package_manifests: &self.manifests,
        };
        let record = InputRecord::of(&workspace, &[&root.join("target/debug/tool")], started)?;
        Ok(serde_json::to_value(record)?)
    }

    /// The paths the record names, of a build that began after every file
    /// was written.
    fn sources(&self) -> Vec<String> {
        let record = self.record(SystemTime::now() + Duration::from_secs(60));
        record.unwrap()["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|input| input[0].as_str().unwrap().to_owned())
            .collect()
    }
}

#[test]
fn a_source_is_recorded_with_its_size_and_its_packages_manifest() {
    let built = Built::new(
        &[
            "crates/engine/Cargo.toml",
            "crates/engine/src/my lib.rs",
            "xtask/Cargo.toml",
        ],
        &["crates/engine/src/my lib.rs"],
    );

    let record = built
        .record(SystemTime::now() + Duration::from_secs(60))
        .unwrap();

    assert_eq!(
        Path::new(record["root"].as_str().unwrap()),
        built.folder.path()
    );
    let inputs: Vec<Value> = record["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|input| json!([input[0], input[1]]))
        .collect();
    assert_eq!(
        inputs,
        [
            json!(["crates/engine/Cargo.toml", 4]),
            json!(["crates/engine/src/my lib.rs", 4])
        ]
    );
}

#[test]
fn the_files_every_build_reads_are_sources() {
    let built = Built::new(
        &[
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            "src/main.rs",
        ],
        &["src/main.rs"],
    );

    let sources = built.sources();

    assert_eq!(
        sources,
        [
            ".cargo/config.toml",
            "Cargo.lock",
            "Cargo.toml",
            "rust-toolchain.toml",
            "src/main.rs"
        ]
    );
}

/// Git's files change at a commit or at another worktree's fetch, and the
/// target folder's at any build: neither is a source.
#[test]
fn gits_files_and_the_target_folders_are_no_sources() {
    let built = Built::new(
        &[
            "src/main.rs",
            ".git/HEAD",
            "target/debug/build/rules/out/table.rs",
        ],
        &[
            "src/main.rs",
            ".git/HEAD",
            "target/debug/build/rules/out/table.rs",
        ],
    );

    let sources = built.sources();

    assert_eq!(sources, ["src/main.rs"]);
}

/// A build script that watches a folder reads the files in it, and a file
/// changed in place leaves the folder's own time as it was.
#[test]
fn a_watched_folder_is_recorded_with_every_file_under_it() {
    let built = Built::new(
        &["src/main.rs", "rules/list.txt", "rules/more/compact.json"],
        &["src/main.rs", "rules"],
    );

    let sources = built.sources();

    assert_eq!(
        sources,
        [
            "rules",
            "rules/list.txt",
            "rules/more",
            "rules/more/compact.json",
            "src/main.rs"
        ]
    );
}

/// A folder's size is the OS's own count, which Linux and Windows make
/// differently, so a record made on Linux and read under Wine would find
/// every watched folder changed (issue 2767): a folder is recorded with
/// none.
#[test]
fn a_watched_folder_is_recorded_with_no_size() {
    let built = Built::new(
        &["src/main.rs", "rules/list.txt"],
        &["src/main.rs", "rules"],
    );

    let record = built
        .record(SystemTime::now() + Duration::from_secs(60))
        .unwrap();

    let sizes: Vec<Value> = record["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|input| json!([input[0], input[1]]))
        .collect();
    assert_eq!(
        sizes,
        [
            json!(["rules", 0]),
            json!(["rules/list.txt", 4]),
            json!(["src/main.rs", 4])
        ]
    );
}

/// A record without the tool's sources would pass tools of any age.
#[test]
fn a_tool_whose_dep_info_names_no_source_of_the_workspace_fails() {
    let built = Built::new(&[".git/HEAD"], &[".git/HEAD"]);

    let record = built.record(SystemTime::now() + Duration::from_secs(60));

    assert!(record.is_err());
}

/// Cargo may have compiled such a source as it was before the change, so a
/// record of it as it is now would pass tools older than it.
#[test]
fn a_source_changed_while_the_tools_were_building_fails() {
    let built = Built::new(&["src/main.rs"], &["src/main.rs"]);

    let record = built.record(SystemTime::now() - Duration::from_secs(60));

    let error = record.unwrap_err().to_string();
    assert!(error.contains("src/main.rs"), "{error}");
}

/// A build that resolves anew writes the lockfile itself, then builds from
/// it.
#[test]
fn a_lockfile_written_while_the_tools_were_building_is_recorded() {
    let built = Built::new(&["Cargo.lock"], &["Cargo.lock"]);

    let record = built.record(SystemTime::now() - Duration::from_secs(60));

    assert_eq!(record.unwrap()["inputs"][0][0], "Cargo.lock");
}
