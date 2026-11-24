//! goodfetch-conformance's integration tests (Seam 3 of the Proving ground
//! and conformance spec, Pre-merge): the writer that turns a case log into a
//! result document, the result document of each kind against its schema,
//! the self-test suite against a running Proving ground, and the suites
//! run against engines over the protocol and against process Plugins.
#![expect(
    clippy::disallowed_methods,
    reason = "these tests start processes and threads and read files outside any async worker"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod case_log;
mod client_case;
mod gate;
mod harness_failures;
mod journey_document;
mod journey_run;
mod keep;
mod page;
mod plugin;
mod protocol;
mod read_command;
mod refusals;
mod self_test;
mod write_command;

use std::path::{Path, PathBuf};

use goodfetch_protocol::result_document::{ConformanceDocument, Platform, ResultDocument};
use goodfetch_test_support::{RESULT_DOCUMENTS_VARIABLE, Variables};
use serde_json::{Value, json};

/// Keeps a suite's document where `just test` gathers the result documents
/// (`GOODFETCH_TEST_RESULT_DOCUMENTS`), as `conformance-<name>.json`, so a
/// Release cell keeps it and the Feature report reads its cases (ADR 0072).
/// Returns the file written; none outside `just test`.
fn keep(document: &ConformanceDocument, name: &str) -> Option<PathBuf> {
    let folder = Variables::from_process()
        .get(RESULT_DOCUMENTS_VARIABLE)
        .map(PathBuf::from);
    keep_in(folder.as_deref(), document, name)
}

/// [`keep`] into `folder`, when there is one.
fn keep_in(folder: Option<&Path>, document: &ConformanceDocument, name: &str) -> Option<PathBuf> {
    let folder = folder?;
    std::fs::create_dir_all(folder).unwrap();
    let path = folder.join(format!("conformance-{name}.json"));
    let mut bytes =
        serde_json::to_vec_pretty(&ResultDocument::Conformance(document.clone())).unwrap();
    bytes.push(b'\n');
    std::fs::write(&path, bytes).unwrap();
    Some(path)
}

/// The committed JSON Schema bundle, which `xtask codegen` writes.
const BUNDLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../goodfetch-protocol/generated/bundle.json"
);

/// The schema violations of a document, as the bundle's result document
/// schema reports them; none when it matches.
fn schema_violations(document: &ResultDocument) -> Vec<String> {
    let bundle: Value = serde_json::from_slice(&std::fs::read(Path::new(BUNDLE)).unwrap()).unwrap();
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": bundle["documents"]["result_document"]["$ref"],
        "$defs": bundle["$defs"],
    });
    let instance = serde_json::to_value(document).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .iter_errors(&instance)
        .map(|error| format!("{}: {error}", error.instance_path()))
        .collect()
}

/// The platform the tests say they ran on.
fn platform() -> Platform {
    Platform {
        os: "linux".to_owned(),
        arch: "x86_64".to_owned(),
    }
}
