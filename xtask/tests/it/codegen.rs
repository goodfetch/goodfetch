//! `xtask codegen` and its drift check (ADRs 0038, 0087 and 0095): the
//! committed bundle, catalog and per-Product outputs, each SDK package's
//! generated layer, and the lists only `xtask codegen` writes: the
//! `wrappers` list and Collie's ban of Foxhound's crates (ADR 0087),
//! cargo-deny's copy of ADR 0018's license allowlist, and the Journey files'
//! schema on its own (ADR 0063). Each codegen test covers story 1-57; the
//! Feature inventory's own tests (#338) declare no Feature, as this spec's
//! stories stay outside the inventory (#325, Out of Scope).

use std::fmt::Write as _;

use rstest::rstest;
use serde_json::Value;

use crate::feature_generators::add_shared_type_values;
use crate::workspace::{Finding, Outcome, Workspace, finding};

const JSON2TS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../node_modules/.bin/json2ts");
const DATAMODEL_CODEGEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../.venv/bin/datamodel-codegen"
);

const CATALOG: &str = "crates/goodfetch-protocol/generated/catalog.json";

fn codegen(workspace: &Workspace) -> Outcome {
    let outcome = workspace.xtask(&[
        "codegen",
        "--json2ts",
        JSON2TS,
        "--datamodel-codegen",
        DATAMODEL_CODEGEN,
    ]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    outcome
}

fn names_and_products(catalog: &Value, section: &str) -> Vec<(String, String)> {
    catalog[section]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["name"].as_str().unwrap().to_owned(),
                entry["product"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// Every file `xtask codegen` writes, with its contents.
fn generated_files(workspace: &Workspace) -> Vec<(String, String)> {
    let mut files = vec![
        ("deny.toml".to_owned(), workspace.read_text("deny.toml")),
        (COLLIE_DENY.to_owned(), workspace.read_text(COLLIE_DENY)),
        (
            JOURNEY_SCHEMA.to_owned(),
            workspace.read_text(JOURNEY_SCHEMA),
        ),
        (SCRIPT_SCHEMA.to_owned(), workspace.read_text(SCRIPT_SCHEMA)),
    ];
    let mut pending: Vec<String> = GENERATED_FOLDERS
        .iter()
        .map(|folder| (*folder).to_owned())
        .collect();
    let mut generated = Vec::new();
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(workspace.path().join(&folder)).unwrap() {
            let entry = entry.unwrap();
            let path = format!("{folder}/{}", entry.file_name().into_string().unwrap());
            if entry.file_type().unwrap().is_dir() {
                pending.push(path);
            } else {
                generated.push(path);
            }
        }
    }
    generated.sort();
    for path in generated {
        let contents = workspace.read_text(&path);
        files.push((path, contents));
    }
    files
}

const GENERATED_FOLDERS: [&str; 10] = [
    "crates/goodfetch-protocol/generated",
    "crates/goodfetch-test-support/generated",
    "xtask/generated",
    "test-support/fixtures/schemas",
    "test-support/typescript/src/generated",
    "test-support/python/src/goodfetch_test_support/generated",
    "sdks/typescript/collie-client/src/generated",
    "sdks/typescript/foxhound/src/generated",
    "sdks/python/collie-client/src/goodfetch_collie_client/generated",
    "sdks/python/foxhound/src/goodfetch_foxhound/generated",
];

/// Covers: story:1-57
#[test]
fn regenerating_twice_gives_identical_output() {
    let workspace = Workspace::from_fixture("codegen");
    codegen(&workspace);
    let first = generated_files(&workspace);

    codegen(&workspace);

    assert_eq!(generated_files(&workspace), first);
}

/// Covers: story:1-57
#[test]
fn check_passes_on_a_clean_tree() {
    let workspace = Workspace::from_fixture("codegen");

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

const SAMPLE: &str = "crates/goodfetch-protocol/src/sample.rs";
const FOXHOUND: &str = "crates/goodfetch-protocol/src/foxhound.rs";
const JOURNEY: &str = "crates/goodfetch-protocol/src/journey.rs";
const JOURNEY_SCHEMA: &str = "journeys/journey.schema.json";
const RESULT_DOCUMENT: &str = "crates/goodfetch-protocol/src/result_document.rs";

/// The fixture's `goodfetch-protocol` holds the protocol crate's `catalog`
/// module, and the two modules it imports, as they are, so the catalog the
/// fixture emits has the shape the real emitter writes and a field the
/// catalog gains reaches these tests (ADR 0038). On a difference, copy the
/// protocol crate's file over the fixture's and regenerate the fixture's
/// committed outputs with `xtask codegen` run in a copy of it.
#[rstest]
#[case::catalog("src/catalog.rs")]
#[case::settings_values("src/catalog/setting.rs")]
#[case::catalog_sections("src/catalog/sections.rs")]
#[case::wire_names("src/wire_name.rs")]
#[case::secret_references("src/secret_reference.rs")]
fn the_fixtures_catalog_module_is_the_protocol_crates_own(#[case] file: &str) {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let protocol = manifest.join("../crates/goodfetch-protocol").join(file);
    let fixture = manifest
        .join("tests/it/fixtures/codegen/crates/goodfetch-protocol")
        .join(file);

    let (own, held) = (
        std::fs::read_to_string(&protocol).unwrap(),
        std::fs::read_to_string(&fixture).unwrap(),
    );

    assert!(
        own == held,
        "{} differs from {}: copy it over, then regenerate the fixture's outputs",
        fixture.display(),
        protocol.display()
    );
}

/// Covers: story:1-57
#[rstest]
#[case::field_added_to_a_collie_type(
    SAMPLE,
    "    /// The text to repeat.\n    pub text: String,\n",
    "    /// The text to repeat.\n    pub text: String,\n    /// How many times to repeat it.\n    pub times: u32,\n",
    vec![
        finding("drift", "crates/goodfetch-protocol/generated/bundle.json"),
        finding("drift", "crates/goodfetch-protocol/generated/collie.json"),
        finding("drift", "crates/goodfetch-protocol/generated/foxhound.json"),
        finding("stale", "sdks/typescript/collie-client/src/generated/types.ts"),
        finding("stale", "sdks/python/collie-client/src/goodfetch_collie_client/generated/models.py"),
    ],
)]
#[case::metric_renamed(
    SAMPLE,
    "\"goodfetch.sample.echoes\"",
    "\"goodfetch.sample.repeats\"",
    vec![
        finding("drift", "crates/goodfetch-protocol/generated/catalog.json"),
        finding("drift", "crates/goodfetch-protocol/generated/collie.json"),
        finding("drift", "crates/goodfetch-protocol/generated/foxhound.json"),
    ],
)]
#[case::foxhound_operation_renamed(
    FOXHOUND,
    "\"sample.crawl\"",
    "\"sample.visit\"",
    vec![
        finding("drift", "crates/goodfetch-protocol/generated/catalog.json"),
        finding("drift", "crates/goodfetch-protocol/generated/foxhound.json"),
        finding("drift", "crates/goodfetch-protocol/generated/foxhound_names.rs"),
        finding("drift", "features/inventory.json"),
        finding("drift", "sdks/python/foxhound/src/goodfetch_foxhound/generated/operations.py"),
        finding("drift", "sdks/typescript/foxhound/src/generated/operations.ts"),
    ],
)]
#[case::field_added_to_the_journey_type(
    JOURNEY,
    "    /// What the person achieves.\n    pub goal: String,\n",
    "    /// What the person achieves.\n    pub goal: String,\n    /// Its Product.\n    pub product: String,\n",
    vec![
        finding("drift", "crates/goodfetch-protocol/generated/bundle.json"),
        finding("drift", JOURNEY_SCHEMA),
    ],
)]
#[case::field_added_to_the_result_document(
    RESULT_DOCUMENT,
    "    /// Its outcome.\n    pub outcome: String,\n",
    "    /// Its outcome.\n    pub outcome: String,\n    /// Its surface.\n    pub surface: String,\n",
    vec![
        finding("drift", "crates/goodfetch-protocol/generated/bundle.json"),
        finding("stale", "test-support/python/src/goodfetch_test_support/generated/result_document.py"),
        finding("stale", "test-support/typescript/src/generated/result-document.ts"),
    ],
)]
#[case::field_added_to_the_case_log_entry(
    RESULT_DOCUMENT,
    "    /// The case's ID.\n    pub case: String,\n",
    "    /// The case's ID.\n    pub case: String,\n    /// Its mode.\n    pub mode: String,\n",
    vec![
        finding("drift", "crates/goodfetch-protocol/generated/bundle.json"),
        finding("stale", "test-support/python/src/goodfetch_test_support/generated/result_document.py"),
        finding("stale", "test-support/typescript/src/generated/result-document.ts"),
    ],
)]
fn check_fails_after_a_type_change_that_was_not_regenerated(
    #[case] source: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] expected: Vec<Finding>,
) {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(source, from, to);

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(outcome.findings(), expected, "{}", outcome.stderr);
    assert_eq!(outcome.code, 1);
}

/// The Feature inventory follows the stories and rows files it reads, as it
/// follows the types (ADR 0072).
/// Covers: story:1-57
#[rstest]
#[case::story_added(
    "features/stories/1.toml",
    "not_journey = \"actor\"\nstatus = \"active\"\n",
    "not_journey = \"actor\"\nstatus = \"active\"\n\n[[story]]\nid = 3\ntitle = \"A new story.\"\nmilestone = \"M1 Core\"\njourney = true\nstatus = \"active\"\n"
)]
#[case::row_given_another_milestone("features/rows.toml", "M1 Core", "M2 Next")]
fn check_fails_after_a_feature_source_change_that_was_not_regenerated(
    #[case] source: &str,
    #[case] from: &str,
    #[case] to: &str,
) {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(source, from, to);

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("drift", "features/inventory.json")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// An outside generator's output depends on its version, which the lockfile
/// pins, so a bump that was not regenerated is stale, though no type changed.
/// Covers: story:1-57
#[rstest]
#[case::json_schema_to_typescript(
    "pnpm-lock.yaml",
    "version: 16.0.0",
    "version: 16.1.0",
    vec![
        finding("stale", "sdks/typescript/collie-client/src/generated/types.ts"),
        finding("stale", "sdks/typescript/foxhound/src/generated/types.ts"),
        finding("stale", "test-support/typescript/src/generated/result-document.ts"),
    ],
)]
#[case::datamodel_code_generator(
    "uv.lock",
    "version = \"0.86.0\"",
    "version = \"0.87.0\"",
    vec![
        finding("stale", "sdks/python/collie-client/src/goodfetch_collie_client/generated/models.py"),
        finding("stale", "sdks/python/foxhound/src/goodfetch_foxhound/generated/models.py"),
        finding("stale", "test-support/python/src/goodfetch_test_support/generated/result_document.py"),
    ],
)]
#[case::ruff_which_formats_the_python_models(
    "uv.lock",
    "version = \"0.17.1\"",
    "version = \"0.18.0\"",
    vec![
        finding("stale", "sdks/python/collie-client/src/goodfetch_collie_client/generated/models.py"),
        finding("stale", "sdks/python/foxhound/src/goodfetch_foxhound/generated/models.py"),
        finding("stale", "test-support/python/src/goodfetch_test_support/generated/result_document.py"),
    ],
)]
fn check_fails_after_a_generator_version_bump_that_was_not_regenerated(
    #[case] lockfile: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] expected: Vec<Finding>,
) {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(lockfile, from, to);

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(outcome.findings(), expected, "{}", outcome.stderr);
    assert_eq!(outcome.code, 1);
}

/// Covers: story:1-57
#[rstest]
#[case::generator_output("sdks/typescript/collie-client/src/generated/types.ts", "edited")]
#[case::method_layer(
    "sdks/python/collie-client/src/goodfetch_collie_client/generated/operations.py",
    "drift"
)]
#[case::catalog("crates/goodfetch-protocol/generated/catalog.json", "drift")]
#[case::feature_inventory("features/inventory.json", "drift")]
#[case::stories_schema("features/stories.schema.json", "drift")]
#[case::journey_schema(JOURNEY_SCHEMA, "drift")]
#[case::canary_rule(CANARY_RULE, "drift")]
fn check_detects_a_hand_edit_under_generated(#[case] path: &str, #[case] kind: &str) {
    let workspace = Workspace::from_fixture("codegen");
    let edited = format!("{}\n", workspace.read_text(path));
    workspace.write(path, edited.as_bytes());

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(kind, path)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// The type `goodfetch-test-env` describes a test's setup with, in the
/// fixture's stand-in of test support.
const DESCRIPTION: &str = "crates/goodfetch-test-support/src/lib.rs";
const DESCRIPTION_SCHEMA: &str = "test-support/fixtures/schemas/test-env-description.schema.json";

/// Covers: story:1-57
#[test]
fn check_fails_when_the_committed_description_schema_drifts_from_the_type() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(
        DESCRIPTION,
        "    pub seed: u64,\n",
        "    pub seed: u64,\n    /// The test's label.\n    pub label: String,\n",
    );

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("drift", DESCRIPTION_SCHEMA)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// The script format's schema, which `xtask codegen` commits among the
/// engine-script fixtures rather than in `schemas/` (ADR 0069).
const SCRIPT_SCHEMA: &str = "test-support/fixtures/engine-scripts/script.schema.json";

/// Covers: story:1-57
#[test]
fn check_fails_when_the_committed_script_schema_drifts_from_the_type() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(
        DESCRIPTION,
        "    pub description: String,\n",
        "    pub description: String,\n    /// Its steps.\n    pub steps: Vec<String>,\n",
    );

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("drift", SCRIPT_SCHEMA)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// Covers: story:1-57
#[test]
fn codegen_writes_the_script_schema_among_the_engine_scripts() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.remove(SCRIPT_SCHEMA);

    codegen(&workspace);

    let schema: Value = serde_json::from_str(&workspace.read_text(SCRIPT_SCHEMA)).unwrap();
    assert_eq!(schema["title"], "Script");
}

/// The Secret canaries' `gitleaks` rule, which the fixture's stand-in of
/// test support generates from its kinds.
const CANARY_RULE: &str = "test-support/fixtures/canaries/gitleaks.toml";

/// Covers: story:1-57
#[test]
fn check_fails_when_the_committed_canary_rule_drifts_from_the_kinds() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(
        DESCRIPTION,
        "pub const KINDS: [(&str, &str); 1] = [(\"typed-password\", \"gfcanary-password-[0-9a-f]{16}\")];",
        "pub const KINDS: [(&str, &str); 2] = [\n    (\"typed-password\", \"gfcanary-password-[0-9a-f]{16}\"),\n    (\"endpoint-token\", \"gfcanary-token-[0-9a-f]{32}\"),\n];",
    );

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(
        outcome.findings(),
        vec![
            finding("drift", CANARY_RULE),
            finding("drift", PYTHON_SECRET_KINDS),
            finding("drift", TYPESCRIPT_SECRET_KINDS),
        ],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// The Secret canary kinds `xtask codegen` writes for the Python and
/// TypeScript helpers from test support's.
const PYTHON_SECRET_KINDS: &str =
    "test-support/python/src/goodfetch_test_support/generated/secret_kinds.py";
const TYPESCRIPT_SECRET_KINDS: &str = "test-support/typescript/src/generated/secret-kinds.ts";

/// The facts `xtask codegen` writes for test support and for `xtask` from
/// their sources in Rust.
const TEST_SUPPORT_FACTS: &str = "crates/goodfetch-test-support/generated/facts.rs";
const XTASK_FACTS: &str = "xtask/generated/test_facts.rs";

/// Covers: story:1-57
#[rstest]
#[case::chrome_for_testing_pin(
    "crates/goodfetch-collie/src/pins.json",
    "\"1.0.0.1\"",
    &[TEST_SUPPORT_FACTS, XTASK_FACTS]
)]
#[case::lightpanda_pin(
    "crates/foxhound/goodfetch-fetch-tier/src/light_browser/pins.json",
    "\"0.0.1\"",
    &[TEST_SUPPORT_FACTS]
)]
#[case::playwright_revision(
    "crates/goodfetch-test-support/src/browsers.rs",
    "\"1\"",
    &[XTASK_FACTS]
)]
#[case::dispatch_target(
    "crates/goodfetch-engine/src/rpc.rs",
    "\"fixture::rpc::dispatch\"",
    &[TEST_SUPPORT_FACTS, XTASK_FACTS]
)]
#[case::outcome_target(
    "crates/goodfetch-engine/src/runs/outcome.rs",
    "\"fixture::run::outcome\"",
    &[TEST_SUPPORT_FACTS, XTASK_FACTS]
)]
#[case::command_target(
    "crates/goodfetch-test-support/src/exercise.rs",
    "\"fixture::test::command\"",
    &[XTASK_FACTS]
)]
fn check_fails_when_a_fact_changes_at_its_source_and_was_not_regenerated(
    #[case] source: &str,
    #[case] value: &str,
    #[case] readers: &[&str],
) {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(source, value, "\"changed\"");

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(
        outcome.findings(),
        readers
            .iter()
            .map(|reader| finding("drift", reader))
            .collect::<Vec<_>>(),
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// A file `xtask codegen` no longer writes, in the folder that holds the
/// special-purpose table, is drift: `--check` names it and a run removes it,
/// as in every folder it owns.
///
/// Covers: story:1-57
#[test]
fn a_stray_file_beside_the_special_purpose_table_is_flagged_and_removed() {
    let workspace = Workspace::from_fixture("codegen");
    let stray = "crates/goodfetch-engine/generated/stray.rs";
    workspace.write(stray, b"// nothing writes this\n");

    let checked = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(checked.findings(), vec![finding("unexpected", stray)]);
    assert_eq!(checked.code, 1);

    codegen(&workspace);

    assert!(workspace.read(stray).is_none());
}

/// Covers: story:1-57
#[test]
fn codegen_writes_a_fact_its_source_gives_to_each_of_its_files() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(
        "crates/goodfetch-collie/src/pins.json",
        "\"1.0.0.1\"",
        "\"2.0.0.2\"",
    );

    codegen(&workspace);

    assert!(
        workspace
            .read_text(TEST_SUPPORT_FACTS)
            .contains("pub const CHROME_FOR_TESTING_BUILD: &str = \"2.0.0.2\";\n"),
        "{}",
        workspace.read_text(TEST_SUPPORT_FACTS)
    );
    assert!(
        workspace
            .read_text(XTASK_FACTS)
            .contains("pub(crate) const CHROME_FOR_TESTING_BUILD: &str = \"2.0.0.2\";\n"),
        "{}",
        workspace.read_text(XTASK_FACTS)
    );
}

/// Covers: story:1-57
#[test]
fn codegen_fails_naming_a_fact_its_source_no_longer_holds() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(
        "crates/goodfetch-engine/src/rpc.rs",
        "DISPATCH_TARGET",
        "RENAMED_TARGET",
    );

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
    assert!(
        outcome.stderr.contains(
            "crates/goodfetch-engine/src/rpc.rs has no string constant `DISPATCH_TARGET`"
        ),
        "{}",
        outcome.stderr
    );
}

/// Covers: story:1-57
#[test]
fn codegen_writes_the_canary_rule_test_env_prints() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.remove(CANARY_RULE);

    codegen(&workspace);

    assert_eq!(
        workspace.read_text(CANARY_RULE),
        "[[rules]]\nid = \"typed-password\"\nregex = '''gfcanary-password-[0-9a-f]{16}'''\n"
    );
}

/// Covers: story:1-57
#[test]
fn codegen_writes_the_description_schema_test_env_prints() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.remove(DESCRIPTION_SCHEMA);

    codegen(&workspace);

    let schema: Value = serde_json::from_str(&workspace.read_text(DESCRIPTION_SCHEMA)).unwrap();
    assert_eq!(schema["title"], "Description");
}

/// Covers: story:1-57
#[test]
fn check_detects_a_file_added_under_generated() {
    let workspace = Workspace::from_fixture("codegen");
    let path = "sdks/typescript/foxhound/src/generated/helpers.ts";
    workspace.write(path, b"export const helper = 1;\n");

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("unexpected", path)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// Covers: story:1-57
#[test]
fn catalog_marks_each_entry_with_the_product_whose_module_defines_it() {
    let workspace = Workspace::from_fixture("codegen");

    codegen(&workspace);

    let catalog: Value = serde_json::from_str(&workspace.read_text(CATALOG)).unwrap();
    assert_eq!(
        names_and_products(&catalog, "operations"),
        vec![
            ("sample.crawl".to_owned(), "foxhound".to_owned()),
            ("sample.echo".to_owned(), "collie".to_owned()),
        ]
    );
}

/// Each matrix row but the excluded ones, each story and each contract of
/// the catalog is a Feature, with its area and milestone (ADR 0072).
#[test]
fn inventory_lists_every_feature_of_the_fixture_with_its_area_and_milestone() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.remove("features/inventory.json");

    codegen(&workspace);

    let inventory: Value =
        serde_json::from_str(&workspace.read_text("features/inventory.json")).unwrap();
    let features: Vec<(String, Value, Value)> = inventory["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|feature| {
            (
                feature["id"].as_str().unwrap().to_owned(),
                feature["area"].clone(),
                feature["milestone"].clone(),
            )
        })
        .collect();
    let feature = |id: &str, area: u32, milestone: Option<&str>| {
        (
            id.to_owned(),
            Value::from(area),
            milestone.map_or(Value::Null, Value::from),
        )
    };
    assert_eq!(
        features,
        vec![
            feature("error:sample_crawl_refused", 1, None),
            feature("error:sample_refused", 1, None),
            feature("event:sample.crawled", 1, None),
            feature("event:sample.echoed", 1, None),
            feature("format:sample-store", 1, None),
            feature("operation:sample.crawl", 1, None),
            feature("operation:sample.echo", 1, None),
            feature("row:RT-01", 3, Some("M1 Core")),
            feature("row:RT-02", 3, None),
            feature("row:durability-resume", 3, None),
            feature("story:1-1", 1, Some("M1 Core")),
            feature("story:1-2", 1, Some("M2 Next")),
        ]
    );
}

/// A Feature named in the table of Foxhound's names is Foxhound's alone;
/// every other is both Products' (ADRs 0087 and 0094).
#[test]
fn inventory_records_the_products_that_carry_each_feature() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.remove("features/inventory.json");

    codegen(&workspace);

    let inventory: Value =
        serde_json::from_str(&workspace.read_text("features/inventory.json")).unwrap();
    let products = |id: &str| -> Value {
        inventory["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|feature| feature["id"] == id)
            .unwrap()["products"]
            .clone()
    };
    let foxhound = serde_json::json!(["foxhound"]);
    let both = serde_json::json!(["collie", "foxhound"]);
    assert_eq!(
        [
            products("operation:sample.crawl"),
            products("error:sample_crawl_refused"),
            products("event:sample.crawled"),
            products("operation:sample.echo"),
            products("error:sample_refused"),
            products("event:sample.echoed"),
            products("story:1-1"),
            products("row:RT-01"),
        ],
        [
            foxhound.clone(),
            foxhound.clone(),
            foxhound,
            both.clone(),
            both.clone(),
            both.clone(),
            both.clone(),
            both,
        ]
    );
}

/// The Connection's own messages (`hello`, `$/cancel`), which every engine
/// speaks, are types of Collie's client package, whose hand-written runtime
/// sends them, and never of Foxhound's layer.
/// Covers: story:1-57
#[rstest]
#[case::typescript_collie_client("sdks/typescript/collie-client/src/generated/types.ts", true)]
#[case::typescript_foxhound("sdks/typescript/foxhound/src/generated/types.ts", false)]
#[case::python_collie_client(
    "sdks/python/collie-client/src/goodfetch_collie_client/generated/models.py",
    true
)]
#[case::python_foxhound(
    "sdks/python/foxhound/src/goodfetch_foxhound/generated/models.py",
    false
)]
fn the_connections_messages_are_types_of_the_collie_client_package_only(
    #[case] path: &str,
    #[case] holds: bool,
) {
    let workspace = Workspace::from_fixture("codegen");

    codegen(&workspace);

    let types = workspace.read_text(path);
    assert_eq!(
        types.contains("class HelloParams") || types.contains("interface HelloParams"),
        holds,
        "{types}"
    );
}

/// Each package spells its own Product's values of a type both Products
/// share (ADR 0095), Foxhound's on top of Collie's, and keeps a value it
/// does not know as received (ADR 0079).
/// Covers: story:1-57
#[rstest]
#[case::typescript_collie_client(
    "sdks/typescript/collie-client/src/generated/shared-types.ts",
    r#"export type BlockCause = "captcha" | "challenge" | (string & {});"#
)]
#[case::typescript_foxhound(
    "sdks/typescript/foxhound/src/generated/shared-types.ts",
    r#"export type BlockCause = CollieBlockCause | "decoy";"#
)]
#[case::typescript_foxhound_own_type(
    "sdks/typescript/foxhound/src/generated/shared-types.ts",
    r#"export type UsageKind = "sample_crawled_pages" | (string & {});"#
)]
#[case::python_collie_client(
    "sdks/python/collie-client/src/goodfetch_collie_client/generated/shared_types.py",
    r#"BlockCause: TypeAlias = Literal["captcha", "challenge"] | str"#
)]
#[case::python_foxhound(
    "sdks/python/foxhound/src/goodfetch_foxhound/generated/shared_types.py",
    r#"BlockCause: TypeAlias = CollieBlockCause | Literal["decoy"]"#
)]
#[case::python_foxhound_own_type(
    "sdks/python/foxhound/src/goodfetch_foxhound/generated/shared_types.py",
    r#"UsageKind: TypeAlias = Literal["sample_crawled_pages"] | str"#
)]
fn each_sdk_layer_spells_its_products_values_of_a_shared_type_and_keeps_an_unknown_one(
    #[case] path: &str,
    #[case] declaration: &str,
) {
    let workspace = Workspace::from_fixture("codegen");
    add_shared_type_values(
        &workspace,
        &[
            ("challenge", "collie", "block_cause"),
            ("captcha", "collie", "block_cause"),
            ("decoy", "foxhound", "block_cause"),
            ("sample_crawled_pages", "foxhound", "usage_kind"),
        ],
    );

    codegen(&workspace);

    let shared_types = workspace
        .read(path)
        .map(|bytes| String::from_utf8(bytes).unwrap());
    assert!(
        shared_types
            .as_deref()
            .is_some_and(|text| text.contains(declaration)),
        "{shared_types:?}"
    );
}

/// The layer files `xtask codegen` writes itself, as a reviewer reads them
/// (TESTING.md, Snapshots: generated surfaces); the outside generators'
/// files are held to their recorded digests instead.
/// Covers: story:1-57
#[rstest]
#[case::typescript_collie_client("sdks/typescript/collie-client/src/generated")]
#[case::typescript_foxhound("sdks/typescript/foxhound/src/generated")]
#[case::python_collie_client("sdks/python/collie-client/src/goodfetch_collie_client/generated")]
#[case::python_foxhound("sdks/python/foxhound/src/goodfetch_foxhound/generated")]
fn each_sdk_layer_is_generated_as_its_snapshot(#[case] folder: &str) {
    let workspace = Workspace::from_fixture("codegen");

    codegen(&workspace);

    let extension = if folder.contains("typescript") {
        "ts"
    } else {
        "py"
    };
    let mut layer = String::new();
    for name in [
        "index",
        "__init__",
        "connection",
        "shared-types",
        "shared_types",
        "operations",
        "events",
        "errors",
        "plugins",
    ] {
        let path = format!("{folder}/{name}.{extension}");
        if let Some(contents) = workspace.read(&path) {
            let _ = writeln!(layer, "==> {name}.{extension}");
            layer.push_str(&String::from_utf8(contents).unwrap());
        }
    }
    insta::assert_snapshot!(folder.replace('/', "__"), layer);
}

/// A name no SDK can spell by its one rule is refused, so the catalog
/// renames the entry for every surface and no language needs an alias
/// (ADRs 0079 and 0099): a member TypeScript, Python or every Rust handle
/// gives meaning on every value, `call`, which is every handle's raw call, a
/// name Rust cannot make raw, and an error class that would shadow a
/// built-in one.
///
/// Covers: story:11-176
#[rstest]
#[case::then("\"sample.echo\"", "\"sample.then\"", "operation `sample.then`")]
#[case::constructor(
    "\"sample.echo\"",
    "\"sample.constructor\"",
    "operation `sample.constructor`"
)]
#[case::call("\"sample.echo\"", "\"sample.call\"", "operation `sample.call`")]
#[case::plugin_method_then(
    "name: \"put\"",
    "name: \"then\"",
    "Plugin kind method `sample_store.then`"
)]
#[case::python_dunder(
    "\"sample.echo\"",
    "\"sample.__init__\"",
    "operation `sample.__init__`"
)]
#[case::rust_self("\"sample.echo\"", "\"sample.self\"", "operation `sample.self`")]
#[case::rust_super("\"sample.echo\"", "\"sample.super\"", "operation `sample.super`")]
#[case::rust_crate("\"sample.echo\"", "\"sample.crate\"", "operation `sample.crate`")]
#[case::rust_self_type("\"sample.echoed\"", "\"sample.self\"", "event `sample.self`")]
#[case::rust_clone("\"sample.echo\"", "\"sample.clone\"", "operation `sample.clone`")]
#[case::rust_eq("\"sample.echo\"", "\"sample.eq\"", "operation `sample.eq`")]
#[case::rust_fmt("\"sample.echo\"", "\"sample.fmt\"", "operation `sample.fmt`")]
#[case::rust_hash("\"sample.echo\"", "\"sample.hash\"", "operation `sample.hash`")]
#[case::plugin_method_clone(
    "name: \"put\"",
    "name: \"clone\"",
    "Plugin kind method `sample_store.clone`"
)]
#[case::timeout_error("\"sample_refused\"", "\"timeout\"", "Error code `timeout`")]
#[case::connection_error("\"sample_refused\"", "\"connection\"", "Error code `connection`")]
#[case::permission_error("\"sample_refused\"", "\"permission\"", "Error code `permission`")]
#[case::abort_error("\"sample_refused\"", "\"abort\"", "Error code `abort`")]
#[case::type_error("\"sample_refused\"", "\"type\"", "Error code `type`")]
fn codegen_refuses_a_name_no_sdk_can_spell_naming_its_entry(
    #[case] from: &str,
    #[case] to: &str,
    #[case] entry: &str,
) {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(SAMPLE, from, to);

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
    assert!(outcome.stderr.contains(entry), "{}", outcome.stderr);
}

/// Two entries that would give one receiver one member are refused across a
/// Product's whole catalog: Foxhound's holds Collie's `sample.to_http_s` beside
/// its own `sample.to_https`, which both spell `toHTTPS`.
///
/// Covers: story:11-176
#[test]
fn codegen_refuses_two_entries_that_give_one_receiver_one_member() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(SAMPLE, "\"sample.echo\"", "\"sample.to_http_s\"");
    workspace.edit(FOXHOUND, "\"sample.crawl\"", "\"sample.to_https\"");

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
    assert!(
        outcome
            .stderr
            .contains("`sample.to_http_s` and `sample.to_https` are one member"),
        "{}",
        outcome.stderr
    );
}

/// No entry has a second name in TypeScript (ADR 0099): each operation is
/// sent by one member of its layer, and no export renames another.
///
/// Covers: story:11-176
#[rstest]
#[case::collie_client(
    "sdks/typescript/collie-client/src/generated",
    "crates/goodfetch-protocol/generated/collie.json",
    "collie"
)]
#[case::foxhound(
    "sdks/typescript/foxhound/src/generated",
    "crates/goodfetch-protocol/generated/foxhound.json",
    "foxhound"
)]
fn the_typescript_layer_holds_no_alias(
    #[case] folder: &str,
    #[case] projection: &str,
    #[case] product: &str,
) {
    let workspace = Workspace::from_fixture("codegen");
    codegen(&workspace);
    let projection: Value = serde_json::from_str(&workspace.read_text(projection)).unwrap();
    let operations = workspace.read_text(&format!("{folder}/operations.ts"));

    let senders: Vec<(String, usize)> = names_and_products(&projection, "operations")
        .into_iter()
        .filter(|(_, entry_product)| entry_product == product)
        .map(|(name, _)| {
            let sent = operations
                .matches(&format!("this.#caller.call({name:?},"))
                .count();
            (name, sent)
        })
        .filter(|(_, sent)| *sent != 1)
        .collect();
    let renamed: Vec<(String, String)> = generated_files(&workspace)
        .into_iter()
        .filter(|(path, _)| path.starts_with(folder))
        .flat_map(|(path, contents)| {
            contents
                .lines()
                .filter(|line| line.starts_with("export") && line.contains(" as "))
                .map(|line| (path.clone(), line.to_owned()))
                .collect::<Vec<_>>()
        })
        .collect();

    assert_eq!((senders, renamed), (vec![], vec![]));
}

/// No entry has a second name in Python (ADR 0099): each operation is sent
/// by one member of each class of its layer, the async one and its sync
/// twin, and a module imports another Product's entry
/// under a second name only to widen its own entry of the first name.
///
/// Covers: story:11-176
#[rstest]
#[case::collie_client(
    "sdks/python/collie-client/src/goodfetch_collie_client/generated",
    "crates/goodfetch-protocol/generated/collie.json",
    "collie"
)]
#[case::foxhound(
    "sdks/python/foxhound/src/goodfetch_foxhound/generated",
    "crates/goodfetch-protocol/generated/foxhound.json",
    "foxhound"
)]
fn the_python_layer_holds_no_alias(
    #[case] folder: &str,
    #[case] projection: &str,
    #[case] product: &str,
) {
    let workspace = Workspace::from_fixture("codegen");
    codegen(&workspace);
    let projection: Value = serde_json::from_str(&workspace.read_text(projection)).unwrap();
    let operations = workspace.read_text(&format!("{folder}/operations.py"));
    let classes: Vec<&str> = operations
        .split("\nclass ")
        .filter(|class| class.contains("self._caller.call("))
        .collect();

    let senders: Vec<(String, Vec<usize>)> = names_and_products(&projection, "operations")
        .into_iter()
        .filter(|(_, entry_product)| entry_product == product)
        .map(|(name, _)| {
            let sent = classes
                .iter()
                .map(|class| {
                    class
                        .matches(&format!("self._caller.call({name:?},"))
                        .count()
                })
                .collect();
            (name, sent)
        })
        .filter(|(_, sent): &(String, Vec<usize>)| sent.iter().any(|count| *count != 1))
        .collect();
    let renamed: Vec<(String, String)> = generated_files(&workspace)
        .into_iter()
        .filter(|(path, _)| path.starts_with(folder))
        .flat_map(|(path, contents)| {
            let imported: Vec<(String, String)> = contents
                .lines()
                .filter_map(|line| line.strip_prefix("from ")?.split_once(" import "))
                .flat_map(|(_, names)| names.split(", "))
                .filter_map(|name| name.split_once(" as "))
                .map(|(first, second)| (first.to_owned(), second.to_owned()))
                .collect();
            imported
                .into_iter()
                .filter(|(first, second)| {
                    first != second
                        && !contents.contains(&format!("\n{first}: TypeAlias = {second} | "))
                })
                .map(|(first, second)| (path.clone(), format!("{first} as {second}")))
                .collect::<Vec<_>>()
        })
        .collect();

    assert_eq!((senders, renamed), (vec![], vec![]));
}

/// No generated Rust file gives an item a second name (ADR 0099).
///
/// Covers: story:11-176
#[test]
fn the_generated_rust_holds_no_alias() {
    let workspace = Workspace::from_fixture("codegen");
    codegen(&workspace);

    let renamed: Vec<(String, String)> = generated_files(&workspace)
        .into_iter()
        .filter(|(path, _)| {
            std::path::Path::new(path)
                .extension()
                .is_some_and(|extension| extension == "rs")
        })
        .flat_map(|(path, contents)| {
            contents
                .lines()
                .map(str::trim_start)
                .filter(|line| {
                    (line.starts_with("pub use ") && line.contains(" as "))
                        || line.starts_with("pub type ")
                })
                .map(|line| (path.clone(), line.to_owned()))
                .collect::<Vec<_>>()
        })
        .collect();

    assert_eq!(renamed, vec![]);
}

const EMIT: &str = "crates/goodfetch-protocol/src/emit.rs";
const COLLIE_CLIENT: &str = "sdks/typescript/collie-client";

/// The `codegen` workspace with the forms catalog (`codegen-forms/forms.rs`)
/// among its entry modules: one entry of each kind an SDK gives a form.
fn workspace_with_the_forms_catalog() -> Workspace {
    let workspace = Workspace::from_fixture("codegen");
    workspace.add_fixture_file(
        "codegen-forms/forms.rs",
        "crates/goodfetch-protocol/src/forms.rs",
    );
    workspace.edit(
        "crates/goodfetch-protocol/src/lib.rs",
        "pub mod foxhound;",
        "pub mod forms;\npub mod foxhound;",
    );
    workspace.edit(
        EMIT,
        "const MODULES: [&Entries; 2] = [&sample::ENTRIES, &foxhound::ENTRIES];",
        "const MODULES: [&Entries; 3] = [&sample::ENTRIES, &crate::forms::ENTRIES, &foxhound::ENTRIES];",
    );
    for prefix in ["run.", "page.", "locator."] {
        workspace.append(
            "features/areas.toml",
            &format!("[[area]]\nnumber = 1\nkind = \"operation\"\nprefix = \"{prefix}\"\n"),
        );
    }
    workspace.append(
        "features/areas.toml",
        "[[area]]\nnumber = 1\nkind = \"event\"\nprefix = \"run.\"\n",
    );
    workspace
}

/// A prefix keeps its one meaning and a query never waits (ADR 0099), a
/// result is typed by fields a call declares (ADR 0147), and a call's own
/// `timeout` names the setting it sets (ADR 0080), and a deprecated
/// parameter or value leaves a call free of it (ADR 0099), and a read
/// takes nothing but its handle and answers a value of the answer that
/// opened it (issue 2798): `xtask codegen` refuses a catalog that says
/// otherwise, naming the operation, in place of writing a member whose
/// name, options or result lie.
///
/// Covers: story:11-178
#[rstest]
#[case::a_waiter_without_the_prefix(
    "\"page.wait_for_ready\"",
    "\"page.ready\"",
    "operation `page.ready`"
)]
#[case::the_prefix_on_a_round_trip(
    "\"page.close\"",
    "\"page.wait_for_close\"",
    "operation `page.wait_for_close`"
)]
#[case::a_query_with_a_timeout(
    "result: schema_of::<Readiness>,",
    "result: schema_of::<Readiness>,\n            params: schema_of::<WaitForReadyParams>,",
    "operation `page.is_ready`"
)]
#[case::new_on_a_query(
    "\"page.is_ready\"",
    "\"page.new_ready\"",
    "operation `page.new_ready`"
)]
#[case::is_on_a_round_trip("\"page.close\"", "\"page.is_closed\"", "operation `page.is_closed`")]
#[case::a_read_its_opener_does_not_answer(
    "pub struct RunLabel {\n    /// The Run's label.\n    pub label: String,",
    "pub struct RunLabel {\n    /// The Run's label.\n    pub name: String,",
    "operation `run.label`"
)]
#[case::a_read_of_another_type_than_its_opener_answers(
    "pub struct RunLabel {\n    /// The Run's label.\n    pub label: String,",
    "pub struct RunLabel {\n    /// The Run's label.\n    pub label: u64,",
    "operation `run.label`"
)]
#[case::a_read_whose_result_has_two_properties(
    "pub struct RunLabel {\n    /// The Run's label.\n    pub label: String,",
    "pub struct RunLabel {\n    /// The Run's label.\n    pub label: String,\n    /// A second value a read cannot answer.\n    pub other: u64,",
    "operation `run.label`: a read answers a value of the answer that opened its handle, without a message, so its result has one property"
)]
#[case::a_read_whose_result_property_is_optional(
    "pub struct RunLabel {\n    /// The Run's label.\n    pub label: String,",
    "pub struct RunLabel {\n    /// The Run's label.\n    pub label: Option<String>,",
    "operation `run.label`: a read answers a value of the answer that opened its handle, without a message, so its result's `label` is required"
)]
#[case::a_read_with_a_parameter(
    "pub struct RunParams {\n    /// The Run.\n    pub run_id: RunId,\n",
    "pub struct RunParams {\n    /// The Run.\n    pub run_id: RunId,\n    /// A parameter a read cannot take.\n    pub fresh: bool,\n",
    "operation `run.is_resumed`"
)]
#[case::on_on_a_round_trip("\"page.close\"", "\"page.on_close\"", "operation `page.on_close`")]
#[case::get_by_on_an_operation(
    "\"page.is_ready\"",
    "\"page.get_by_ready\"",
    "operation `page.get_by_ready`"
)]
#[case::typed_by_no_parameter(
    "extend(\"typed_by\" = \"fields\")",
    "extend(\"typed_by\" = \"names\")",
    "operation `page.read`"
)]
#[case::typed_by_an_optional_parameter(
    "pub fields: BTreeMap<String, PageField>,",
    "pub fields: Option<BTreeMap<String, PageField>>,",
    "operation `page.read`"
)]
#[case::typed_by_on_two_properties(
    "pub unread: Vec<String>,",
    "#[schemars(extend(\"typed_by\" = \"fields\"))]\n    pub unread: Vec<String>,",
    "operation `page.read`"
)]
#[case::typed_by_on_an_opened_value(
    "pub struct PageOpened {\n    /// The page.\n",
    "pub struct PageOpened {\n    /// The page.\n    #[schemars(extend(\"typed_by\" = \"run_id\"))]\n",
    "operation `run.new_page`"
)]
#[case::typed_by_on_a_result_of_one_property(
    "    /// The names of the fields the page did not give.\n    pub unread: Vec<String>,\n",
    "",
    "operation `page.read`: its result's one property"
)]
#[case::a_handle_id_without_a_type(
    "pub page_id: PageId,\n    /// The URL to navigate to.",
    "pub page_id: String,\n    /// The URL to navigate to.",
    "operation `page.goto`"
)]
#[case::a_timeout_naming_no_setting(
    "#[schemars(extend(\"setting\" = \"timeout.navigation\"))]\n",
    "",
    "operation `page.goto`"
)]
/// Covers: story:11-182
#[case::a_deprecated_required_parameter(
    "    /// The URL to navigate to.\n",
    "    /// The URL to navigate to.\n    #[schemars(extend(\"deprecation\" = {\"since\": \"0.1\", \"replacement\": null, \"earliest_removal\": \"1.0\", \"note\": \"\"}))]\n",
    "operation `page.goto`: its required parameter `url` is deprecated"
)]
/// Covers: story:11-182
#[case::every_value_deprecated(
    "    /// Parsed.\n    Interactive,\n    /// Loaded, with its subresources.\n    Complete,\n",
    "",
    "operation `page.wait_for_ready`: every value of its parameter `state` is deprecated"
)]
/// Covers: story:11-182
#[case::a_deprecated_value_that_is_no_string(
    "    Loaded,\n}",
    "    Loaded(u64),\n}",
    "operation `page.wait_for_ready`: its parameter `state` takes a deprecated variant that is no string"
)]
fn codegen_refuses_a_member_whose_form_its_kind_does_not_give(
    #[case] from: &str,
    #[case] to: &str,
    #[case] refusal: &str,
) {
    let workspace = workspace_with_the_forms_catalog();
    workspace.edit("crates/goodfetch-protocol/src/forms.rs", from, to);

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
    assert!(outcome.stderr.contains(refusal), "{}", outcome.stderr);
}

/// Each declared kind has one TypeScript form (ADR 0099), which the
/// generator writes on the handle its namespace and its handle ID name: the
/// generated handles type-check with the client package's own hand-written
/// modules under `tsc` with `skipLibCheck: false`, and
/// `codegen-forms/forms.types.ts` holds what a program can and cannot write
/// against them. A deprecated option or value has an overload of its own,
/// which oxlint's `no-deprecated` reports on each use the fixture marks and
/// on no other, since `tsc` reports no deprecation (ADR 0099).
///
/// Covers: story:11-178, story:11-182
#[test]
fn each_declared_kind_generates_its_typescript_form_which_type_checks() {
    let workspace = workspace_with_the_forms_catalog();
    codegen(&workspace);
    let repository = repository();
    for module in [
        "caller.ts",
        "field-values.ts",
        "framing.ts",
        "goodfetch-error.ts",
        "handle.ts",
        "json.ts",
        "markers.ts",
        "raw-call.ts",
        "wire.ts",
    ] {
        let contents =
            std::fs::read(repository.join(COLLIE_CLIENT).join("src").join(module)).unwrap();
        workspace.write(&format!("{COLLIE_CLIENT}/src/{module}"), &contents);
    }
    workspace.add_fixture_file(
        "codegen-forms/forms.types.ts",
        &format!("{COLLIE_CLIENT}/src/forms.types.ts"),
    );
    workspace.add_fixture_file(
        "codegen-forms/tsconfig.json",
        &format!("{COLLIE_CLIENT}/tsconfig.json"),
    );
    workspace.link_installed_runners();

    let checked = workspace
        .command(repository.join("node_modules/.bin/tsc"))
        .args(["-p", &format!("{COLLIE_CLIENT}/tsconfig.json")])
        .output()
        .unwrap();

    assert!(
        checked.status.success(),
        "{}{}",
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr)
    );
    workspace.add_fixture_file(
        "codegen-forms/oxlintrc.json",
        &format!("{COLLIE_CLIENT}/.oxlintrc.json"),
    );
    let linted = workspace
        .command(repository.join("node_modules/.bin/oxlint"))
        .current_dir(workspace.path().join(COLLIE_CLIENT))
        .args([
            "-c",
            ".oxlintrc.json",
            "--tsconfig",
            "tsconfig.json",
            "src/forms.types.ts",
        ])
        .output()
        .unwrap();
    assert!(
        linted.status.success(),
        "oxlint: {}{}",
        String::from_utf8_lossy(&linted.stdout),
        String::from_utf8_lossy(&linted.stderr)
    );
    insta::assert_snapshot!(
        "typescript_forms_handles",
        workspace.read_text(&format!("{COLLIE_CLIENT}/src/generated/handles.ts"))
    );
}

const PYTHON_CLIENT: &str = "sdks/python/collie-client";

/// The repository the tests run from, whose installed checkers and
/// hand-written client modules the type-checking tests use.
fn repository() -> &'static std::path::Path {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
}

/// Runs `checker`, one of the repository's installed Python type checkers,
/// in Collie's Python client package of `workspace`, and fails with what it
/// printed.
fn python_type_check(workspace: &Workspace, checker: &str, args: &[&str]) {
    let repository = repository();
    let ran = workspace
        .command(repository.join(".venv/bin").join(checker))
        .current_dir(workspace.path().join(PYTHON_CLIENT))
        .args(args)
        .output()
        .unwrap();
    assert!(
        ran.status.success(),
        "{checker}: {}{}",
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr)
    );
}

/// Each declared kind has one Python form (ADR 0099), on an async handle
/// and its sync twin: the generated handles type-check with the client
/// package's own hand-written modules under pyright's strict mode and
/// `mypy --strict`, and `codegen-forms/forms_types.py` holds what a program
/// can and cannot write against them, each refusal checked by both. A
/// deprecated keyword or value has an `@overload` of its own, which both
/// report on each use the fixture marks and on no other (ADR 0099).
///
/// Covers: story:11-178, story:11-182
#[test]
fn each_declared_kind_generates_its_python_form_which_type_checks() {
    let workspace = workspace_with_the_forms_catalog();
    codegen(&workspace);
    let repository = repository();
    let package = format!("{PYTHON_CLIENT}/src/goodfetch_collie_client");
    for module in [
        "caller.py",
        "handle.py",
        "markers.py",
        "point_in_time.py",
        "raw_call.py",
    ] {
        let contents = std::fs::read(repository.join(&package).join(module)).unwrap();
        workspace.write(&format!("{package}/{module}"), &contents);
    }
    workspace.write(&format!("{package}/__init__.py"), b"");
    workspace.add_fixture_file(
        "codegen-forms/forms_types.py",
        &format!("{package}/forms_types.py"),
    );
    for config in ["pyrightconfig.json", "mypy.ini"] {
        workspace.add_fixture_file(
            &format!("codegen-forms/{config}"),
            &format!("{PYTHON_CLIENT}/{config}"),
        );
    }
    workspace.link_installed_runners();
    let python = workspace.path().join(".venv/bin/python");
    let python = python.to_str().unwrap();

    python_type_check(&workspace, "pyright", &["--pythonpath", python]);
    python_type_check(
        &workspace,
        "mypy",
        &[
            "--python-executable",
            python,
            "src/goodfetch_collie_client/generated/handles.py",
            "src/goodfetch_collie_client/forms_types.py",
        ],
    );

    insta::assert_snapshot!(
        "python_forms_handles",
        workspace.read_text(&format!("{package}/generated/handles.py"))
    );
}

/// Every async handle class has its sync twin under the same name less
/// `Async`, with the same members (ADR 0099).
///
/// Covers: story:11-178
#[test]
fn every_async_python_handle_has_a_sync_twin_with_the_same_members() {
    let workspace = workspace_with_the_forms_catalog();
    codegen(&workspace);

    let handles = workspace.read_text(&format!(
        "{PYTHON_CLIENT}/src/goodfetch_collie_client/generated/handles.py"
    ));
    let mut classes: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut class = String::new();
    for line in handles.lines() {
        if let Some(rest) = line.strip_prefix("class ") {
            class = rest.split([':', '(']).next().unwrap().to_owned();
            classes.entry(class.clone()).or_default();
        } else if let Some(rest) = line
            .strip_prefix("    async def ")
            .or_else(|| line.strip_prefix("    def "))
        {
            let name = rest.split('(').next().unwrap();
            let name = match name {
                "__aenter__" => "__enter__",
                "__aexit__" => "__exit__",
                other => other,
            };
            classes.get_mut(&class).unwrap().push(name.to_owned());
        }
    }
    let (asynchronous, sync): (Vec<_>, Vec<_>) = classes
        .iter()
        .partition(|(name, _)| name.starts_with("Async"));

    assert!(!asynchronous.is_empty(), "{handles}");
    assert_eq!(
        asynchronous
            .iter()
            .map(|(name, members)| (
                name.trim_start_matches("Async").to_owned(),
                (*members).clone()
            ))
            .collect::<Vec<_>>(),
        sync.iter()
            .map(|(name, members)| ((*name).clone(), (*members).clone()))
            .collect::<Vec<_>>()
    );
}

const RUST_CLIENT: &str = "crates/goodfetch-collie-client";

/// The `codegen` workspace with the forms catalog and a stand-in for
/// Collie's Rust client crate, with the real crate's call builders and
/// handle host, after `xtask codegen` wrote its handles.
fn workspace_with_the_rust_forms() -> Workspace {
    let workspace = workspace_with_the_forms_catalog();
    workspace.add_fixture_file(
        "codegen-forms/rust-client/Cargo.toml",
        &format!("{RUST_CLIENT}/Cargo.toml"),
    );
    workspace.add_fixture_file(
        "codegen-forms/rust-client/lib.rs",
        &format!("{RUST_CLIENT}/src/lib.rs"),
    );
    codegen(&workspace);
    workspace
}

/// Each declared kind has one Rust form (ADR 0099), on an async handle and
/// its `blocking` twin: a round trip is a `Call`, a query a `Query`, a
/// builder a method that makes no call, a sub-object a method, and a
/// `new_<value>` member a `Call` of the handle it opens; the generated
/// handles compile against the client crate's builders.
///
/// Covers: story:11-178
#[test]
fn each_declared_kind_generates_its_rust_form_which_compiles() {
    let workspace = workspace_with_the_rust_forms();

    let checked = workspace.cargo(&["check", "--offline", "-p", "goodfetch-collie-client"]);

    assert_eq!(checked.code, 0, "{}", checked.stderr);
    insta::assert_snapshot!(
        "rust_forms_handles",
        workspace.read_text(&format!("{RUST_CLIENT}/src/generated/handles.rs"))
    );
}

/// The `pub` items of `text`, a generated handles module: each type it
/// declares or adds methods to, with its methods, one `blocking::` prefix
/// left out of the type's path.
fn public_items(text: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut items: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut owner = String::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("pub struct ") {
            items
                .entry(name.split([' ', '{']).next().unwrap().to_owned())
                .or_default();
        } else if let Some(header) = line.strip_prefix("impl") {
            owner = header
                .trim_end_matches(" {")
                .trim_start_matches("<T>")
                .trim()
                .replace("crate::blocking::", "crate::");
            items.entry(owner.clone()).or_default();
        } else if let Some(method) = line.strip_prefix("    pub fn ") {
            let name = method.split(['(', '<']).next().unwrap().to_owned();
            items.get_mut(&owner).unwrap().push(name);
        }
    }
    items
}

/// The `blocking` twin repeats the async API under the same type names:
/// each handle, sub-object and builder option of the async module, with
/// the same methods (ADR 0099).
///
/// Covers: story:11-178
#[test]
fn the_blocking_rust_handles_repeat_the_async_api_under_the_same_names() {
    let workspace = workspace_with_the_rust_forms();

    let asynchronous =
        public_items(&workspace.read_text(&format!("{RUST_CLIENT}/src/generated/handles.rs")));
    let blocking =
        public_items(&workspace.read_text(&format!("{RUST_CLIENT}/src/generated/blocking.rs")));

    assert!(asynchronous.contains_key("Page"), "{asynchronous:?}");
    assert_eq!(asynchronous, blocking);
}

/// An Experimental or deprecated entry's docstring adds paragraphs after
/// its description, which keeps every line it has.
/// Covers: story:1-57
#[test]
fn python_docstring_keeps_every_line_of_a_marked_entry_description() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(
        SAMPLE,
        "description: \"Repeats a text back to the caller.\",",
        "description: \"Repeats a text\\nback to the caller.\",",
    );

    codegen(&workspace);

    let operations = workspace
        .read_text("sdks/python/collie-client/src/goodfetch_collie_client/generated/operations.py");
    assert!(
        operations
            .contains("\"\"\"Repeats a text\n        back to the caller.\n\n        Experimental"),
        "{operations}"
    );
}

/// A field whose type is an enum and whose default is one of its values
/// defaults to that enum's member, which a type checker accepts and which
/// dumps as the value, never to the value's bare string (fix-queue 115).
/// Covers: story:1-57
#[test]
fn python_enum_field_defaults_to_its_member() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(
        FOXHOUND,
        "    /// How many pages to visit.\n    pub pages: u32,\n}",
        "    /// How many pages to visit.\n    pub pages: u32,\n    \
         /// Which pages to visit.\n    #[serde(default)]\n    pub follow: SampleFollow,\n}\n\n\
         /// Which pages a sample crawl visits.\n\
         #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]\n\
         #[serde(rename_all = \"snake_case\")]\n\
         pub enum SampleFollow {\n    /// Every page.\n    #[default]\n    Scope,\n    \
         /// The seeds alone.\n    None,\n}",
    );

    codegen(&workspace);

    let models =
        workspace.read_text("sdks/python/foxhound/src/goodfetch_foxhound/generated/models.py");
    assert!(
        models.contains("    follow: SampleFollow | None = SampleFollow.scope\n"),
        "{models}"
    );
}

/// A field whose type is a model and whose default is an object, a
/// tagged union's variant or a struct, defaults through a factory that
/// constructs that model from the object, which a type checker accepts and
/// which each instance gets afresh, never through the object's bare dict
/// (`ValueField.transform`, #1002's); a field typed as a map keeps its
/// dict default.
/// Covers: story:1-57
#[test]
fn python_model_field_defaults_to_a_constructed_model() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(
        FOXHOUND,
        "    /// How many pages to visit.\n    pub pages: u32,\n}",
        "    /// How many pages to visit.\n    pub pages: u32,\n    \
         /// How each page is read.\n    #[serde(default)]\n    pub mode: SampleMode,\n    \
         /// The crawl's limits.\n    #[serde(default)]\n    pub limits: SampleLimits,\n    \
         /// Each page's tags.\n    #[serde(default = \"tags\")]\n    \
         pub tags: std::collections::BTreeMap<String, String>,\n}\n\n\
         fn tags() -> std::collections::BTreeMap<String, String> {\n    \
         std::collections::BTreeMap::from([(\"kind\".to_owned(), \"sample\".to_owned())])\n}\n\n\
         /// How a sample crawl reads each page.\n\
         #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]\n\
         #[serde(tag = \"kind\", rename_all = \"snake_case\")]\n\
         pub enum SampleMode {\n    /// Its text.\n    #[default]\n    Plain,\n    \
         /// Its links, so deep.\n    Deep {\n        /// How deep.\n        depth: u32,\n    },\n}\n\n\
         /// A sample crawl's limits.\n\
         #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]\n\
         pub struct SampleLimits {\n    /// The most pages.\n    pub pages: u32,\n}",
    );

    codegen(&workspace);

    let models =
        workspace.read_text("sdks/python/foxhound/src/goodfetch_foxhound/generated/models.py");
    let flat: String = models.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains(
            "mode: Annotated[SampleMode | None, Field(validate_default=True)] = Field( default_factory=lambda: SampleMode.model_validate({\"kind\": \"plain\"}) )"
        ) && flat.contains(
            "limits: Annotated[SampleLimits | None, Field(validate_default=True)] = Field( default_factory=lambda: SampleLimits.model_validate({\"pages\": 0}) )"
        ) && flat.contains("= {\"kind\": \"sample\"}"),
        "{models}"
    );
}

/// A property whose type is a named type keeps its own description in the
/// TypeScript layer, and the type keeps its own, even where another property
/// refers to that type with no description of its own (issue 2895: an array
/// of the type gave every such property the type's description).
/// Covers: story:1-57
#[test]
fn typescript_property_of_a_named_type_keeps_its_own_description() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(
        FOXHOUND,
        "    /// How many pages to visit.\n    pub pages: u32,\n}",
        "    /// How many pages to visit.\n    pub pages: u32,\n    \
         /// The finders tried when `start` finds no page.\n    \
         pub fallbacks: Vec<SampleFinder>,\n    \
         /// What finds the first page.\n    pub start: SampleFinder,\n}\n\n\
         /// What finds a page, named by `kind`.\n\
         #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]\n\
         #[serde(tag = \"kind\", rename_all = \"snake_case\")]\n\
         pub enum SampleFinder {\n    /// By its title.\n    Title {\n        \
         /// The title.\n        title: String,\n    },\n    \
         /// By its path.\n    Path {\n        /// The path.\n        path: String,\n    },\n}",
    );

    codegen(&workspace);

    let types = workspace.read_text("sdks/typescript/foxhound/src/generated/types.ts");
    let doc_before = |declaration: &str| {
        let at = types
            .find(declaration)
            .unwrap_or_else(|| panic!("no `{declaration}` in\n{types}"));
        let opening = types[..at].rfind("/**").unwrap();
        types[opening..at].to_owned()
    };
    let start = doc_before("  start:");
    assert!(
        start.contains("What finds the first page.") && !start.contains("What finds a page"),
        "{types}"
    );
    assert!(
        doc_before("export type SampleFinder =").contains("What finds a page, named by `kind`."),
        "{types}"
    );
}

const POLITENESS_MANIFEST: &str = "crates/foxhound/goodfetch-politeness/Cargo.toml";
const POLITENESS_LIB: &str = "crates/foxhound/goodfetch-politeness/src/lib.rs";

fn add_politeness_crate(workspace: &Workspace) {
    workspace.write(
        POLITENESS_MANIFEST,
        b"[package]\nname = \"goodfetch-politeness\"\nversion.workspace = true\nedition.workspace = true\npublish.workspace = true\n",
    );
    workspace.write(
        POLITENESS_LIB,
        b"//! A Foxhound crate added to the fixture.\n",
    );
}

/// Each crate cargo-deny bans with `wrappers`, and those wrappers.
fn wrapped_crates(deny: &str) -> Vec<(String, Vec<String>)> {
    let deny: toml::Table = deny.parse().unwrap();
    deny["bans"]["deny"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| {
            let wrappers = entry.get("wrappers")?.as_array()?;
            Some((
                entry["crate"].as_str().unwrap().to_owned(),
                wrappers
                    .iter()
                    .map(|wrapper| wrapper.as_str().unwrap().to_owned())
                    .collect(),
            ))
        })
        .collect()
}

/// Covers: story:1-57
#[test]
fn wrappers_list_follows_the_folders_under_crates_foxhound() {
    let workspace = Workspace::from_fixture("codegen");
    add_politeness_crate(&workspace);

    codegen(&workspace);

    let wrappers: Vec<String> = [
        "goodfetch-frontier",
        "goodfetch-politeness",
        "goodfetch-sink",
        "xtask",
    ]
    .map(str::to_owned)
    .to_vec();
    assert_eq!(
        wrapped_crates(&workspace.read_text("deny.toml")),
        vec![
            ("goodfetch-frontier".to_owned(), wrappers.clone()),
            ("goodfetch-politeness".to_owned(), wrappers.clone()),
            ("goodfetch-sink".to_owned(), wrappers),
        ]
    );
}

/// Covers: story:1-57
#[test]
fn check_fails_on_stale_lists_of_foxhounds_crates() {
    let workspace = Workspace::from_fixture("codegen");
    add_politeness_crate(&workspace);

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("drift", COLLIE_DENY), finding("drift", "deny.toml")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// Collie's cargo-deny configuration, rooted at its executable (ADR 0087).
const COLLIE_DENY: &str = "ci/deny/collie.toml";

/// Each crate a cargo-deny configuration bans, in order.
fn banned_crates(deny: &str) -> Vec<String> {
    let deny: toml::Table = deny.parse().unwrap();
    deny["bans"]["deny"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["crate"].as_str().unwrap().to_owned())
        .collect()
}

/// Covers: story:1-57
#[test]
fn collies_bans_follow_the_folders_under_crates_foxhound() {
    let workspace = Workspace::from_fixture("codegen");
    add_politeness_crate(&workspace);

    codegen(&workspace);

    assert_eq!(
        banned_crates(&workspace.read_text(COLLIE_DENY)),
        [
            "duckdb",
            "goodfetch-frontier",
            "goodfetch-politeness",
            "goodfetch-sink",
        ]
    );
}

/// The licenses cargo-deny allows.
fn allowed_licenses(deny: &str) -> Vec<String> {
    let deny: toml::Table = deny.parse().unwrap();
    deny["licenses"]["allow"]
        .as_array()
        .unwrap()
        .iter()
        .map(|license| license.as_str().unwrap().to_owned())
        .collect()
}

/// ADR 0018's license allowlist, the one list every license check reads.
const ALLOWLIST: &str = "license-allowlist.toml";

const ISC: &str = "[[license]]\nid = \"ISC\"\n";

/// Covers: story:1-57
#[test]
fn cargo_deny_allows_the_allowlists_licenses_but_those_for_filter_lists_only() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.append(ALLOWLIST, ISC);

    codegen(&workspace);

    assert_eq!(
        allowed_licenses(&workspace.read_text("deny.toml")),
        ["MIT", "Apache-2.0", "MPL-2.0", "ISC"]
    );
}

/// A license allowed for one npm or Python package alone, as PSF-2.0 is for
/// typing-extensions, is no license cargo-deny may allow for every crate.
/// Covers: story:1-57
#[test]
fn cargo_deny_never_allows_a_license_allowed_for_one_package() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.append(
        ALLOWLIST,
        "[[license]]\nid = \"PSF-2.0\"\nonly = { ecosystem = \"python\", package = \"typing-extensions\" }\n",
    );

    codegen(&workspace);

    assert_eq!(
        allowed_licenses(&workspace.read_text("deny.toml")),
        ["MIT", "Apache-2.0", "MPL-2.0"]
    );
}

/// Covers: story:1-57
#[test]
fn check_fails_on_a_stale_license_list() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.append(ALLOWLIST, ISC);

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("drift", "deny.toml")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// IANA's IPv4 special-purpose registry and the classification table
/// `xtask codegen` writes from it into `goodfetch-engine` (ADR 0067).
const IPV4_REGISTRY: &str = "crates/goodfetch-engine/iana/iana-ipv4-special-registry-1.csv";
const IPV4_REGISTRY_SOURCE: &str =
    "crates/goodfetch-engine/iana/iana-ipv4-special-registry-1.source.toml";
const SPECIAL_PURPOSE_TABLE: &str = "crates/goodfetch-engine/generated/special_purpose.rs";

/// Covers: story:4-8
#[rstest]
#[case::a_block_no_longer_globally_reachable(
    IPV4_REGISTRY,
    "192.31.196.0/24,AS112-v4,[RFC 7535],2014-12,N/A,True,True,True,True,False",
    "192.31.196.0/24,AS112-v4,[RFC 7535],2014-12,N/A,True,True,True,False,False"
)]
#[case::a_new_registry_date(
    IPV4_REGISTRY_SOURCE,
    "registry_updated = \"2025-10-09\"",
    "registry_updated = \"2026-10-01\""
)]
fn check_fails_when_the_special_purpose_registries_change_and_the_table_was_not_regenerated(
    #[case] path: &str,
    #[case] from: &str,
    #[case] to: &str,
) {
    let workspace = Workspace::from_fixture("codegen");
    workspace.edit(path, from, to);

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("drift", SPECIAL_PURPOSE_TABLE)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}
