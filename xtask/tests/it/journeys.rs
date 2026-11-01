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

use goodfetch_test_support::Tool;
use rstest::rstest;
use serde_json::Value;

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

/// A check scoped to surfaces its Journey has no variant on belongs to no
/// variant.
#[rstest]
#[case::unlisted_by_adr_0063("[\"desktop\"]")]
#[case::not_a_variant_of_the_journey("[\"console\"]")]
fn check_scoped_to_a_surface_the_journey_has_no_variant_on_fails(#[case] surfaces: &str) {
    let workspace = catalog();
    workspace.edit(
        "journeys/J-003.toml",
        "task = \"catalog/1\"",
        &format!("task = \"catalog/1\"\nsurfaces = {surfaces}"),
    );

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("check-surface", "journeys/J-003.toml")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// A check that names a task its Journey does not pin asks the oracle for
/// a task no variant can reach.
#[test]
fn check_naming_a_task_the_journey_does_not_pin_fails() {
    let workspace = catalog();
    workspace.edit(
        "journeys/J-003.toml",
        "task = \"catalog/1\"",
        "task = \"catalog/2\"",
    );

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding("check-task-unpinned", "journeys/J-003.toml")],
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

/// `xtask journeys tasks` on the commit's real Proving ground, which `just
/// test` builds and lists in the test tool manifest.
fn tasks(workspace: &Workspace, arguments: &[&str]) -> crate::workspace::Outcome {
    let tools = goodfetch_test_support::tools().unwrap();
    let proving_ground = tools.path(Tool::ProvingGround).unwrap();
    let mut command = vec![
        "journeys",
        "tasks",
        "--proving-ground",
        proving_ground.to_str().unwrap(),
    ];
    command.extend_from_slice(arguments);
    workspace.xtask(&command)
}

/// A seed no Journey of the fixture catalog uses.
const UNUSED_LIST: &str = "journeys/tasks/seed-9.json";

#[test]
fn tasks_commits_the_proving_grounds_list_of_each_seed_the_catalog_uses() {
    let workspace = catalog();
    workspace.remove(TASK_LIST);
    workspace.write(UNUSED_LIST, b"{}\n");

    let outcome = tasks(&workspace, &[]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let list: Value = serde_json::from_str(&workspace.read_text(TASK_LIST)).unwrap();
    assert_eq!(list["seed"], 3, "{list}");
    let crawl = list["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["id"] == "catalog/1")
        .unwrap_or_else(|| panic!("the list names catalog/1: {list}"));
    assert_eq!(crawl["start_url"], "http://1.catalog.test/catalog/1");
    let digest = crawl["digest"].as_str().unwrap();
    assert!(
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "{digest}"
    );
    assert_eq!(workspace.read(UNUSED_LIST), None);
    let checked = tasks(&workspace, &["--check"]);
    assert_eq!(checked.findings(), vec![], "{}", checked.stderr);
    assert_eq!(checked.code, 0, "{}", checked.stderr);
}

/// The Pre-merge drift check: a committed list that is not the Proving
/// ground's for its seed fails, and the check writes nothing.
#[rstest]
#[case::digest_changed("digest", TASK_LIST)]
#[case::list_missing("missing", TASK_LIST)]
#[case::seed_no_journey_uses("unused", UNUSED_LIST)]
fn drift_check_fails_on_a_list_that_is_not_the_proving_grounds(
    #[case] drift: &str,
    #[case] subject: &str,
) {
    let workspace = catalog();
    let refreshed = tasks(&workspace, &[]);
    assert_eq!(refreshed.code, 0, "{}", refreshed.stderr);
    match drift {
        "digest" => {
            let mut list: Value = serde_json::from_str(&workspace.read_text(TASK_LIST)).unwrap();
            list["tasks"][0]["digest"] = Value::from("0".repeat(64));
            workspace.write(TASK_LIST, &serde_json::to_vec_pretty(&list).unwrap());
        }
        "missing" => workspace.remove(TASK_LIST),
        _ => workspace.write(UNUSED_LIST, &workspace.read(TASK_LIST).unwrap()),
    }
    let before = (workspace.read(TASK_LIST), workspace.read(UNUSED_LIST));

    let outcome = tasks(&workspace, &["--check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("task-list-drift", subject)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
    assert_eq!(
        (workspace.read(TASK_LIST), workspace.read(UNUSED_LIST)),
        before
    );
}

/// A Journey that does not parse names no seed, so writing the lists then
/// would delete its seed's committed list: `tasks` refuses instead, and
/// keeps every list.
#[test]
fn tasks_refuses_while_a_journey_does_not_parse_and_keeps_its_list() {
    let workspace = catalog();
    workspace.edit("journeys/J-003.toml", "[surfaces]", "[surfaces");

    let outcome = tasks(&workspace, &[]);

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
    assert!(
        outcome.stderr.contains("journeys/J-003.toml"),
        "{}",
        outcome.stderr
    );
    assert!(workspace.read(TASK_LIST).is_some());
}
