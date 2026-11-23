//! Which `registered` values a `hello`'s handlers give (ADR 0104).

use std::collections::{BTreeMap, BTreeSet};

use goodfetch_protocol::catalog::{PluginKind, ProtocolVersion, Stability};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::proxy::PROXY_PROVIDER;
use serde_json::json;

use super::registered;

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

const KINDS: &[&PluginKind] = &[&CHOSEN_BY_ONE_SETTING, &NAMED_BY_A_REFERENCE];

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

    assert_eq!(
        serde_json::Value::Object(given),
        json!({ "proxy.provider": "my_pool" })
    );
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
