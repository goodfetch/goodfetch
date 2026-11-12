//! The settings the catalog marks as unbuilt (`Setting::unbuilt`) against
//! the workspace's sources (ADR 0083): every catalog setting is read by
//! product code, marked as unbuilt, or locked at its default for every
//! layer, so no value a scope sets is accepted and then ignored; and a
//! marked setting has no reader, so a key loses its mark in the change that
//! builds one.
//!
//! A read is a use of the setting's constant, `&NAME` or `NAME.`, in a
//! source file that is no test's: the catalog's own `settings: &[NAME]`
//! lists are none.
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads the workspace's sources itself"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use goodfetch_protocol::catalog::{Authority, Product};
use goodfetch_protocol::settings::SettingsLayer;
use serde_json::json;

use crate::settings::resolve::{Layers, resolve};
use crate::settings::values::Values;
use crate::settings::{SettingsError, keys};

/// The crates that are no product code: test tools and the lint tasks.
const NOT_PRODUCT: [&str; 5] = [
    "goodfetch-conformance",
    "goodfetch-stub-gateway",
    "goodfetch-test-client",
    "goodfetch-test-support",
    "goodfetch-workspace-hack",
];

/// The crate that declares the settings.
const PROTOCOL: &str = "goodfetch-protocol";

/// The Rust sources of the product code under `folder`, a crate's `src` or
/// a folder beneath one: no test module (`tests.rs`, `tests/`) and no
/// generated file.
fn sources(folder: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.map(|entry| entry.unwrap().path()) {
        let name = entry.file_name().unwrap().to_string_lossy().into_owned();
        if entry.is_dir() {
            if name != "tests" && name != "generated" {
                sources(&entry, into);
            }
        } else if entry.extension().is_some_and(|extension| extension == "rs") && name != "tests.rs"
        {
            into.push(entry);
        }
    }
}

/// The text of every product source of each crate under `crates`, by the
/// crate's name.
fn crates(folder: &Path, into: &mut BTreeMap<String, Vec<String>>) {
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.join("Cargo.toml").is_file() {
            if NOT_PRODUCT.contains(&name.as_str()) {
                continue;
            }
            let mut files = Vec::new();
            sources(&path.join("src"), &mut files);
            let texts = files
                .iter()
                .map(|file| std::fs::read_to_string(file).unwrap())
                .collect();
            into.insert(name, texts);
        } else if path.is_dir() {
            crates(&path, into);
        }
    }
}

/// The constant each setting is declared as, by the setting's name: the
/// identifier of each `pub const NAME: Setting =` of the protocol's
/// sources, with the first string after it, the setting's name.
fn constants(protocol: &[String]) -> BTreeMap<String, String> {
    const DECLARED: &str = ": Setting =";
    let mut constants = BTreeMap::new();
    for text in protocol {
        for (at, _) in text.match_indices(DECLARED) {
            let constant = text[..at].rsplit(char::is_whitespace).next().unwrap();
            let after = &text[at + DECLARED.len()..];
            let Some(name) = after.split('"').nth(1) else {
                continue;
            };
            constants.insert(name.to_owned(), constant.to_owned());
        }
    }
    constants
}

/// Whether `text` reads the setting declared as `constant`: `&NAME` or
/// `NAME.`, as a whole identifier.
fn reads(text: &str, constant: &str) -> bool {
    let part_of_a_name = |character: char| character.is_alphanumeric() || character == '_';
    text.match_indices(constant).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + constant.len()..].chars().next();
        let whole = !before.is_some_and(part_of_a_name) && !after.is_some_and(part_of_a_name);
        whole && (before == Some('&') || after == Some('.'))
    })
}

/// Each setting of the catalog is read by product code, is listed as
/// unbuilt, or is locked at its default whatever the layer (an honesty
/// guarantee, which no value can change); and no setting marked as unbuilt
/// is read outside the crate that declares it. A setting this test names is
/// one a scope may set and nothing reads: build its reader, or mark it.
///
/// Covers: story:3-88
#[test]
fn every_setting_is_read_by_product_code_or_listed_as_unbuilt() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut by_crate = BTreeMap::new();
    crates(workspace, &mut by_crate);
    let constants = constants(&by_crate[PROTOCOL]);
    let listed = |name: &str| keys::unbuilt().any(|(setting, _)| setting.name == name);
    let read_in = |name: &str, protocol: bool| {
        constants.get(name).is_some_and(|constant| {
            by_crate
                .iter()
                .filter(|(of, _)| protocol || of.as_str() != PROTOCOL)
                .any(|(_, texts)| texts.iter().any(|text| reads(text, constant)))
        })
    };

    let mut ignored = Vec::new();
    let mut built = Vec::new();
    for key in keys::all() {
        let name = key.setting.name;
        if listed(name) {
            if read_in(name, false) {
                built.push(name);
            }
        } else if !matches!(key.setting.authority, Authority::Fixed) && !read_in(name, true) {
            ignored.push(name);
        }
    }

    assert_eq!(
        (ignored, built),
        (Vec::<&str>::new(), Vec::<&str>::new()),
        "settings no product code reads that are not listed as unbuilt, then listed ones a reader was built for"
    );
}

/// A scope beneath a Run, a Session's, that sets a listed setting away
/// from its default is refused as the Run is, naming the key, the run
/// layer and the feature; one that sets it to its default opens.
///
/// Covers: story:3-88, setting:security.secure_delete
#[test]
fn a_scope_beneath_a_run_that_sets_an_unbuilt_setting_is_refused() {
    let run = resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap();
    let own = |secure_delete: bool| {
        let serde_json::Value::Object(set) =
            json!({ "security": { "secure_delete": secure_delete } })
        else {
            unreachable!("a table")
        };
        Values::read(SettingsLayer::Run, &set, Product::Collie).unwrap()
    };

    let refused = run.beneath(&own(false)).map(|_| ());
    let at_its_default = run.beneath(&own(true)).map(|_| ());

    assert!(
        matches!(
            &refused,
            Err(SettingsError::Unavailable { key, layer: SettingsLayer::Run, feature, .. })
                if key == "security.secure_delete" && feature == "plain_delete"
        ),
        "{refused:?}"
    );
    assert!(at_its_default.is_ok(), "{at_its_default:?}");
}
