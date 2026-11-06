//! `xtask compat` (ADR 0066): the Compatibility corpus written from the
//! format registry's writers, checked against them and against its frozen
//! sets, and frozen from a release tag.
//! Covers: story:1-110

use rstest::rstest;

use crate::workspace::{Finding, Outcome, Workspace, finding};

/// Every file under `compat/`, with its contents.
fn corpus(workspace: &Workspace) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![workspace.path().join("compat")];
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(workspace.path()).unwrap();
                files.push((
                    relative.to_string_lossy().into_owned(),
                    std::fs::read(&path).unwrap(),
                ));
            }
        }
    }
    files.sort();
    files
}

const BUNDLE: &str = "crates/goodfetch-protocol/generated/bundle.json";

fn write(workspace: &Workspace) -> Outcome {
    let outcome = workspace.xtask(&["compat", "write"]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    outcome
}

#[test]
fn writing_twice_gives_identical_bytes() {
    let workspace = Workspace::from_fixture("compat");
    write(&workspace);
    let first = corpus(&workspace);

    write(&workspace);

    assert_eq!(corpus(&workspace), first);
}

#[test]
fn writing_removes_the_head_set_of_a_format_no_longer_registered() {
    let workspace = Workspace::from_fixture("compat");
    add_head_of_an_unregistered_format(&workspace);

    write(&workspace);

    assert!(!workspace.path().join("compat/retired-format/head").exists());
}

#[test]
fn check_passes_on_a_clean_tree() {
    let workspace = Workspace::from_fixture("compat");

    let outcome = workspace.xtask(&["compat", "check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

/// A passing check still names, for each format whose writer it ran, the
/// digest of what both runs wrote, so its log is never empty and two runs'
/// logs show the output identical across runs (#2109's three runs logged
/// nothing to compare).
#[test]
fn check_notes_the_digest_each_writer_wrote_on_both_runs() {
    let workspace = Workspace::from_fixture("compat");

    let first = workspace.xtask(&["compat", "check"]);
    let second = workspace.xtask(&["compat", "check"]);

    assert_eq!(first.code, 0, "{}", first.stderr);
    assert_eq!(
        first.notes(),
        vec![
            finding("identical", "compat/sample-protocol/head"),
            finding("identical", "compat/sample-settings/head"),
            finding("identical", "compat/sample-store/head"),
        ],
        "{}",
        first.stdout
    );
    assert_eq!(first.stdout, second.stdout);
}

fn edit_head(workspace: &Workspace) {
    workspace.edit(
        "compat/sample-store/head/store.sql",
        "INSERT INTO pages VALUES(1,X'00ff');",
        "INSERT INTO pages VALUES(1,X'00fe');",
    );
}

fn add_head_of_an_unregistered_format(workspace: &Workspace) {
    workspace.write("compat/retired-format/head/old.json", b"{}\n");
}

fn edit_frozen_byte(workspace: &Workspace) {
    workspace.edit(
        "compat/sample-settings/0.1/settings.json",
        "\"pages\": 10",
        "\"pages\": 11",
    );
}

fn add_file_to_frozen_set(workspace: &Workspace) {
    workspace.write("compat/sample-settings/0.1/extra.json", b"{}\n");
}

fn remove_head_set(workspace: &Workspace) {
    workspace.remove_folder("compat/sample-protocol/head");
}

fn record_a_shipped_version_never_frozen(workspace: &Workspace) {
    workspace.edit(
        "compat/releases.toml",
        "formats = { sample-settings = \"0.1\" }",
        "formats = { sample-settings = \"0.1\", sample-store = \"0.1\" }",
    );
}

fn register_a_format_without_writer(workspace: &Workspace) {
    workspace.edit(
        BUNDLE,
        "\"formats\": [\n",
        "\"formats\": [\n    {\"name\": \"sample-cache\", \"schema\": {\"type\": \"object\"}, \"storage\": \"json\", \"version\": \"0.1\"},\n",
    );
}

#[rstest]
#[case::head_differs_from_what_write_produces(
    edit_head,
    vec![finding("drift", "compat/sample-store/head/store.sql")],
)]
#[case::head_of_a_format_no_longer_registered(
    add_head_of_an_unregistered_format,
    vec![finding("drift", "compat/retired-format/head/old.json")],
)]
#[case::frozen_byte_changed(
    edit_frozen_byte,
    vec![finding("frozen", "compat/sample-settings/0.1/settings.json")],
)]
#[case::file_added_to_a_frozen_set(
    add_file_to_frozen_set,
    vec![finding("frozen", "compat/sample-settings/0.1/extra.json")],
)]
#[case::registered_format_without_head_set(
    remove_head_set,
    vec![finding("missing-head", "compat/sample-protocol/head")],
)]
#[case::registered_format_without_writer(
    register_a_format_without_writer,
    vec![
        finding("no-writer", "compat/sample-cache"),
        finding("missing-head", "compat/sample-cache/head"),
    ],
)]
#[case::shipped_version_without_frozen_set(
    record_a_shipped_version_never_frozen,
    vec![finding("missing-frozen", "compat/sample-store/0.1")],
)]
fn check_fails(#[case] change: fn(&Workspace), #[case] expected: Vec<Finding>) {
    let workspace = Workspace::from_fixture("compat");
    change(&workspace);

    let outcome = workspace.xtask(&["compat", "check"]);

    assert_eq!(outcome.findings(), expected, "{}", outcome.stderr);
    assert_eq!(outcome.code, 1);
}

/// A writer that fails stops the check with its error, whatever the other
/// writers, which run beside it, produce.
#[test]
fn a_failing_writer_fails_the_check_with_its_error() {
    let workspace = Workspace::from_fixture("compat");
    workspace.edit(
        "crates/goodfetch-sample-formats/examples/compat-sample-store.rs",
        "std::fs::write(folder.join(\"store.sql\"), DUMP).expect(\"writing store.sql\");",
        "let _ = (folder, DUMP);\n    eprintln!(\"the scenario broke\");\n    std::process::exit(3);",
    );

    let outcome = workspace.xtask(&["compat", "check"]);

    assert_ne!(outcome.code, 0, "{}", outcome.stdout);
    assert!(
        outcome
            .stderr
            .contains("compat-sample-store failed: the scenario broke"),
        "{}",
        outcome.stderr
    );
}

/// A writer whose bytes differ between two runs fails the check, even when
/// its first run matches the committed set: the check runs every writer
/// twice, so a writer that is right most of the time (#1924's run state, 18
/// runs in 19) cannot pass by chance.
#[test]
fn a_writer_that_differs_between_two_runs_fails_the_check() {
    let workspace = Workspace::from_fixture("compat");
    // The first run writes the committed bytes and leaves a mark; the run
    // after it finds the mark and writes another line.
    workspace.edit(
        "crates/goodfetch-sample-formats/examples/compat-sample-store.rs",
        "std::fs::write(folder.join(\"store.sql\"), DUMP).expect(\"writing store.sql\");",
        "let mark = std::path::Path::new(\"second-run\");\n    \
         let dump = if mark.exists() { format!(\"{DUMP}-- again\\n\") } else { DUMP.to_owned() };\n    \
         std::fs::write(mark, b\"\").expect(\"writing the mark\");\n    \
         std::fs::write(folder.join(\"store.sql\"), dump).expect(\"writing store.sql\");",
    );

    let outcome = workspace.xtask(&["compat", "check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "nondeterministic",
            "compat/sample-store/head/store.sql"
        )],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

const SETTINGS_NAME_TYPE: &str =
    "\"description\": \"The settings' name.\",\n          \"type\": \"string\"";

const SETTINGS_VERSION: &str = "\"storage\": \"json\",\n      \"version\": \"0.1\"";

/// `sample-settings`'s field `name` made nullable: a change of its shape.
const NAME_MADE_NULLABLE: (&str, &str) = (
    SETTINGS_NAME_TYPE,
    "\"description\": \"The settings' name.\",\n          \"type\": [\"string\", \"null\"]",
);

/// The registry's schema of `sample-settings`, whose version 0.1 is frozen,
/// changed without a version bump, then its `head/` set written again.
#[test]
fn schema_changed_without_a_version_bump_fails() {
    let workspace = Workspace::from_fixture("compat");
    workspace.edit(BUNDLE, NAME_MADE_NULLABLE.0, NAME_MADE_NULLABLE.1);
    write(&workspace);

    let outcome = workspace.xtask(&["compat", "check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("unbumped", "compat/sample-settings")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// A schema change with its version bump, or a reworded description, which
/// changes no shape, passes.
#[rstest]
#[case::changed_with_a_version_bump(&[
    NAME_MADE_NULLABLE,
    (SETTINGS_VERSION, "\"storage\": \"json\",\n      \"version\": \"0.2\""),
])]
#[case::description_reworded(&[(SETTINGS_NAME_TYPE, "\"description\": \"What the settings are called.\",\n          \"type\": \"string\"")])]
fn schema_change_the_version_covers_passes(#[case] edits: &[(&str, &str)]) {
    let workspace = Workspace::from_fixture("compat");
    for (from, to) in edits {
        workspace.edit(BUNDLE, from, to);
    }
    write(&workspace);

    let outcome = workspace.xtask(&["compat", "check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

const WRITERS: &str = "crates/goodfetch-sample-formats/examples";

const STORE_VERSION: &str = "\"storage\": \"sqlite\",\n      \"version\": \"0.1\"";

/// Commits the fixture and tags it, then changes `sample-store` on `main`
/// after the tag: a new value in its scenario and a new version.
fn tag_then_change_main(workspace: &Workspace, tag: &str) {
    workspace.commit_all();
    workspace.git(&["tag", tag]);
    workspace.edit(
        &format!("{WRITERS}/compat-sample-store.rs"),
        "X'00ff'",
        "X'00ee'",
    );
    workspace.edit(
        BUNDLE,
        STORE_VERSION,
        "\"storage\": \"sqlite\",\n      \"version\": \"0.2\"",
    );
    write(workspace);
    workspace.commit_all();
}

#[test]
fn freezing_copies_each_shipped_version_from_the_tag_not_from_main() {
    let workspace = Workspace::from_fixture("compat");
    tag_then_change_main(&workspace, "v0.2.0");
    let tagged = workspace.git(&["show", "v0.2.0:compat/sample-store/head/store.sql"]);

    let outcome = workspace.xtask(&["compat", "freeze", "--tag", "v0.2.0"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        workspace.read_text("compat/sample-store/0.1/store.sql"),
        tagged
    );
    assert_eq!(
        workspace.read_text("compat/sample-store/0.1/SHA256SUMS"),
        "d5c3d31c37db82c26e9fe1f4bd5c0d7d14fe6c515c3bfc2b5e7c50d003b92f6e  format.json\n\
         8f8f57fe6901dab0443acb727b6cc5548ce7dfc40b08191121a159e252647a5b  store.sql\n"
    );
    assert!(
        workspace
            .path()
            .join("compat/sample-protocol/0.1/SHA256SUMS")
            .exists()
    );
}

#[test]
fn freezing_leaves_a_frozen_set_as_it_is() {
    let workspace = Workspace::from_fixture("compat");
    workspace.edit(
        &format!("{WRITERS}/compat-sample-settings.rs"),
        "\"pages\": 10",
        "\"pages\": 12",
    );
    write(&workspace);
    let frozen = workspace.read_text("compat/sample-settings/0.1/settings.json");
    tag_then_change_main(&workspace, "v0.2.0");

    let outcome = workspace.xtask(&["compat", "freeze", "--tag", "v0.2.0"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        workspace.read_text("compat/sample-settings/0.1/settings.json"),
        frozen
    );
}

#[rstest]
#[case::release_candidate("v0.2.0-rc.1")]
#[case::nightly("v0.2.0-nightly.20261018")]
fn freezing_from_a_prerelease_freezes_nothing(#[case] tag: &str) {
    let workspace = Workspace::from_fixture("compat");
    tag_then_change_main(&workspace, tag);
    let before = corpus(&workspace);

    let outcome = workspace.xtask(&["compat", "freeze", "--tag", tag]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(corpus(&workspace), before);
}

#[rstest]
#[case::json_property_in_no_document(
    "compat-sample-settings.rs",
    "      \"delay_ms\": 250,\n",
    "",
    vec![finding("coverage", "compat/sample-settings/head:retries[].delay_ms")],
)]
#[case::json_property_only_null(
    "compat-sample-settings.rs",
    "\"proxy\": {\n    \"url\": \"secret://proxy\"\n  },",
    "\"proxy\": null,",
    vec![
        finding("coverage", "compat/sample-settings/head:proxy"),
        finding("coverage", "compat/sample-settings/head:proxy.url"),
    ],
)]
#[case::sqlite_column_null_in_every_row(
    "compat-sample-store.rs",
    "'2026-01-01T00:00:00Z'",
    "NULL",
    vec![finding("coverage", "compat/sample-store/head:runs.finished_at")],
)]
#[case::sqlite_table_without_rows(
    "compat-sample-store.rs",
    "INSERT INTO pages VALUES(1,X'00ff');\n",
    "",
    vec![
        finding("coverage", "compat/sample-store/head:pages.body"),
        finding("coverage", "compat/sample-store/head:pages.run"),
    ],
)]
#[case::message_kind_in_no_transcript(
    "compat-sample-protocol.rs",
    "{\"id\":1,\"result\":{\"text\":\"hello\"}}\n",
    "",
    vec![
        finding("coverage", "compat/sample-protocol/head:result"),
        finding("coverage", "compat/sample-protocol/head:result.text"),
    ],
)]
fn check_fails_on_incomplete_schema_coverage(
    #[case] writer: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] expected: Vec<Finding>,
) {
    let workspace = Workspace::from_fixture("compat");
    workspace.edit(&format!("{WRITERS}/{writer}"), from, to);
    write(&workspace);

    let outcome = workspace.xtask(&["compat", "check"]);

    assert_eq!(outcome.findings(), expected, "{}", outcome.stderr);
    assert_eq!(outcome.code, 1);
}
