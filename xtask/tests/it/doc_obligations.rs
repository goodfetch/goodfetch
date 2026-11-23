//! `xtask sdk-exports`'s doc obligations: each rule `sdks/doc-obligations.toml`
//! lists is in the doc text of every member, catalog entry, schema and page
//! it binds, at its start when it opens it (ADRs 0117, 0119, 0121). An
//! obligation bound to nothing, or pending on an entry that now exists,
//! fails; each text is English only (ADR 0073), and an operation's
//! description, which an MCP tool's description copies, holds at most 2,048
//! characters. The fixture is the one of `xtask sdk-exports`.
//! Covers: story:11-193, story:11-194, story:11-196

use rstest::rstest;

use crate::workspace::{Finding, Workspace, finding};

const OBLIGATIONS: &str = "sdks/doc-obligations.toml";
const CATALOG: &str = "crates/goodfetch-protocol/generated/catalog.json";
const BUNDLE: &str = "crates/goodfetch-protocol/generated/bundle.json";
const GUIDE: &str = "docs/runs.md";
const COLLIE: &str = "sdks/typescript/collie/src/collie.ts";
const RUST_FOXHOUND: &str = "crates/foxhound/goodfetch-foxhound/src/foxhound.rs";
const PYTHON_COLLIE: &str = "sdks/python/collie/src/goodfetch_collie/collie.py";
/// `run.abort`'s description, which an obligation of the fixture binds.
const ABORTS: &str = "Aborts the Run.";

fn workspace() -> Workspace {
    let workspace = Workspace::from_fixture("sdk-exports");
    workspace.link_installed_runners();
    workspace
}

/// The fixture's obligation that each member serving `run.start` opens with
/// "Starts a Run".
#[rstest]
#[case::typescript(COLLIE, "sdks/typescript/manifest.toml Collie.start")]
#[case::rust(
    RUST_FOXHOUND,
    "crates/foxhound/goodfetch-foxhound/manifest.toml Foxhound.launch"
)]
#[case::python(PYTHON_COLLIE, "sdks/python/manifest.toml Collie.launch")]
fn check_refuses_a_member_whose_doc_does_not_open_with_its_obligation(
    #[case] file: &str,
    #[case] subject: &str,
) {
    let workspace = workspace();
    workspace.edit(
        file,
        "Starts a Run, and",
        "Once called, it Starts a Run, and",
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("missing-obligation", subject)],
        "{}",
        outcome.stdout
    );
    assert!(
        outcome
            .stdout
            .contains("does not open with \"Starts a Run\""),
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

/// A member is chosen by its receiver as every SDK names it and its words
/// joined by `_`, and its doc holds the text however it wraps it.
#[rstest]
#[case::held("Closes it.", vec![])]
#[case::wrapped("Closes\n it.", vec![])]
#[case::lacking(
    "Closes the Run.",
    vec![finding(
        "missing-obligation",
        "crates/foxhound/goodfetch-foxhound/manifest.toml Foxhound.close"
    )]
)]
fn check_reads_the_members_an_obligation_chooses(
    #[case] text: &str,
    #[case] expected: Vec<Finding>,
) {
    let workspace = workspace();
    workspace.append(
        OBLIGATIONS,
        &format!(
            "\n[[obligation]]\ntext = \"\"\"{text}\"\"\"\nmembers = [{{ receiver = \"Foxhound\", member = \"close\" }}]\n"
        ),
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(outcome.findings(), expected, "{}", outcome.stdout);
}

/// The generator copies an operation's description and a schema's into
/// every SDK, so the obligation is read there, as on a docs page.
#[rstest]
#[case::operation(
    CATALOG,
    ABORTS,
    "Aborts it.",
    "crates/goodfetch-protocol/generated/catalog.json run.abort"
)]
#[case::schema(
    BUNDLE,
    "resolved again at each use",
    "resolved once",
    "crates/goodfetch-protocol/generated/bundle.json Locator"
)]
#[case::page(GUIDE, "`SIGINT` and", "`SIGHUP` and", GUIDE)]
fn check_refuses_a_text_lacking_its_obligation(
    #[case] file: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] subject: &str,
) {
    let workspace = workspace();
    workspace.edit(file, from, to);

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("missing-obligation", subject)],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::operation("operations = [\"page.gone\"]", "page.gone")]
#[case::setting("settings = [\"cache.gone\"]", "cache.gone")]
#[case::schema("schemas = [\"Gone\"]", "Gone")]
#[case::page("pages = [\"docs/gone.md\"]", "docs/gone.md")]
#[case::member(
    "members = [{ receiver = \"Nobody\", member = \"close\" }]",
    "Nobody.close"
)]
#[case::nothing("", "obligation 6")]
fn check_refuses_an_obligation_bound_to_nothing(#[case] binding: &str, #[case] bound: &str) {
    let workspace = workspace();
    workspace.append(
        OBLIGATIONS,
        &format!("\n[[obligation]]\ntext = \"Gone.\"\n{binding}\n"),
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("unbound", &format!("{OBLIGATIONS} {bound}"))],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

/// The fixture's pending obligation waits for `page.act` and `cache.mode`.
#[rstest]
#[case::operation(
    "\"operations\": [\n    {\n      \"name\": \"page.act\",\n      \"product\": \"collie\",\n      \"description\": \"Needs a model.\",\n      \"stability\": \"stable\",\n      \"deprecated\": null,\n      \"reach\": \"person_only\"\n    },",
    "page.act"
)]
#[case::setting(
    "\"settings\": [{ \"name\": \"cache.mode\", \"description\": \"How the Response cache serves.\" }],\n  \"operations\": [",
    "cache.mode"
)]
fn check_refuses_a_pending_obligation_whose_entry_exists(#[case] to: &str, #[case] entry: &str) {
    let workspace = workspace();
    workspace.edit(CATALOG, "\"operations\": [", to);

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("pending-bound", &format!("{OBLIGATIONS} {entry}"))],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::obligation(
    OBLIGATIONS,
    "text = \"Needs a model\"",
    "text = \"Exige un modèle\"",
    "sdks/doc-obligations.toml obligation 5"
)]
#[case::operation(
    CATALOG,
    ABORTS,
    "Aborts the Run. Arrête le Run.",
    "crates/goodfetch-protocol/generated/catalog.json run.abort"
)]
#[case::member(
    COLLIE,
    "once it is started.",
    "once it is started. Démarre.",
    "sdks/typescript/manifest.toml Collie.start"
)]
#[case::page(
    GUIDE,
    "In a service",
    "Dans un service, à l'arrêt, in a service",
    GUIDE
)]
fn check_refuses_a_text_in_another_language(
    #[case] file: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] subject: &str,
) {
    let workspace = workspace();
    workspace.edit(file, from, to);

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("not-english", subject)],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

/// 2,048 characters is the longest description: "Aborts the Run. " and
/// 2,032 more make 2,048.
#[rstest]
#[case::at_the_limit(2_032, vec![])]
#[case::past_it(
    2_033,
    vec![finding("too-long", "crates/goodfetch-protocol/generated/catalog.json run.abort")]
)]
fn check_refuses_an_operation_description_past_2048_characters(
    #[case] more: usize,
    #[case] expected: Vec<Finding>,
) {
    let workspace = workspace();
    workspace.edit(CATALOG, ABORTS, &format!("{ABORTS} {}", "x".repeat(more)));

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(outcome.findings(), expected, "{}", outcome.stdout);
}

#[test]
fn check_fails_without_its_obligations() {
    let workspace = workspace();
    workspace.remove(OBLIGATIONS);

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert!(outcome.stderr.contains(OBLIGATIONS), "{}", outcome.stderr);
    assert_eq!(outcome.code, 2);
}
