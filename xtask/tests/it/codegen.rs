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

const GENERATED_FOLDERS: [&str; 8] = [
    "crates/goodfetch-protocol/generated",
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
        vec![finding("drift", CANARY_RULE)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
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

/// Covers: story:1-57
#[rstest]
#[case::typescript_collie_client(
    "sdks/typescript/collie-client/src/generated/operations.ts",
    "\"sample.echo\"",
    "\"sample.crawl\""
)]
#[case::typescript_foxhound(
    "sdks/typescript/foxhound/src/generated/operations.ts",
    "\"sample.crawl\"",
    "\"sample.echo\""
)]
#[case::python_collie_client(
    "sdks/python/collie-client/src/goodfetch_collie_client/generated/operations.py",
    "\"sample.echo\"",
    "\"sample.crawl\""
)]
#[case::python_foxhound(
    "sdks/python/foxhound/src/goodfetch_foxhound/generated/operations.py",
    "\"sample.crawl\"",
    "\"sample.echo\""
)]
fn each_sdk_package_generates_only_its_products_operations(
    #[case] path: &str,
    #[case] own_operation: &str,
    #[case] other_operation: &str,
) {
    let workspace = Workspace::from_fixture("codegen");

    codegen(&workspace);

    let operations = workspace.read_text(path);
    assert!(operations.contains(own_operation), "{operations}");
    assert!(!operations.contains(other_operation), "{operations}");
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
/// (#1002's `ExtractionField.transform`); a field typed as a map keeps its
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
/// `xtask codegen` writes from it into `goodfetch-protocol` (ADR 0067).
const IPV4_REGISTRY: &str = "crates/goodfetch-protocol/iana/iana-ipv4-special-registry-1.csv";
const IPV4_REGISTRY_SOURCE: &str =
    "crates/goodfetch-protocol/iana/iana-ipv4-special-registry-1.source.toml";
const SPECIAL_PURPOSE_TABLE: &str = "crates/goodfetch-protocol/generated/special_purpose.rs";

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
