//! The Feature inventory's generators of `command:`, `screen:` and
//! `block-cause:` (ADR 0072): each command of both Products' executables,
//! each route of the Console's packages and each Block cause of the catalog
//! is a Feature the moment it exists, with the Products that carry it, and
//! leaves the inventory when it goes. A kind whose source does not exist yet
//! generates nothing. A story is carried by both Products unless its entry
//! names the one Product that carries it.

use serde_json::{Value, json};

use crate::workspace::{Workspace, finding};

const JSON2TS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../node_modules/.bin/json2ts");
const DATAMODEL_CODEGEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../.venv/bin/datamodel-codegen"
);

fn codegen(workspace: &Workspace) {
    let outcome = workspace.xtask(&[
        "codegen",
        "--json2ts",
        JSON2TS,
        "--datamodel-codegen",
        DATAMODEL_CODEGEN,
    ]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// The inventory's Features of `kind`, each as its ID and its Products.
fn features_of_kind(workspace: &Workspace, kind: &str) -> Vec<(String, Value)> {
    let inventory: Value =
        serde_json::from_str(&workspace.read_text("features/inventory.json")).unwrap();
    inventory["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|feature| feature["kind"] == kind)
        .map(|feature| {
            (
                feature["id"].as_str().unwrap().to_owned(),
                feature["products"].clone(),
            )
        })
        .collect()
}

fn both() -> Value {
    json!(["collie", "foxhound"])
}

fn foxhound() -> Value {
    json!(["foxhound"])
}

const COMMAND_LINES_EXAMPLE: &str = "crates/foxhound/goodfetch-foxhound/examples/command-lines.rs";

/// The namespaces file the stand-in gives.
const NAMESPACES: &str = "// The fixture's namespaces.\n[]\n";

/// Adds Foxhound's executable crate to the fixture, whose `command-lines`
/// example writes `command_lines` as both executables' command trees.
fn add_executable_crate(workspace: &Workspace, command_lines: &Value) {
    workspace.write(
        "crates/foxhound/goodfetch-foxhound/Cargo.toml",
        b"[package]\nname = \"goodfetch-foxhound\"\nversion.workspace = true\nedition.workspace = true\npublish.workspace = true\n\n[features]\nexecutable = []\n",
    );
    workspace.write(
        "crates/foxhound/goodfetch-foxhound/src/lib.rs",
        b"//! The fixture's stand-in of Foxhound's executable crate.\n",
    );
    write_command_lines(workspace, command_lines);
}

fn write_command_lines(workspace: &Workspace, command_lines: &Value) {
    let printed =
        |product: &str| json!({ "command_line": command_lines[product], "namespaces": NAMESPACES });
    let written = json!({ "collie": printed("collie"), "foxhound": printed("foxhound") });
    let source = format!(
        "fn main() {{\n    let folder = std::path::PathBuf::from(std::env::args_os().nth(1).unwrap());\n    std::fs::write(folder.join(\"command-lines.json\"), {:?}).unwrap();\n}}\n",
        written.to_string()
    );
    workspace.write(COMMAND_LINES_EXAMPLE, source.as_bytes());
}

fn tree(name: &str, commands: &[Value]) -> Value {
    json!({ "name": name, "commands": commands })
}

/// Collie's command line, which Foxhound's keeps, and Foxhound's, which adds
/// `crawl`.
fn command_lines(foxhound_adds_crawl: bool) -> Value {
    let shared = [
        tree("attached-engine", &[]),
        tree("browsers", &[tree("install", &[])]),
    ];
    let mut foxhound = shared.to_vec();
    if foxhound_adds_crawl {
        foxhound.push(tree("crawl", &[]));
    }
    json!({
        "collie": tree("goodfetch-collie", &shared),
        "foxhound": tree("goodfetch-foxhound", &foxhound),
    })
}

#[test]
fn each_command_of_both_executables_is_a_feature_with_its_products() {
    let workspace = Workspace::from_fixture("codegen");
    add_executable_crate(&workspace, &command_lines(true));

    codegen(&workspace);

    assert_eq!(
        features_of_kind(&workspace, "command"),
        vec![
            ("command:attached-engine".to_owned(), both()),
            ("command:browsers.install".to_owned(), both()),
            ("command:crawl".to_owned(), foxhound()),
        ]
    );
}

#[test]
fn the_table_of_foxhounds_names_lists_the_commands_only_foxhound_carries() {
    let workspace = Workspace::from_fixture("codegen");
    add_executable_crate(&workspace, &command_lines(true));

    codegen(&workspace);

    let table = workspace.read_text("crates/goodfetch-protocol/generated/foxhound_names.rs");
    assert!(
        table.contains("pub const COMMANDS: &[&str] = &[\"crawl\"];\n"),
        "{table}"
    );
}

#[test]
fn a_removed_command_leaves_the_inventory() {
    let workspace = Workspace::from_fixture("codegen");
    add_executable_crate(&workspace, &command_lines(true));
    codegen(&workspace);
    write_command_lines(&workspace, &command_lines(false));

    codegen(&workspace);

    assert_eq!(
        features_of_kind(&workspace, "command"),
        vec![
            ("command:attached-engine".to_owned(), both()),
            ("command:browsers.install".to_owned(), both()),
        ]
    );
}

#[test]
fn the_check_reads_the_committed_command_lines_and_runs_no_example() {
    let workspace = Workspace::from_fixture("codegen");
    add_executable_crate(&workspace, &command_lines(true));
    codegen(&workspace);
    workspace.write(
        "crates/goodfetch-protocol/generated/command_lines.json",
        command_lines(false).to_string().as_bytes(),
    );
    workspace.write(
        COMMAND_LINES_EXAMPLE,
        b"fn main() {\n    std::process::exit(3);\n}\n",
    );

    let outcome = workspace.xtask(&["codegen", "--check"]);

    assert_eq!(
        outcome.findings(),
        vec![
            finding(
                "drift",
                "crates/goodfetch-protocol/generated/foxhound_names.rs"
            ),
            finding("drift", "features/inventory.json"),
        ]
    );
    assert_eq!(outcome.code, 1);
}

const COLLIE_ROUTES: &str = "console/collie/routes.json";
const FOXHOUND_ROUTES: &str = "console/foxhound/routes.json";

#[test]
fn each_console_route_is_a_feature_with_its_products() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.write(COLLIE_ROUTES, br#"{ "routes": ["/runs", "/runs/$run"] }"#);
    workspace.write(FOXHOUND_ROUTES, br#"{ "routes": ["/datasets"] }"#);

    codegen(&workspace);

    assert_eq!(
        features_of_kind(&workspace, "screen"),
        vec![
            ("screen:/datasets".to_owned(), foxhound()),
            ("screen:/runs".to_owned(), both()),
            ("screen:/runs/$run".to_owned(), both()),
        ]
    );
}

#[test]
fn a_removed_route_leaves_the_inventory() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.write(COLLIE_ROUTES, br#"{ "routes": ["/runs", "/runs/$run"] }"#);
    codegen(&workspace);
    workspace.write(COLLIE_ROUTES, br#"{ "routes": ["/runs"] }"#);

    codegen(&workspace);

    assert_eq!(
        features_of_kind(&workspace, "screen"),
        vec![("screen:/runs".to_owned(), both())]
    );
}

/// The fixture's modules of Collie's and Foxhound's entries.
const COLLIE_ENTRIES: &str = "crates/goodfetch-protocol/src/sample.rs";
const FOXHOUND_ENTRIES: &str = "crates/goodfetch-protocol/src/foxhound.rs";
/// The list of values of shared types each of them declares, empty.
const NO_SHARED_TYPE_VALUES: &str = "    shared_type_values: &[],\n";

/// The declaration of the value `name` of the shared type `shared_type`,
/// one line of its module's `shared_type_values`.
fn shared_type_value(name: &str, shared_type: &str) -> String {
    let variant: String = shared_type
        .split('_')
        .map(|word| word[..1].to_uppercase() + &word[1..])
        .collect();
    format!(
        "        crate::catalog::SharedTypeValue {{ shared_type: crate::catalog::SharedType::{variant}, name: {name:?}, description: \"A value.\", arrived_in: ARRIVED_IN }},\n"
    )
}

/// Gives the fixture's catalog the values of shared types `values` lists,
/// each as its name, Product and type, declared in that Product's module.
pub(crate) fn add_shared_type_values(workspace: &Workspace, values: &[(&str, &str, &str)]) {
    for (product, module) in [("collie", COLLIE_ENTRIES), ("foxhound", FOXHOUND_ENTRIES)] {
        let declared: String = values
            .iter()
            .filter(|(_, of, _)| *of == product)
            .map(|(name, _, shared_type)| shared_type_value(name, shared_type))
            .collect();
        if !declared.is_empty() {
            workspace.edit(
                module,
                NO_SHARED_TYPE_VALUES,
                &format!("    shared_type_values: &[\n{declared}    ],\n"),
            );
        }
    }
}

#[test]
fn each_block_cause_is_a_feature_with_its_products() {
    let workspace = Workspace::from_fixture("codegen");
    add_shared_type_values(
        &workspace,
        &[
            ("challenge", "collie", "block_cause"),
            ("decoy", "foxhound", "block_cause"),
            ("sample_echoes", "collie", "usage_kind"),
        ],
    );

    codegen(&workspace);

    assert_eq!(
        features_of_kind(&workspace, "block-cause"),
        vec![
            ("block-cause:challenge".to_owned(), both()),
            ("block-cause:decoy".to_owned(), foxhound()),
        ]
    );
}

#[test]
fn a_removed_block_cause_leaves_the_inventory() {
    let workspace = Workspace::from_fixture("codegen");
    add_shared_type_values(
        &workspace,
        &[
            ("challenge", "collie", "block_cause"),
            ("rate_limit", "collie", "block_cause"),
        ],
    );
    codegen(&workspace);
    workspace.edit(
        COLLIE_ENTRIES,
        &shared_type_value("rate_limit", "block_cause"),
        "",
    );

    codegen(&workspace);

    assert_eq!(
        features_of_kind(&workspace, "block-cause"),
        vec![("block-cause:challenge".to_owned(), both())]
    );
}

/// The fixture has no executable crate, an empty Console and no Block cause,
/// as the repository had before each source landed.
#[test]
fn a_kind_whose_source_does_not_exist_generates_nothing_and_fails_nothing() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.write("console/README.md", b"# Console\n");

    codegen(&workspace);

    assert_eq!(
        [
            features_of_kind(&workspace, "command"),
            features_of_kind(&workspace, "screen"),
            features_of_kind(&workspace, "block-cause"),
        ],
        [vec![], vec![], vec![]]
    );
}

/// A story of Foxhound's part, such as a work item's lease, names its
/// Product, so Collie's view never lists it (ADR 0094).
#[test]
fn a_story_naming_its_product_is_carried_by_that_product_alone() {
    let workspace = Workspace::from_fixture("codegen");
    workspace.append(
        "features/stories/1.toml",
        "\n[[story]]\nid = 900\ntitle = \"As a developer, I want a crawl's work item leased.\"\nmilestone = \"M1 Core\"\njourney = false\nnot_journey = \"part\"\nstatus = \"active\"\nproduct = \"foxhound\"\n",
    );

    codegen(&workspace);

    let stories = features_of_kind(&workspace, "story");
    assert!(
        stories.contains(&("story:1-900".to_owned(), foxhound())),
        "{stories:?}"
    );
    assert!(stories.iter().any(|(_, products)| *products == both()));
}
