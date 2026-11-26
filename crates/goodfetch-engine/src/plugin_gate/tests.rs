use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::plugin::net::{NetFetchHeader, NetFetchParams};
use goodfetch_protocol::settings::SettingsLayer;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Value, json};

use super::{PluginFetch, SettingsSource};
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

/// The origins of `fetch`'s endpoints, sorted.
fn endpoints(fetch: &PluginFetch) -> Vec<String> {
    let mut origins: Vec<String> = fetch
        .endpoints
        .iter()
        .map(url::Origin::ascii_serialization)
        .collect();
    origins.sort();
    origins
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
}

/// The endpoints are the origins of the `http` and `https` URLs the
/// Plugin's settings hold at any depth, when a configuration layer set
/// them; text that is no such URL names none, and settings a run gave name
/// none at all.
///
/// Covers: story:7-57, story:7-58
#[test]
fn the_endpoints_are_the_origins_of_the_urls_a_configuration_layer_set() {
    let settings = json!({
        "api": "https://api.example.com/v1/items?page=2",
        "nested": { "list": ["http://127.0.0.1:8080/x", "not a url", "ftp://files.example.com/", 3] },
        "again": "https://api.example.com/other",
    });
    let user = resolved(
        json!({ "plugins": { "kettle": { "settings": settings.clone() } } }),
        json!({}),
    );
    let run = resolved(
        json!({}),
        json!({ "plugins": { "kettle": { "settings": settings } } }),
    );

    assert_eq!(
        endpoints(&fetch_of(&user)),
        ["http://127.0.0.1:8080", "https://api.example.com"]
    );
    assert_eq!(endpoints(&fetch_of(&run)), Vec::<String>::new());
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
