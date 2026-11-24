//! `xtask chromium package` (ADR 0074; TESTING.md, The Goodfetch Chromium):
//! each suite's runtime dependencies, read from the `.runtime_deps` list `gn`
//! writes in a fixture output folder, kept by SHA-256 with a manifest of
//! digests, a file over the split size cut into parts. The local tooling
//! declares no Feature.
#![cfg(unix)]

use std::collections::BTreeSet;

use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::workspace::{Outcome, Workspace, finding, write_executable};

/// The fixture's `gn` output folder, two levels under the source root as
/// `out/Release` is.
const OUT: &str = "src/out/Release";

/// A fixture output folder: two suites' executables, their runtime-deps
/// lists naming a file beside them, one in the source tree and a folder,
/// and files no list names.
fn built() -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(&format!("{OUT}/obj/unlisted.o"), b"object\n");
    write_executable(
        &workspace.path().join(OUT).join("unit_tests"),
        "unit tests\n",
    );
    write_executable(
        &workspace.path().join(OUT).join("browser_tests"),
        "browser tests\n",
    );
    workspace.write("src/third_party/data/a.txt", b"source data\n");
    workspace.write("src/third_party/data/unlisted.txt", b"not listed\n");
    workspace.write(&format!("{OUT}/test_data/one.bin"), b"one\n");
    workspace.write(&format!("{OUT}/test_data/nested/two.bin"), b"two\n");
    workspace.write(
        &format!("{OUT}/unit_tests.runtime_deps"),
        b"./unit_tests\n../../third_party/data/a.txt\n./test_data/\n",
    );
    workspace.write(
        &format!("{OUT}/browser_tests.runtime_deps"),
        b"./browser_tests\n",
    );
    workspace
}

fn package(workspace: &Workspace, into: &str, extra: &[&str]) -> Outcome {
    let mut args = vec![
        "chromium",
        "package",
        "--out",
        OUT,
        "--into",
        into,
        "src/out/Release/unit_tests.runtime_deps",
        "src/out/Release/browser_tests.runtime_deps",
    ];
    args.extend_from_slice(extra);
    workspace.xtask(&args)
}

fn manifest(workspace: &Workspace, into: &str) -> Value {
    let text = workspace.read_text(&format!("{into}/manifest.json"));
    serde_json::from_str(&text).unwrap()
}

/// The manifest's entries of one suite, by path.
fn files<'a>(manifest: &'a Value, suite: &str) -> Vec<&'a Value> {
    let suite = manifest["suites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == suite)
        .unwrap_or_else(|| panic!("no suite {suite} in {manifest}"));
    suite["files"].as_array().unwrap().iter().collect()
}

fn paths(files: &[&Value]) -> Vec<String> {
    files
        .iter()
        .map(|file| file["path"].as_str().unwrap().to_owned())
        .collect()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The file an entry names, put back together from its parts' blobs, each
/// checked to be kept under its own digest.
fn reassembled(workspace: &Workspace, into: &str, file: &Value) -> Vec<u8> {
    let mut whole = Vec::new();
    for part in file["parts"].as_array().unwrap() {
        let digest = part["sha256"].as_str().unwrap();
        let blob = workspace
            .read(&format!("{into}/blobs/{digest}"))
            .unwrap_or_else(|| panic!("no blob {digest}"));
        assert_eq!(sha256(&blob), digest, "blob kept under another digest");
        assert_eq!(part["size"].as_u64().unwrap(), blob.len() as u64);
        whole.extend_from_slice(&blob);
    }
    whole
}

#[test]
fn the_package_holds_exactly_the_listed_files() {
    let workspace = built();

    let outcome = package(&workspace, "package", &[]);

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    let manifest = manifest(&workspace, "package");
    let names: Vec<_> = manifest["suites"]
        .as_array()
        .unwrap()
        .iter()
        .map(|suite| suite["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["browser_tests", "unit_tests"]);
    let unit = files(&manifest, "unit_tests");
    assert_eq!(
        paths(&unit),
        [
            "../../third_party/data/a.txt",
            "./test_data/nested/two.bin",
            "./test_data/one.bin",
            "./unit_tests",
        ]
    );
    assert_eq!(
        paths(&files(&manifest, "browser_tests")),
        ["./browser_tests"]
    );
    let mut referenced = BTreeSet::new();
    for file in unit.iter().chain(&files(&manifest, "browser_tests")) {
        let path = file["path"].as_str().unwrap();
        let source = workspace.read(&format!("{OUT}/{path}")).unwrap();
        let kept = reassembled(&workspace, "package", file);
        assert_eq!(kept, source, "{path}");
        assert_eq!(file["sha256"].as_str().unwrap(), sha256(&source), "{path}");
        assert_eq!(file["size"].as_u64().unwrap(), source.len() as u64);
        assert_eq!(file["parts"].as_array().unwrap().len(), 1, "{path}");
        for part in file["parts"].as_array().unwrap() {
            referenced.insert(part["sha256"].as_str().unwrap().to_owned());
        }
    }
    let executable: Vec<_> = unit
        .iter()
        .filter(|file| file["executable"] == true)
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(executable, ["./unit_tests"]);
    let blobs: BTreeSet<_> = std::fs::read_dir(workspace.path().join("package/blobs"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(blobs, referenced, "the package holds an unlisted file");
}

#[test]
fn a_missing_listed_file_fails() {
    let workspace = built();
    workspace.append(&format!("{OUT}/unit_tests.runtime_deps"), "./gone.bin\n");

    let outcome = package(&workspace, "package", &[]);

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        outcome.findings(),
        [finding("missing-runtime-dep", "unit_tests ./gone.bin")]
    );
    assert!(
        workspace.read("package/manifest.json").is_none(),
        "a failed package wrote its manifest"
    );
}

#[test]
fn a_file_over_the_split_size_is_split_into_parts_that_reassemble_it() {
    let workspace = built();
    let big = b"0123456789abcdefghij-";
    workspace.write(&format!("{OUT}/test_data/one.bin"), big);

    let outcome = package(&workspace, "package", &["--part-size", "8"]);

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    let manifest = manifest(&workspace, "package");
    let unit = files(&manifest, "unit_tests");
    let file = unit
        .iter()
        .find(|file| file["path"] == "./test_data/one.bin")
        .unwrap();
    let sizes: Vec<_> = file["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| part["size"].as_u64().unwrap())
        .collect();
    assert_eq!(sizes, [8, 8, 5]);
    let whole = reassembled(&workspace, "package", file);
    assert_eq!(whole, big);
    assert_eq!(file["sha256"].as_str().unwrap(), sha256(&whole));
    let small = unit
        .iter()
        .find(|file| file["path"] == "./test_data/nested/two.bin")
        .unwrap();
    assert_eq!(small["parts"].as_array().unwrap().len(), 1);
}

#[test]
fn packaging_the_same_inputs_twice_gives_identical_digests() {
    let workspace = built();
    workspace.write(
        &format!("{OUT}/test_data/one.bin"),
        b"0123456789abcdefghij-",
    );

    let first = package(&workspace, "first", &["--part-size", "8"]);
    let second = package(&workspace, "second", &["--part-size", "8"]);

    assert_eq!(first.code, 0, "{}{}", first.stdout, first.stderr);
    assert_eq!(second.code, 0, "{}{}", second.stdout, second.stderr);
    assert_eq!(
        workspace.read("first/manifest.json").unwrap(),
        workspace.read("second/manifest.json").unwrap()
    );
}
