#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use rstest::rstest;
use serde_json::{Value, json};

use super::{
    RECIPE_MANIFEST_VERSION, RecipeEntry, RecipeManifest, Runtime, RuntimeLanguage, read,
    version_of,
};
use crate::catalog::{Product, ProtocolVersion};

fn document(toml_text: &str) -> Value {
    toml::from_str(toml_text).expect("the test's manifest is TOML")
}

/// The field `read` refuses `toml_text` for.
fn refused_field(toml_text: &str) -> String {
    read(document(toml_text))
        .expect_err("the manifest is refused")
        .field
}

/// Covers: format:recipe-manifest
#[test]
fn a_program_manifest_reads_with_its_runtime_and_settings() {
    let manifest = read(document(
        r#"
        version = "0.1"
        name = "price-watch"
        product = "collie"
        entry.program = "src/main.ts"
        runtime = { language = "node", version = "24" }
        settings.timeout.navigation = 30000
        "#,
    ))
    .unwrap();

    assert_eq!(
        manifest,
        RecipeManifest {
            version: RECIPE_MANIFEST_VERSION,
            name: "price-watch".to_owned(),
            product: Product::Collie,
            entry: RecipeEntry::Program("src/main.ts".to_owned()),
            runtime: Some(Runtime {
                language: RuntimeLanguage::Node,
                version: "24".to_owned(),
            }),
            settings: json!({ "timeout": { "navigation": 30000 } })
                .as_object()
                .unwrap()
                .clone(),
            expose: BTreeMap::default(),
        }
    );
}

/// A Recipe made of data holds no source and names no runtime.
///
/// Covers: format:recipe-manifest
#[rstest]
#[case::steps("collie", "steps", RecipeEntry::Steps("login.steps.json".to_owned()))]
#[case::crawl("foxhound", "crawl", RecipeEntry::Crawl("shop.crawl.json".to_owned()))]
fn a_data_entry_reads_with_no_runtime(
    #[case] product: &str,
    #[case] key: &str,
    #[case] expected: RecipeEntry,
) {
    let manifest = read(document(&format!(
        "version = \"0.1\"\nname = \"data\"\nproduct = \"{product}\"\nentry.{key} = \"{}\"\n",
        expected.path()
    )))
    .unwrap();

    assert_eq!((manifest.entry, manifest.runtime), (expected, None));
}

/// Covers: error:recipe_invalid
#[rstest]
#[case::two_entries(
    "entry = { program = \"main.py\", steps = \"s.json\" }\nruntime = { language = \"python\", version = \"3.12\" }",
    "entry"
)]
#[case::no_entry("entry = {}", "entry")]
#[case::entry_absent("", "entry")]
#[case::unknown_entry_kind("entry.script = \"main.sh\"", "entry.script")]
#[case::unknown_field("entry.steps = \"s.json\"\nschedule = \"hourly\"", "schedule")]
#[case::unknown_runtime_field(
    "entry.program = \"main.py\"\nruntime = { language = \"python\", version = \"3.12\", pip = true }",
    "runtime"
)]
#[case::program_without_runtime("entry.program = \"main.py\"", "runtime")]
#[case::unlisted_runtime_version(
    "entry.program = \"main.py\"\nruntime = { language = \"python\", version = \"3.9\" }",
    "runtime.version"
)]
#[case::data_with_runtime(
    "entry.steps = \"s.json\"\nruntime = { language = \"node\", version = \"24\" }",
    "runtime"
)]
#[case::entry_leaving_the_package(
    "entry.program = \"../main.py\"\nruntime = { language = \"python\", version = \"3.12\" }",
    "entry.program"
)]
#[case::absolute_entry("entry.steps = \"/etc/s.json\"", "entry.steps")]
fn a_faulty_manifest_is_refused_naming_its_field(#[case] rest: &str, #[case] field: &str) {
    let manifest = format!("version = \"0.1\"\nname = \"r\"\nproduct = \"collie\"\n{rest}\n");

    assert_eq!(refused_field(&manifest), field, "{manifest}");
}

/// Covers: error:recipe_invalid
#[rstest]
#[case::name(
    "version = \"0.1\"\nproduct = \"collie\"\nentry.steps = \"s.json\"",
    "name"
)]
#[case::product("version = \"0.1\"\nname = \"r\"\nentry.steps = \"s.json\"", "product")]
#[case::version(
    "name = \"r\"\nproduct = \"collie\"\nentry.steps = \"s.json\"",
    "version"
)]
#[case::bad_name(
    "version = \"0.1\"\nname = \"Price Watch\"\nproduct = \"collie\"\nentry.steps = \"s.json\"",
    "name"
)]
fn a_missing_or_malformed_required_field_is_refused_naming_it(
    #[case] manifest: &str,
    #[case] field: &str,
) {
    assert_eq!(refused_field(manifest), field);
}

/// Covers: error:recipe_invalid
#[test]
fn a_crawl_declaration_for_collie_is_refused_with_the_fix_naming_foxhound() {
    let details = read(document(
        "version = \"0.1\"\nname = \"r\"\nproduct = \"collie\"\nentry.crawl = \"c.json\"\n",
    ))
    .unwrap_err();

    assert_eq!(details.field, "entry.crawl");
    assert!(
        details.fix.contains("product = \"foxhound\""),
        "{details:?}"
    );
}

/// The version is read on its own, so that a reader refuses a newer
/// manifest for its version before it reads a field it does not know.
#[test]
fn version_of_reads_a_newer_manifest_whose_fields_it_does_not_know() {
    assert_eq!(
        version_of(&document("version = \"0.9\"\nsample = { pages = 3 }\n")),
        Ok(ProtocolVersion { major: 0, minor: 9 })
    );
    assert_eq!(
        version_of(&json!({ "version": 1 })).unwrap_err().field,
        "version"
    );
}

/// The bundle's schema of the manifest accepts what `read` accepts and
/// refuses an unknown field, so an editor flags it as the engine does.
///
/// Covers: format:recipe-manifest
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test's own validator, on no Shared path (ADR 0140)"
)]
fn the_schema_accepts_a_manifest_and_refuses_an_unknown_field() {
    let schema = serde_json::to_value(schemars::schema_for!(RecipeManifest)).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let manifest = document(
        "version = \"0.1\"\nname = \"r\"\nproduct = \"foxhound\"\nentry.crawl = \"c.json\"\n",
    );
    read(manifest.clone()).unwrap();

    assert!(validator.is_valid(&manifest));
    let mut unknown = manifest;
    unknown["schedule"] = json!("hourly");
    assert!(!validator.is_valid(&unknown));
}

/// A program's runtime is one the Product version is tested and built on:
/// the SDKs' Node lines and CPython minors, and Rust from the workspace's
/// MSRV to its toolchain.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "a synchronous unit test reads the repository's own files; no async worker runs"
)]
fn the_runtime_versions_are_the_ones_the_repository_tests_and_builds_on() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read_toml = |path: &str| -> toml::Table {
        std::fs::read_to_string(root.join(path))
            .unwrap()
            .parse()
            .unwrap()
    };
    let runtimes = read_toml("ci/runtimes.toml");
    let tested = |prefix: &str| -> Vec<String> {
        runtimes["runtimes"]
            .as_table()
            .unwrap()
            .keys()
            .filter_map(|id| id.strip_prefix(prefix))
            .filter(|version| !version.ends_with('t'))
            .map(str::to_owned)
            .collect()
    };
    let msrv = read_toml("Cargo.toml")["workspace"]["package"]["rust-version"]
        .as_str()
        .unwrap()
        .to_owned();
    let channel = read_toml("rust-toolchain.toml")["toolchain"]["channel"]
        .as_str()
        .unwrap()
        .to_owned();
    let minor = |version: &str| -> u32 { version.split('.').nth(1).unwrap().parse().unwrap() };

    assert_eq!(RuntimeLanguage::Node.versions(), tested("node-"));
    assert_eq!(RuntimeLanguage::Python.versions(), tested("cpython-"));
    assert_eq!(
        RuntimeLanguage::Rust.versions(),
        (minor(&msrv)..=minor(&channel))
            .map(|minor| format!("1.{minor}"))
            .collect::<Vec<_>>()
    );
}
