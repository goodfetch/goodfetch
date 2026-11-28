//! Which `registered` values and validation checks a `hello`'s handlers give
//! (ADR 0104).

use std::collections::{BTreeMap, BTreeSet};

use goodfetch_protocol::catalog::{PluginKind, Product, ProtocolVersion, Stability};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::CONTENT_EXTRACTOR;
use goodfetch_protocol::page::proxy::PROXY_PROVIDER;
use goodfetch_protocol::plugin::extractor::EXTRACTOR;
use goodfetch_protocol::settings::{SETTING_INVALID, SettingsLayer};
use serde_json::{Map, Value, json};

use super::{check_ambiguous, check_clashes, check_extractor_known, registered};
use crate::plugin_store::PluginStore;
use crate::settings::{Layers, Values, resolve};

/// A kind one setting chooses, as area 7 declares a Proxy provider.
static CHOSEN_BY_ONE_SETTING: PluginKind = PluginKind {
    name: "proxy_provider",
    description: "Gives a Session's proxy.",
    methods: &[],
    chosen_by: Some(&PROXY_PROVIDER),
    stability: Stability::Experimental,
    arrived_in: ProtocolVersion { major: 0, minor: 1 },
};

/// A kind a reference names, as a Model provider is.
static NAMED_BY_A_REFERENCE: PluginKind = PluginKind {
    name: "model_provider",
    description: "Answers a Model reference's calls.",
    methods: &[],
    chosen_by: None,
    stability: Stability::Experimental,
    arrived_in: ProtocolVersion { major: 0, minor: 1 },
};

const KINDS: &[&PluginKind] = &[&CHOSEN_BY_ONE_SETTING, &NAMED_BY_A_REFERENCE, &EXTRACTOR];

fn handlers(kinds: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
    kinds
        .iter()
        .map(|(kind, names)| {
            (
                (*kind).to_owned(),
                names.iter().map(|name| (*name).to_owned()).collect(),
            )
        })
        .collect()
}

/// The only handler of a kind one setting chooses is that setting's value.
#[test]
fn the_only_handler_of_a_kind_one_setting_chooses_is_its_value() {
    let given = registered(&handlers(&[("proxy_provider", &["my_pool"])]), KINDS).unwrap();

    assert_eq!(Value::Object(given), json!({ "proxy.provider": "my_pool" }));
}

/// Two handlers of such a kind, or a kind no setting chooses, give no
/// value: nothing chooses between two, and a reference names the other.
#[test]
fn two_handlers_or_a_kind_no_setting_chooses_give_no_value() {
    let given = registered(
        &handlers(&[
            ("proxy_provider", &["east", "west"]),
            ("model_provider", &["inhouse"]),
        ]),
        KINDS,
    )
    .unwrap();

    assert!(given.is_empty(), "{given:?}");
}

/// A kind the engine does not carry is refused, never ignored (ADR 0083).
#[test]
fn a_kind_the_engine_does_not_carry_is_refused() {
    let refused = registered(&handlers(&[("fetch_backend", &["mine"])]), KINDS).unwrap_err();

    assert_eq!(refused.code(), ProtocolFailure::InvalidParams.name());
}

/// A handler named as a built-in Extractor or an installed Extractor Plugin
/// fails `hello` with `setting_invalid` (ADR 0104).
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "unit test writes a fixture manifest into a tempdir on its own thread"
)]
fn handler_named_as_builtin_or_installed_plugin_fails_with_setting_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let store = PluginStore::new(dir.path().to_path_buf());

    let builtin =
        check_clashes(&handlers(&[("extract", &["dom_smoothie"])]), KINDS, &store).unwrap_err();
    assert_eq!(builtin.code(), SETTING_INVALID.name);

    let bundle = dir.path().join("installed_ext").join("1.0.0");
    std::fs::create_dir_all(&bundle).unwrap();
    std::fs::write(
        bundle.join("goodfetch-plugin.json"),
        json!({
            "manifest_version": "0.1",
            "name": "installed_ext",
            "version": "1.0.0",
            "kinds": ["extract"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "plugin",
            "network_level": "none",
        })
        .to_string(),
    )
    .unwrap();

    let installed =
        check_clashes(&handlers(&[("extract", &["installed_ext"])]), KINDS, &store).unwrap_err();
    assert_eq!(installed.code(), SETTING_INVALID.name);

    check_clashes(&handlers(&[("extract", &["custom_ext"])]), KINDS, &store).unwrap();
}

/// Two handlers of a single-key kind fail `hello` with `setting_invalid`
/// when nothing else sets the choosing key, and succeed once a layer sets it.
#[test]
fn two_handlers_without_setting_fail_and_with_setting_pass() {
    let two = handlers(&[("extract", &["alpha", "beta"])]);
    let default_resolved = resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap();
    let refused = check_ambiguous(&two, KINDS, &default_resolved).unwrap_err();
    assert_eq!(refused.code(), SETTING_INVALID.name);

    let mut run_values = Map::new();
    run_values.insert("content".to_owned(), json!({ "extractor": "alpha" }));
    let layers = Layers {
        run: Values::read(SettingsLayer::Run, &run_values, Product::Collie).unwrap(),
        ..Default::default()
    };
    let chosen_resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();
    check_ambiguous(&two, KINDS, &chosen_resolved).unwrap();
}

/// A `content.extractor` naming an unknown handler (neither built-in,
/// installed Plugin, nor registered SDK handler) fails with `setting_invalid`.
#[test]
fn unknown_extractor_fails_and_registered_sdk_handler_passes() {
    let dir = tempfile::tempdir().unwrap();
    let store = PluginStore::new(dir.path().to_path_buf());
    let mut run_values = Map::new();
    run_values.insert("content".to_owned(), json!({ "extractor": "my_custom" }));
    let layers = Layers {
        run: Values::read(SettingsLayer::Run, &run_values, Product::Collie).unwrap(),
        ..Default::default()
    };
    let resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();

    let refused = check_extractor_known(&resolved, &BTreeMap::new(), &store).unwrap_err();
    assert_eq!(refused.code(), SETTING_INVALID.name);

    check_extractor_known(&resolved, &handlers(&[("extract", &["my_custom"])]), &store).unwrap();
    let _ = CONTENT_EXTRACTOR;
}
