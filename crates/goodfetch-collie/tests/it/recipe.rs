//! `recipe digest` through Collie's executable (issue 1664): the digest of
//! a Recipe's package on disk, the one the conformance suite's vectors
//! give, printed alone or as JSON, and a package that is not valid refused
//! naming its path.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the package's files the command reads"
)]

use std::path::Path;

use goodfetch_test_support::{TestEnv, cli};
use serde_json::Value;

use crate::daemon::validate;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The conformance suite's digest vectors.
const VECTORS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../conformance/recipe-digest/vectors.json"
));

/// The vector named `name`.
fn vector(name: &str) -> Value {
    let vectors: Value = serde_json::from_str(VECTORS).unwrap();
    vectors["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|vector| vector["name"] == name)
        .unwrap()
        .clone()
}

/// Writes `vector`'s files under `root`.
fn write(root: &Path, vector: &Value) {
    for file in vector["files"].as_array().unwrap() {
        let path = root.join(file["path"].as_str().unwrap());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file["text"].as_str().unwrap()).unwrap();
    }
}

/// A package on disk prints its vector's digest: alone, and under `--json`
/// with the files it covers.
///
/// Covers: command:recipe.digest
#[test]
fn recipe_digest_prints_the_digest_of_the_package_on_disk() {
    let env = TestEnv::new();
    let root = env.folder().join("price-watch");
    let vector = vector("a step list");
    write(&root, &vector);
    let root = root.display().to_string();

    let alone = cli(&env, COLLIE)
        .args(["recipe", "digest", &root])
        .run()
        .unwrap();
    let json = cli(&env, COLLIE)
        .args(["recipe", "digest", &root, "--json"])
        .run()
        .unwrap();

    let printed: Value = serde_json::from_str(&json.stdout).unwrap();
    validate(&printed, "RecipeDigestOutput");
    let digest = vector["digest"].as_str().unwrap();
    assert_eq!(
        (alone.exit, alone.stdout.as_str(), json.exit, printed),
        (
            Some(0),
            format!("{digest}\n").as_str(),
            Some(0),
            serde_json::json!({
                "digest": digest,
                "files": ["goodfetch-recipe.toml", "steps.json"],
            }),
        )
    );
}

/// A folder with no manifest is refused with `recipe_invalid` naming it.
///
/// Covers: command:recipe.digest, error:recipe_invalid
#[test]
fn recipe_digest_refuses_a_package_that_is_not_valid_naming_its_path() {
    let env = TestEnv::new();
    let root = env.folder().join("price-watch");
    std::fs::create_dir_all(&root).unwrap();

    let output = cli(&env, COLLIE)
        .args(["recipe", "digest", &root.display().to_string(), "--json"])
        .run()
        .unwrap();

    let printed: Value = serde_json::from_str(&output.stdout).unwrap();
    validate(&printed, "CommandFailed");
    assert_ne!(output.exit, Some(0), "{output}");
    assert_eq!(
        (&printed["error"]["code"], &printed["error"]["field"]),
        (
            &Value::from("recipe_invalid"),
            &Value::from("goodfetch-recipe.toml")
        )
    );
}
