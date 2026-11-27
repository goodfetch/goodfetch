use std::fs;
use std::path::Path;

use goodfetch_protocol::plugin::PluginKindName;
use rstest::rstest;
use serde_json::{Value, json};

use super::PluginStore;

/// A manifest of the Extractor `name`.
fn manifest(name: &str) -> Value {
    json!({
        "manifest_version": "0.1",
        "name": name,
        "version": "1.0.0",
        "kinds": ["extract"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
        "network_level": "none",
    })
}

/// Writes `manifest` as the bundle `<name>/<version>` of the store at
/// `store`.
fn installed(store: &Path, name: &str, version: &str, manifest: &Value) {
    let bundle = store.join(name).join(version);
    fs::create_dir_all(&bundle).unwrap();
    fs::write(bundle.join("goodfetch-plugin.json"), manifest.to_string()).unwrap();
}

/// An installed Plugin resolves to its bundle and its manifest.
#[test]
fn an_installed_plugin_resolves_to_its_bundle() {
    let store = tempfile::tempdir().unwrap();
    installed(store.path(), "kettle", "1.0.0", &manifest("kettle"));

    let resolved = PluginStore::new(store.path().to_owned())
        .resolve("kettle", &PluginKindName::EXTRACT)
        .unwrap();

    assert_eq!(
        (resolved.bundle, resolved.manifest.name),
        (
            store.path().join("kettle").join("1.0.0"),
            "kettle".to_owned()
        )
    );
}

/// A name resolves to nothing the store cannot vouch for: each refusal
/// names the Plugin and no path.
#[rstest]
#[case::not_installed("absent", "no Plugin `absent` is installed")]
#[case::parent_folder("..", "`..` is no Plugin's name")]
#[case::nested_path("kettle/1.0.0", "is no Plugin's name")]
#[case::windows_separator("kettle\\1.0.0", "is no Plugin's name")]
#[case::two_versions("two", "the Plugin `two` has 2 versions installed")]
#[case::no_version("empty", "the Plugin `empty` has 0 versions installed")]
#[case::misnamed("misnamed", "names the Plugin `kettle`")]
#[case::another_kind("decision", "the Plugin `decision` is no extract Plugin")]
#[case::newer_major("newer", "is of version 1.0")]
#[case::malformed("malformed", "the Plugin `malformed`'s manifest")]
#[case::no_manifest("bare", "the Plugin `bare`'s files cannot be read")]
fn a_name_the_store_cannot_vouch_for_resolves_to_nothing(
    #[case] name: &str,
    #[case] refused: &str,
) {
    let store = tempfile::tempdir().unwrap();
    let root = store.path();
    installed(root, "two", "1.0.0", &manifest("two"));
    installed(root, "two", "1.1.0", &manifest("two"));
    fs::create_dir_all(root.join("empty")).unwrap();
    installed(root, "misnamed", "1.0.0", &manifest("kettle"));
    let mut decision = manifest("decision");
    decision["kinds"] = json!(["decision"]);
    installed(root, "decision", "1.0.0", &decision);
    let mut newer = manifest("newer");
    newer["manifest_version"] = json!("1.0");
    installed(root, "newer", "1.0.0", &newer);
    installed(
        root,
        "malformed",
        "1.0.0",
        &json!({ "manifest_version": "0.1" }),
    );
    fs::create_dir_all(root.join("bare").join("1.0.0")).unwrap();

    let resolved = PluginStore::new(root.to_owned()).resolve(name, &PluginKindName::EXTRACT);

    let Err(error) = resolved else {
        panic!("`{name}` resolved");
    };
    let text = error.to_string();
    assert!(
        text.contains(refused) && !text.contains(&*root.to_string_lossy()),
        "{text}"
    );
}

/// The manifest of the Extractor `name` with `field` set to `value`.
fn manifest_with(name: &str, field: &str, value: Value) -> Value {
    let mut manifest = manifest(name);
    manifest[field] = value;
    manifest
}

/// Writes `manifest` at the root of a new staging folder of `store`.
fn staged(store: &PluginStore, manifest: &Value) -> tempfile::TempDir {
    let staging = store.staging().unwrap();
    fs::write(
        staging.path().join("goodfetch-plugin.json"),
        manifest.to_string(),
    )
    .unwrap();
    staging
}

/// An added Plugin's staged bundle moves whole to `<name>/<version>` and
/// resolves; its staging folder sat beside the store, never in it.
///
/// Covers: story:7-5
#[test]
fn an_added_plugin_moves_into_the_store_whole() {
    let home = tempfile::tempdir().unwrap();
    let store = PluginStore::new(home.path().join("plugins"));
    let staging = staged(&store, &manifest("kettle"));
    fs::write(staging.path().join("data.txt"), "kept").unwrap();
    assert_eq!(staging.path().parent(), Some(home.path()));

    let added = store.add("kettle", staging.path(), DIGEST).unwrap();

    let bundle = home.path().join("plugins").join("kettle").join("1.0.0");
    assert_eq!(added.bundle, bundle);
    assert_eq!(fs::read_to_string(bundle.join("data.txt")).unwrap(), "kept");
    assert!(!staging.path().exists());
    let resolved = store.resolve("kettle", &PluginKindName::EXTRACT).unwrap();
    assert_eq!(resolved.bundle, bundle);
}

/// A visibly fake image index digest.
const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// An added Plugin resolves with the image index digest it was added at,
/// which an Operator policy's pin is held to; a Plugin the store holds
/// that `plugin add` did not add has none.
///
/// Covers: story:7-17
#[test]
fn an_added_plugin_resolves_with_its_digest() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("plugins");
    installed(&root, "placed", "1.0.0", &manifest("placed"));
    let store = PluginStore::new(root);
    let staging = staged(&store, &manifest("kettle"));
    store.add("kettle", staging.path(), DIGEST).unwrap();

    let digest = |name| {
        store
            .resolve(name, &PluginKindName::EXTRACT)
            .unwrap()
            .digest
    };
    assert_eq!(
        (digest("kettle"), digest("placed")),
        (Some(DIGEST.to_owned()), None)
    );
}

/// A staged bundle the store cannot vouch for is not added, and the store
/// is left as it was.
#[rstest]
#[case::misnamed("kettle", manifest("teapot"), "names the Plugin `teapot`")]
#[case::path_name("../kettle", manifest("../kettle"), "is no Plugin's name")]
#[case::path_version(
    "kettle",
    manifest_with("kettle", "version", json!("../1.0.0")),
    "version `../1.0.0` holds a path"
)]
#[case::installed(
    "present",
    manifest("present"),
    "`present` is installed already, at version 1.0.0"
)]
#[case::newer_major(
    "kettle",
    manifest_with("kettle", "manifest_version", json!("1.0")),
    "is of version 1.0"
)]
fn a_staged_bundle_the_store_cannot_vouch_for_is_not_added(
    #[case] name: &str,
    #[case] staged_manifest: Value,
    #[case] refused: &str,
) {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("plugins");
    installed(&root, "present", "1.0.0", &manifest("present"));
    let store = PluginStore::new(root.clone());
    let staging = staged(&store, &staged_manifest);

    let error = store.add(name, staging.path(), DIGEST).unwrap_err();

    assert!(error.to_string().contains(refused), "{error}");
    assert!(staging.path().join("goodfetch-plugin.json").exists());
    let names: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["present"]);
}

/// A Secret provider manifest claiming `scheme`.
fn secret_manifest(name: &str, scheme: &str) -> Value {
    json!({
        "manifest_version": "0.1",
        "name": name,
        "version": "1.0.0",
        "kinds": ["secret"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
        "network_level": "none",
        "scheme": scheme,
    })
}

/// A Secret provider installed with a declared scheme resolves by that scheme;
/// a second Secret provider claiming the same scheme is refused at install and
/// on resolution (ADRs 0015 and 0033).
///
/// Covers: story:7-90, story:7-51
#[test]
fn a_secret_provider_resolves_by_scheme_and_a_second_claiming_it_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("plugins");
    let store = PluginStore::new(root.clone());

    let first = staged(&store, &secret_manifest("vault-a", "corp-vault"));
    store.add("vault-a", first.path(), DIGEST).unwrap();

    let resolved = store.resolve_secret_scheme("corp-vault").unwrap();
    assert_eq!(resolved.manifest.name, "vault-a");

    let missing = store.resolve_secret_scheme("other-vault").unwrap_err();
    assert!(
        matches!(missing, super::StoreError::NoScheme(ref scheme) if scheme == "other-vault"),
        "{missing:?}"
    );

    let duplicate = staged(&store, &secret_manifest("vault-b", "corp-vault"));
    let conflict = store.add("vault-b", duplicate.path(), DIGEST).unwrap_err();
    assert!(
        matches!(
            conflict,
            super::StoreError::SchemeConflict { ref scheme, ref existing }
                if scheme == "corp-vault" && existing == "vault-a"
        ),
        "{conflict:?}"
    );
    assert!(!root.join("vault-b").exists());

    // Even if a second provider were placed on disk directly, resolution refuses the conflict.
    installed(
        &root,
        "vault-b",
        "1.0.0",
        &secret_manifest("vault-b", "corp-vault"),
    );
    let resolve_conflict = store.resolve_secret_scheme("corp-vault").unwrap_err();
    assert!(
        matches!(
            resolve_conflict,
            super::StoreError::SchemeConflict { ref scheme, .. } if scheme == "corp-vault"
        ),
        "{resolve_conflict:?}"
    );
}
