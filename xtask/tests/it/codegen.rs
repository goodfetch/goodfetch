//! `xtask codegen` and its drift check (ADRs 0038, 0087 and 0095): the
//! committed bundle, catalog and per-Product outputs, each SDK package's
//! generated layer, and the lists only `xtask codegen` writes: the
//! `wrappers` list and Collie's ban of Foxhound's crates (ADR 0087), and
//! cargo-deny's copy of ADR 0018's license allowlist.
//! Covers: story:1-57

use rstest::rstest;
use serde_json::Value;

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
    ];
    for folder in GENERATED_FOLDERS {
        let mut paths: Vec<_> = std::fs::read_dir(workspace.path().join(folder))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        paths.sort();
        for name in paths {
            let path = format!("{folder}/{name}");
            let contents = workspace.read_text(&path);
            files.push((path, contents));
        }
    }
    files
}

const GENERATED_FOLDERS: [&str; 5] = [
    "crates/goodfetch-protocol/generated",
    "sdks/typescript/collie-client/src/generated",
    "sdks/typescript/foxhound/src/generated",
    "sdks/python/collie-client/src/goodfetch_collie_client/generated",
    "sdks/python/foxhound/src/goodfetch_foxhound/generated",
];

#[test]
fn regenerating_twice_gives_identical_output() {
    let workspace = Workspace::from_fixture("codegen");
    codegen(&workspace);
    let first = generated_files(&workspace);

    codegen(&workspace);

    assert_eq!(generated_files(&workspace), first);
}

#[test]
fn check_passes_on_a_clean_tree() {
    let workspace = Workspace::from_fixture("codegen");

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

const SAMPLE: &str = "crates/goodfetch-protocol/src/sample.rs";
const FOXHOUND: &str = "crates/goodfetch-protocol/src/foxhound.rs";

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
        finding("drift", "sdks/python/foxhound/src/goodfetch_foxhound/generated/operations.py"),
        finding("drift", "sdks/typescript/foxhound/src/generated/operations.ts"),
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

/// An outside generator's output depends on its version, which the lockfile
/// pins, so a bump that was not regenerated is stale, though no type changed.
#[rstest]
#[case::json_schema_to_typescript(
    "pnpm-lock.yaml",
    "version: 16.0.0",
    "version: 16.1.0",
    vec![
        finding("stale", "sdks/typescript/collie-client/src/generated/types.ts"),
        finding("stale", "sdks/typescript/foxhound/src/generated/types.ts"),
    ],
)]
#[case::datamodel_code_generator(
    "uv.lock",
    "version = \"0.86.0\"",
    "version = \"0.87.0\"",
    vec![
        finding("stale", "sdks/python/collie-client/src/goodfetch_collie_client/generated/models.py"),
        finding("stale", "sdks/python/foxhound/src/goodfetch_foxhound/generated/models.py"),
    ],
)]
#[case::ruff_which_formats_the_python_models(
    "uv.lock",
    "version = \"0.17.1\"",
    "version = \"0.18.0\"",
    vec![
        finding("stale", "sdks/python/collie-client/src/goodfetch_collie_client/generated/models.py"),
        finding("stale", "sdks/python/foxhound/src/goodfetch_foxhound/generated/models.py"),
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

#[rstest]
#[case::generator_output("sdks/typescript/collie-client/src/generated/types.ts", "edited")]
#[case::method_layer(
    "sdks/python/collie-client/src/goodfetch_collie_client/generated/operations.py",
    "drift"
)]
#[case::catalog("crates/goodfetch-protocol/generated/catalog.json", "drift")]
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
