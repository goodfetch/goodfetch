//! `xtask journeys check`: the rules of ADR 0063 over the Journey catalog
//! that need no process, in the Quick suite. A Journey file parses under
//! `goodfetch-protocol`'s `Journey` type and the committed
//! `journeys/journey.schema.json` accepts it; its name is its ID; no two
//! Journeys share a number and none reaches `journeys/catalog.toml`'s next
//! free number; every surface is one ADR 0063 lists, never named for a
//! milestone before the surface ships; a retired Journey gives its reason;
//! each task a Journey names is in its seed's committed task list, with the
//! digest it pins. Each finding names the file and the rule it breaks.
//!
//! `xtask journeys tasks`, in the Pre-merge suite, writes and drift-checks
//! those lists from a real Proving ground.

use std::path::Path;

use rstest::rstest;

use crate::workspace::{Workspace, finding};

/// The repository's own `journeys/` folder.
fn committed() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("journeys")
}

const SCHEMA: &str = "journeys/journey.schema.json";

/// The fixture catalog, with the schema `xtask codegen` commits.
fn catalog() -> Workspace {
    let workspace = Workspace::from_fixture("journeys");
    workspace.write(
        SCHEMA,
        &std::fs::read(committed().join("journey.schema.json")).unwrap(),
    );
    workspace
}

fn check(workspace: &Workspace) -> crate::workspace::Outcome {
    workspace.xtask(&["journeys", "check"])
}

#[test]
fn clean_catalog_passes() {
    let workspace = catalog();

    let outcome = check(&workspace);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// Copies the files of `folder`, and of its folders, to `journeys/` in the
/// workspace under `prefix`.
fn copy_catalog(workspace: &Workspace, folder: &Path, prefix: &str) {
    for entry in std::fs::read_dir(folder).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_catalog(workspace, &entry.path(), &format!("{prefix}{name}/"));
        } else {
            workspace.write(
                &format!("journeys/{prefix}{name}"),
                &std::fs::read(entry.path()).unwrap(),
            );
        }
    }
}

/// The repository's catalog as committed, J-001, its schema and its task
/// lists included.
#[test]
fn committed_catalog_passes() {
    let workspace = Workspace::from_fixture("empty");
    copy_catalog(&workspace, &committed(), "");

    let outcome = check(&workspace);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::unknown_field("status = \"active\"\n", "status = \"active\"\nowner = \"area 3\"\n")]
#[case::no_product("product = \"collie\"\n", "")]
#[case::unknown_product("product = \"collie\"", "product = \"cloud\"")]
#[case::malformed_id("id = \"J-001\"", "id = \"J-1\"")]
#[case::malformed_feature_id("covers = [\"row:CN-03\"]", "covers = [\"CN-03\"]")]
#[case::unknown_milestone("console = \"M1\"", "console = \"M5\"")]
#[case::not_toml("[surfaces]", "[surfaces")]
fn journey_that_does_not_parse_fails(#[case] from: &str, #[case] to: &str) {
    let workspace = catalog();
    workspace.edit("journeys/J-001.toml", from, to);

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("parse", "journeys/J-001.toml")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::missing(None)]
#[case::unknown_field(Some("next = 4\nlast = 3\n"))]
fn catalog_file_that_does_not_parse_fails(#[case] contents: Option<&str>) {
    let workspace = catalog();
    match contents {
        Some(contents) => workspace.write("journeys/catalog.toml", contents.as_bytes()),
        None => workspace.remove("journeys/catalog.toml"),
    }

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("parse", "journeys/catalog.toml")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn file_named_other_than_its_id_fails() {
    let workspace = catalog();
    let contents = workspace.read_text("journeys/J-001.toml");
    workspace.remove("journeys/J-001.toml");
    workspace.write("journeys/start-the-daemon.toml", contents.as_bytes());

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("file-name", "journeys/start-the-daemon.toml")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// `J-0001` and `J-001` are one number, so one ID.
#[test]
fn two_journeys_with_one_number_fail() {
    let workspace = catalog();
    let contents = workspace
        .read_text("journeys/J-001.toml")
        .replace("id = \"J-001\"", "id = \"J-0001\"");
    workspace.write("journeys/J-0001.toml", contents.as_bytes());

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![
            finding("duplicate-id", "journeys/J-0001.toml"),
            finding("duplicate-id", "journeys/J-001.toml"),
        ],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::equal_to_it("next = 3", &["journeys/J-003.toml"])]
#[case::above_it("next = 2", &["journeys/J-002.toml", "journeys/J-003.toml"])]
fn id_not_below_the_next_free_number_fails(#[case] next: &str, #[case] subjects: &[&str]) {
    let workspace = catalog();
    workspace.edit("journeys/catalog.toml", "next = 4", next);

    let outcome = check(&workspace);

    let expected: Vec<_> = subjects
        .iter()
        .map(|subject| finding("id-too-high", subject))
        .collect();
    assert_eq!(outcome.findings(), expected, "{}", outcome.stderr);
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::listed_surface("console = \"M1\"", "console = \"M1\"\ndesktop = \"M2\"")]
#[case::pinned_step("on = \"cli\"", "on = \"desktop\"")]
#[case::adapter_without_a_name("console = \"M1\"", "console = \"M1\"\nadapter- = \"M4\"")]
fn surface_adr_0063_does_not_list_fails(#[case] from: &str, #[case] to: &str) {
    let workspace = catalog();
    workspace.edit("journeys/J-001.toml", from, to);

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("unlisted-surface", "journeys/J-001.toml")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::sdk_before_m2("sdk-rust = \"M2\"", "sdk-rust = \"M1\"")]
#[case::adapter_before_m4("adapter-langchain = \"M4\"", "adapter-langchain = \"M3\"")]
fn variant_earlier_than_its_surface_fails(#[case] from: &str, #[case] to: &str) {
    let workspace = catalog();
    workspace.edit("journeys/J-003.toml", from, to);

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("milestone-too-early", "journeys/J-003.toml")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::no_reason("")]
#[case::blank_reason("reason = \"  \"\n")]
fn retired_journey_without_a_reason_fails(#[case] reason: &str) {
    let workspace = catalog();
    workspace.edit(
        "journeys/J-002.toml",
        "reason = \"Its goal now includes exporting the Dataset, which J-003 holds\"\n",
        reason,
    );

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("retired-without-reason", "journeys/J-002.toml")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// A schema that disagrees with the type, as a stale or wrongly generated
/// one would: it refuses every Journey the type parses.
#[test]
fn journey_the_committed_schema_refuses_fails() {
    let workspace = catalog();
    workspace.write(
        SCHEMA,
        br#"{ "$schema": "https://json-schema.org/draft/2020-12/schema", "required": ["owner"] }"#,
    );

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![
            finding("schema", "journeys/J-001.toml"),
            finding("schema", "journeys/J-002.toml"),
            finding("schema", "journeys/J-003.toml"),
        ],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn missing_schema_fails() {
    let workspace = catalog();
    workspace.remove(SCHEMA);

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("schema", SCHEMA)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// The fixture's committed task list of seed 3, which J-003 pins.
const TASK_LIST: &str = "journeys/tasks/seed-3.json";

#[rstest]
#[case::removed_from_its_list(TASK_LIST, "\"id\": \"catalog/1\"", "\"id\": \"catalog/7\"")]
#[case::seed_with_no_list("journeys/J-003.toml", "seed = 3", "seed = 4")]
fn journey_naming_a_task_absent_from_its_seed_list_fails(
    #[case] file: &str,
    #[case] from: &str,
    #[case] to: &str,
) {
    let workspace = catalog();
    workspace.edit(file, from, to);

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("missing-task", "journeys/J-003.toml")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// The Proving ground changed the task's entry or expected result, so the
/// digest the Journey pins is no longer the one its seed's list holds.
#[test]
fn pinned_digest_its_seed_list_no_longer_holds_fails() {
    let workspace = catalog();
    workspace.edit(
        TASK_LIST,
        "00000000000000000000000000000000000000000000000000000000000000a1",
        "00000000000000000000000000000000000000000000000000000000000000b1",
    );

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("task-digest", "journeys/J-003.toml")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn task_list_that_does_not_parse_fails() {
    let workspace = catalog();
    workspace.edit(TASK_LIST, "\"seed\": 3", "\"seed\": \"3\"");

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("parse", TASK_LIST)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// The Quick suite starts no process (ADR 0063): the task rule reads the
/// committed lists, so the check passes with no program it could start.
#[test]
fn task_rule_reads_the_committed_lists_and_starts_no_process() {
    let workspace = catalog();
    let nothing = tempfile::tempdir().unwrap();
    let no_cargo = nothing.path().join("cargo");

    let outcome = workspace.xtask_with_env(
        &["journeys", "check"],
        &[
            ("PATH", nothing.path().as_os_str()),
            ("CARGO", no_cargo.as_os_str()),
        ],
    );

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}
