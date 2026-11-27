use std::collections::BTreeMap;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::plugin::net::{NetFetchHeader, NetFetchParams};
use goodfetch_protocol::settings::SettingsLayer;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Value, json};
use url::Url;

use super::{CALL_BACKS, PluginFetch, SettingsSource, admit_call_back};
use crate::clock::Clock;
use crate::egress::EgressClass;
use crate::settings::{Layers, Resolved, Values, resolve};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// The settings `layer` gives when it sets `value`, a settings object.
fn values(layer: SettingsLayer, value: Value) -> Values {
    let Value::Object(nested) = value else {
        panic!("settings are an object");
    };
    Values::read(layer, &nested, Product::Collie).unwrap()
}

/// The settings that resolve when user configuration sets `user` and the
/// run sets `run`.
fn resolved(user: Value, run: Value) -> Resolved {
    let layers = Layers {
        user_configuration: values(SettingsLayer::UserConfiguration, user),
        run: values(SettingsLayer::Run, run),
        ..Layers::default()
    };
    resolve(&layers, Product::Collie, Product::Collie).unwrap()
}

/// The `net.fetch` of the Plugin `kettle` under `settings`.
fn fetch_of(settings: &Resolved) -> PluginFetch {
    PluginFetch::of(
        settings,
        "kettle",
        Clock::system(),
        TaskOwner::root(BlockingPolicy::Inline).blocking_threads(),
    )
    .unwrap()
}

/// The egress class `fetch` gives `destination`, which may be no URL.
fn class_of(fetch: &PluginFetch, destination: &str) -> EgressClass {
    fetch.class(Url::parse(destination).ok().as_ref())
}

/// The settings of the Plugin `kettle` that user configuration sets in
/// [`configured_fetch`]: two endpoints, one of them at depth.
fn configured_settings() -> Value {
    json!({
        "api": "https://api.example.com:8443/v1/items?page=2",
        "nested": { "list": ["http://127.0.0.1:8080/x", "not a url", "ftp://files.example.com/", 3] },
    })
}

/// The `net.fetch` of `kettle` when user configuration sets
/// [`configured_settings`].
fn configured_fetch() -> PluginFetch {
    fetch_of(&resolved(
        json!({ "plugins": { "kettle": { "settings": configured_settings() } } }),
        json!({}),
    ))
}

proptest! {
    /// A URL in a Plugin's settings is an Operator-configured endpoint
    /// only when the Operator policy or user or project configuration set
    /// them, and project configuration only when neither a Client acting
    /// for a model nor an Exposed Daemon's Client gave it; every other
    /// source, the Plugin's own defaults included, is checked as a page's.
    ///
    /// Covers: story:7-57
    #[test]
    fn only_a_configuration_layer_makes_a_plugins_endpoint(
        layer in proptest::option::of(proptest::sample::select(SettingsLayer::ALL.to_vec())),
        for_model in any::<bool>(),
        on_an_exposed_daemon in any::<bool>(),
    ) {
        let configured = match layer {
            Some(SettingsLayer::OperatorPolicy | SettingsLayer::UserConfiguration) => true,
            Some(SettingsLayer::ProjectConfiguration) => !for_model && !on_an_exposed_daemon,
            Some(SettingsLayer::Default | SettingsLayer::Recipe | SettingsLayer::Run) | None => false,
        };
        let class = SettingsSource { layer, for_model, on_an_exposed_daemon }.class();

        prop_assert_eq!(class == EgressClass::OperatorConfigured, configured);
    }

    /// A destination is one of the Plugin's endpoints exactly when its
    /// scheme, host and port, the scheme's default when it names none, are
    /// those of a URL its configured settings hold; its path and query do
    /// not matter.
    ///
    /// Covers: story:7-57, story:7-58
    #[test]
    fn a_destination_is_an_endpoint_only_at_a_configured_origin(
        scheme in proptest::sample::select(vec!["http", "https"]),
        host in proptest::sample::select(vec!["api.example.com", "127.0.0.1", "other.example"]),
        port in proptest::option::of(proptest::sample::select(vec![80_u16, 443, 8080, 8443])),
        path in "(/[a-z0-9]{0,6}){0,3}",
    ) {
        let configured = [("https", "api.example.com", 8443), ("http", "127.0.0.1", 8080)];
        let effective = port.unwrap_or(if scheme == "http" { 80 } else { 443 });
        let expected = configured.contains(&(scheme, host, effective));
        let authority = port.map_or_else(|| host.to_owned(), |port| format!("{host}:{port}"));

        let class = class_of(&configured_fetch(), &format!("{scheme}://{authority}{path}"));

        prop_assert_eq!(class == EgressClass::OperatorConfigured, expected);
    }
}

/// The endpoints are the origins of the `http` and `https` URLs the
/// Plugin's settings hold at any depth, when a configuration layer set
/// them; text that is no such URL names none, and settings a run gave name
/// none at all.
///
/// Covers: story:7-57, story:7-58
#[rstest]
#[case::at_the_top("https://api.example.com:8443/other", true)]
#[case::at_depth("http://127.0.0.1:8080/", true)]
#[case::another_port("https://api.example.com/", false)]
#[case::an_ftp_url("ftp://files.example.com/", false)]
#[case::no_url("not a url", false)]
fn the_endpoints_are_the_origins_of_the_urls_a_configuration_layer_set(
    #[case] destination: &str,
    #[case] endpoint: bool,
) {
    let run = fetch_of(&resolved(
        json!({}),
        json!({ "plugins": { "kettle": { "settings": configured_settings() } } }),
    ));

    assert_eq!(
        class_of(&configured_fetch(), destination) == EgressClass::OperatorConfigured,
        endpoint
    );
    assert_eq!(class_of(&run, destination), EgressClass::PageReachable);
}

/// A request the engine would send for a Plugin is refused before any
/// connection when it is no request, asks for what the Plugin may not do,
/// or heads where a page may not go: `CONNECT`, a method other than `GET`
/// or `HEAD` toward a destination checked as a page's (an Effect), a
/// header the engine sets itself, a scheme other than `http` or `https`,
/// or a loopback address that is none of the Plugin's endpoints, whose
/// refusal names no setting, since the run's private ranges never reach a
/// Plugin.
///
/// Covers: story:7-57, story:7-58
#[rstest]
#[case::connect("CONNECT", "https://example.com/", &[], "plugin_call_back_refused")]
#[case::an_effect("POST", "https://example.com/", &[], "plugin_call_back_refused")]
#[case::method_override("GET", "https://example.com/", &[("X-HTTP-Method-Override", "DELETE")], "plugin_call_back_refused")]
#[case::no_method("NOT A METHOD", "https://example.com/", &[], "invalid_params")]
#[case::host_header("GET", "https://example.com/", &[("Host", "other.example")], "plugin_call_back_refused")]
#[case::proxy_header("GET", "https://example.com/", &[("Proxy-Authorization", "Basic eA==")], "plugin_call_back_refused")]
#[case::no_header_value("GET", "https://example.com/", &[("x-test", "a\nb")], "invalid_params")]
#[case::websocket("GET", "wss://example.com/", &[], "egress_refused")]
#[case::loopback("GET", "http://127.0.0.1:9/", &[], "egress_refused")]
fn a_request_a_plugin_may_not_send_is_refused_before_any_connection(
    #[case] method: &str,
    #[case] url: &str,
    #[case] headers: &[(&str, &str)],
    #[case] code: &str,
) {
    let fetch = fetch_of(&resolved(
        json!({ "egress": { "private_ranges": ["127.0.0.0/8"] } }),
        json!({}),
    ));
    let params = NetFetchParams {
        call_id: RequestId::Number(1),
        method: method.to_owned(),
        url: url.to_owned(),
        headers: headers
            .iter()
            .map(|(name, value)| NetFetchHeader {
                name: (*name).to_owned(),
                value: (*value).to_owned(),
            })
            .collect(),
        body: None,
        secrets: Vec::new(),
    };

    let error = goodfetch_test_support::runtime()
        .block_on(fetch.fetch(params))
        .unwrap_err();

    assert_eq!(error.code(), code);
    let setting = error
        .details()
        .and_then(|details| details.get("setting"))
        .filter(|setting| !setting.is_null());
    assert_eq!(setting, None);
}

/// The calls an instance awaits, by their ids; each value stands in for
/// what the instance keeps for a call.
fn awaiting(ids: impl IntoIterator<Item = i64>) -> BTreeMap<RequestId, &'static str> {
    ids.into_iter()
        .map(|id| (RequestId::Number(id), "extract"))
        .collect()
}

proptest! {
    /// A call back naming no call the instance awaits is never admitted,
    /// however few run; one naming an awaited call is admitted exactly
    /// while fewer than [`CALL_BACKS`] run.
    ///
    /// Covers: story:7-53
    #[test]
    fn a_call_back_is_admitted_only_inside_a_call_and_under_the_most_at_once(
        awaited in proptest::collection::btree_set(0..8_i64, 0..4),
        named in 0..8_i64,
        running in 0..2 * CALL_BACKS,
    ) {
        let inside_a_call = awaited.contains(&named);

        let admitted = admit_call_back(&awaiting(awaited), running, &RequestId::Number(named));

        prop_assert_eq!(admitted.is_ok(), inside_a_call && running < CALL_BACKS);
        if let Err(error) = admitted {
            prop_assert_eq!(error.code(), "plugin_call_back_refused");
        }
    }
}

/// Covers: story:7-53
#[rstest]
#[case::inside_its_call(&[1, 2], 0, 2, true)]
#[case::the_last_one_allowed(&[1], CALL_BACKS - 1, 1, true)]
#[case::past_the_most_at_once(&[1], CALL_BACKS, 1, false)]
#[case::naming_another_call(&[1], 0, 3, false)]
#[case::before_any_call(&[], 0, 1, false)]
fn a_call_back_is_admitted_inside_the_call_it_names(
    #[case] awaited: &[i64],
    #[case] running: usize,
    #[case] named: i64,
    #[case] admitted: bool,
) {
    let decision = admit_call_back(
        &awaiting(awaited.iter().copied()),
        running,
        &RequestId::Number(named),
    );

    assert_eq!(decision.is_ok(), admitted, "{decision:?}");
}
