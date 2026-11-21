//! The settings resolver through its interface, in process (spec #213's
//! module seam): layers given as the files and values each layer writes.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::Duration;

use goodfetch_protocol::catalog::{Product, Setting};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::secrets::RUN_STATE_KEY_REFERENCE;
use goodfetch_protocol::settings::{
    Exposure, RecipeManifest, RunOrigin, SettingLimit, SettingsLayer, TIMEOUT_NAVIGATION,
    TIMEOUT_RUN, WeakenedSetting,
};
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::{
    HostDefaults, Layers, Policy, Recipe, Resolved, SettingsError, SettingsFiles, Values,
    check_environment, configuration, get_or_default, operator_policy, resolve, rewrite,
};
use crate::home::Home;

fn nested(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => panic!("settings are a table"),
    }
}

fn values(layer: SettingsLayer, value: Value) -> Values {
    Values::read(layer, &nested(value), Product::Foxhound).unwrap()
}

fn policy(settings: Value, lock: Value, cap: Value) -> Policy {
    Policy {
        settings: values(SettingsLayer::OperatorPolicy, settings),
        lock: values(SettingsLayer::OperatorPolicy, lock),
        cap: values(SettingsLayer::OperatorPolicy, cap),
        version: Some("sha256:0123456789abcdef".to_owned()),
        managed: None,
    }
}

fn recipe(settings: Value, expose: &[(&str, Exposure)]) -> Recipe {
    Recipe::read(
        &RecipeManifest {
            name: "listings".to_owned(),
            product: Product::Collie,
            settings: nested(settings),
            expose: expose
                .iter()
                .map(|(key, exposure)| ((*key).to_owned(), exposure.clone()))
                .collect(),
        },
        Product::Foxhound,
    )
    .unwrap()
}

/// The layers with `value` for `key` in `layer` alone.
fn only(layer: SettingsLayer, key: &str, value: &Value) -> Layers {
    let set = json!({ key: value });
    let mut layers = Layers::default();
    match layer {
        SettingsLayer::OperatorPolicy => {
            layers.operator_policy = policy(set, json!({}), json!({}));
        }
        SettingsLayer::UserConfiguration => {
            layers.user_configuration = values(layer, set);
        }
        SettingsLayer::ProjectConfiguration => {
            layers.project_configuration = values(layer, set);
        }
        SettingsLayer::Recipe => layers.recipe = Some(recipe(set, &[])),
        SettingsLayer::Run => layers.run = values(layer, set),
        SettingsLayer::Default => {}
    }
    layers
}

/// The catalog's setting named `name`.
fn catalog_key(name: &str) -> &'static Setting {
    super::setting(name).expect("a key of the catalog")
}

fn resolved(layers: &Layers) -> Resolved {
    resolve(layers, Product::Foxhound, Product::Foxhound).unwrap()
}

fn refusal(layers: &Layers) -> SettingsError {
    resolve(layers, Product::Foxhound, Product::Foxhound).unwrap_err()
}

fn every_layer_sets_navigation() -> Layers {
    Layers {
        host: HostDefaults::default(),
        operator_policy: policy(
            json!({ "timeout": { "navigation": 1_000 } }),
            json!({}),
            json!({}),
        ),
        user_configuration: values(
            SettingsLayer::UserConfiguration,
            json!({ "timeout": { "navigation": 2_000 } }),
        ),
        project_configuration: values(
            SettingsLayer::ProjectConfiguration,
            json!({ "timeout.navigation": 3_000 }),
        ),
        recipe: Some(recipe(
            json!({ "timeout": { "navigation": 4_000 } }),
            &[("timeout.navigation", Exposure::default())],
        )),
        run: values(
            SettingsLayer::Run,
            json!({ "timeout": { "navigation": 5_000 } }),
        )
        .from_origin(RunOrigin::Settings),
    }
}

/// Covers: story:3-86, setting:timeout.navigation
#[test]
fn the_highest_layer_that_sets_a_value_wins_where_nothing_locked_it() {
    let effective = resolved(&every_layer_sets_navigation());

    let navigation = effective
        .effective(catalog_key("timeout.navigation"))
        .unwrap();
    assert_eq!(
        (
            &navigation.value,
            navigation.layer,
            navigation.origin,
            navigation.locked_by
        ),
        (
            &json!(5_000),
            SettingsLayer::Run,
            Some(RunOrigin::Settings),
            None
        )
    );
}

/// Covers: story:3-86
#[rstest]
#[case::the_default(SettingsLayer::Default, json!(30_000))]
#[case::the_operator_policy(SettingsLayer::OperatorPolicy, json!(1_000))]
#[case::user_configuration(SettingsLayer::UserConfiguration, json!(1_000))]
#[case::project_configuration(SettingsLayer::ProjectConfiguration, json!(1_000))]
#[case::the_recipe(SettingsLayer::Recipe, json!(1_000))]
#[case::the_run(SettingsLayer::Run, json!(1_000))]
fn a_value_set_in_one_layer_is_the_effective_one_with_that_layer(
    #[case] layer: SettingsLayer,
    #[case] value: Value,
) {
    let effective = resolved(&only(layer, "timeout.navigation", &value));

    let navigation = effective
        .effective(catalog_key("timeout.navigation"))
        .unwrap();
    assert_eq!((&navigation.value, navigation.layer), (&value, layer));
}

/// `null` clears a setting that can be cleared, over a lower layer's value
/// (ADR 0083's three states).
///
/// Covers: story:3-86, setting:timeout.run
#[test]
fn null_from_the_run_clears_a_lower_layers_value() {
    let mut layers = only(SettingsLayer::Run, "timeout.run", &Value::Null);
    layers.user_configuration = values(
        SettingsLayer::UserConfiguration,
        json!({ "timeout": { "run": 3_600_000 } }),
    );

    let effective = resolved(&layers);

    let run = effective.effective(catalog_key("timeout.run")).unwrap();
    assert_eq!((&run.value, run.layer), (&Value::Null, SettingsLayer::Run));
}

/// A setting reads as its kind's type: a clearable one as an `Option`,
/// `None` once a layer cleared it.
///
/// Covers: story:3-86, setting:timeout.run, setting:timeout.navigation
#[test]
fn a_setting_reads_as_its_kind_and_a_cleared_one_as_none() {
    let set = resolved(&only(SettingsLayer::Run, "timeout.run", &json!(3_600_000)));
    let cleared = resolved(&only(SettingsLayer::Run, "timeout.run", &Value::Null));

    assert_eq!(
        (
            set.get::<Option<Duration>>(&TIMEOUT_RUN),
            cleared.get::<Option<Duration>>(&TIMEOUT_RUN),
            set.get::<Duration>(&TIMEOUT_NAVIGATION),
        ),
        (
            Some(Duration::from_secs(3_600)),
            None,
            Duration::from_secs(30)
        )
    );
}

/// A Run of a Product that does not honour a key reads its catalog
/// default, as a scope that resolved no settings does.
///
/// Covers: story:3-86, setting:provenance.log, setting:crawl.max_requests
#[test]
fn a_key_the_runs_product_does_not_honour_reads_as_its_catalog_default() {
    let collie = resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap();
    let provenance_log = super::setting("provenance.log").unwrap();
    let max_requests = super::setting("crawl.max_requests").unwrap();

    assert_eq!(
        (
            collie.effective(provenance_log).is_none(),
            collie.get::<bool>(provenance_log),
            collie.get::<Option<u64>>(max_requests),
            get_or_default::<bool>(None, provenance_log),
        ),
        (true, true, None, true)
    );
}

/// Reading a setting as a type its kind is not is a mistake of the
/// reading code, refused at once rather than read as some default.
///
/// Covers: story:3-86, setting:timeout.navigation
#[test]
fn a_setting_read_as_another_kind_is_refused() {
    let effective = resolved(&Layers::default());

    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        effective.get::<u64>(&TIMEOUT_NAVIGATION)
    }));

    assert!(read.is_err(), "{read:?}");
}

/// Covers: story:3-86, setting:egress.offline
#[rstest]
#[case::by_default(None, false)]
#[case::from_user_configuration(Some(true), true)]
fn offline_is_off_unless_a_layer_turns_it_on(#[case] set: Option<bool>, #[case] offline: bool) {
    let layers = set.map_or_else(Layers::default, |value| {
        only(
            SettingsLayer::UserConfiguration,
            "egress.offline",
            &json!(value),
        )
    });

    let effective = resolved(&layers);

    assert_eq!(
        effective.get::<bool>(catalog_key("egress.offline")),
        offline
    );
}

/// Covers: story:3-86, story:3-88
#[rstest]
#[case::user_configuration(SettingsLayer::UserConfiguration)]
#[case::project_configuration(SettingsLayer::ProjectConfiguration)]
#[case::the_recipe(SettingsLayer::Recipe)]
#[case::the_run(SettingsLayer::Run)]
fn an_operator_policy_lock_refuses_every_higher_layers_value(#[case] layer: SettingsLayer) {
    let mut layers = only(layer, "timeout.navigation", &json!(9_000));
    layers.operator_policy = policy(
        json!({}),
        json!({ "timeout": { "navigation": 10_000 } }),
        json!({}),
    );

    let refused = refusal(&layers);

    assert_eq!(
        refused,
        SettingsError::Locked {
            key: "timeout.navigation".to_owned(),
            layer,
            limit: SettingLimit {
                key: "timeout.navigation".to_owned(),
                value: json!(10_000),
                layer: SettingsLayer::OperatorPolicy,
                origin: None,
            },
            fix: "ask the operator to change the Operator policy's lock on `timeout.navigation`"
                .to_owned(),
        }
    );
}

/// A program's registered handler is the run layer's value with the origin
/// `registered`, which the program never wrote: a lock, from an Operator
/// policy or a Recipe, wins over it with no refusal (ADR 0104's amendment
/// of ADR 0083).
#[rstest]
#[case::by_an_operator_policy(
    Layers {
        operator_policy: policy(json!({}), json!({ "timeout": { "navigation": 10_000 } }), json!({})),
        ..Layers::default()
    },
    SettingsLayer::OperatorPolicy,
    json!(10_000)
)]
#[case::by_a_recipe(
    Layers {
        recipe: Some(recipe(json!({ "timeout": { "navigation": 4_000 } }), &[])),
        ..Layers::default()
    },
    SettingsLayer::Recipe,
    json!(4_000)
)]
fn a_lock_wins_over_a_registered_value_with_no_refusal(
    #[case] mut layers: Layers,
    #[case] locking: SettingsLayer,
    #[case] locked: Value,
) {
    layers.run = values(
        SettingsLayer::Run,
        json!({ "timeout": { "navigation": 9_000 } }),
    )
    .from_origin(RunOrigin::Registered);

    let effective = resolved(&layers);

    let navigation = effective
        .effective(catalog_key("timeout.navigation"))
        .unwrap();
    assert_eq!(
        (
            &navigation.value,
            navigation.layer,
            navigation.origin,
            navigation.locked_by
        ),
        (&locked, locking, None, Some(locking))
    );
}

/// Unlocked, the registered value is the run layer's, with its origin.
#[test]
fn an_unlocked_registered_value_is_the_run_layers_with_its_origin() {
    let layers = Layers {
        run: values(
            SettingsLayer::Run,
            json!({ "timeout": { "navigation": 9_000 } }),
        )
        .from_origin(RunOrigin::Registered),
        ..every_layer_sets_navigation()
    };

    let effective = resolved(&layers);

    let navigation = effective
        .effective(catalog_key("timeout.navigation"))
        .unwrap();
    assert_eq!(
        (&navigation.value, navigation.layer, navigation.origin),
        (
            &json!(9_000),
            SettingsLayer::Run,
            Some(RunOrigin::Registered)
        )
    );
}

/// Covers: story:3-88, story:3-89
#[test]
fn a_locked_value_says_the_layer_that_locked_it() {
    let layers = Layers {
        operator_policy: policy(
            json!({}),
            json!({ "network": { "resolve": { "staging.example": ["127.0.0.1"] } } }),
            json!({}),
        ),
        ..Layers::default()
    };

    let effective = resolved(&layers);

    let resolve = effective.effective(catalog_key("network.resolve")).unwrap();
    assert_eq!(
        (&resolve.value, resolve.layer, resolve.locked_by),
        (
            &json!({ "staging.example": ["127.0.0.1"] }),
            SettingsLayer::OperatorPolicy,
            Some(SettingsLayer::OperatorPolicy)
        )
    );
}

/// Covers: story:3-134, setting:network.resolve
#[test]
fn resolve_gives_each_host_its_addresses_for_a_resolvers_mapping() {
    let effective = resolved(&only(
        SettingsLayer::UserConfiguration,
        "network.resolve",
        &json!({ "staging.example": ["127.0.0.1", "::1"] }),
    ));

    assert_eq!(
        effective.hosts(catalog_key("network.resolve")),
        [(
            "staging.example".to_owned(),
            vec![
                std::net::IpAddr::from([127, 0, 0, 1]),
                std::net::IpAddr::from(std::net::Ipv6Addr::LOCALHOST)
            ]
        )]
    );
}

/// Covers: story:3-134, setting:network.resolve
#[rstest]
#[case::the_operator_policy(SettingsLayer::OperatorPolicy)]
#[case::user_configuration(SettingsLayer::UserConfiguration)]
#[case::project_configuration(SettingsLayer::ProjectConfiguration)]
#[case::the_recipe(SettingsLayer::Recipe)]
#[case::the_run(SettingsLayer::Run)]
fn resolve_maps_host_names_from_every_layer(#[case] layer: SettingsLayer) {
    let mapping = json!({ "staging.example": ["127.0.0.1", "::1"] });

    let effective = resolved(&only(layer, "network.resolve", &mapping));

    assert_eq!(effective.value(catalog_key("network.resolve")), &mapping);
}

/// Covers: story:3-87
#[test]
fn a_recipes_declared_setting_is_locked_for_its_users() {
    let layers = Layers {
        recipe: Some(recipe(json!({ "timeout": { "navigation": 4_000 } }), &[])),
        run: values(
            SettingsLayer::Run,
            json!({ "timeout": { "navigation": 5_000 } }),
        ),
        ..Layers::default()
    };

    let refused = refusal(&layers);

    assert_eq!(refused.code(), "setting_locked");
}

/// Covers: story:3-87
#[test]
fn a_recipes_declared_setting_says_the_recipe_locked_it() {
    let layers = Layers {
        recipe: Some(recipe(json!({ "timeout": { "navigation": 4_000 } }), &[])),
        ..Layers::default()
    };

    let effective = resolved(&layers);

    let navigation = effective
        .effective(catalog_key("timeout.navigation"))
        .unwrap();
    assert_eq!(
        (navigation.layer, navigation.locked_by),
        (SettingsLayer::Recipe, Some(SettingsLayer::Recipe))
    );
}

fn bounded() -> Exposure {
    Exposure {
        minimum: Some(1_000),
        maximum: Some(10_000),
        one_of: Vec::new(),
    }
}

/// Covers: story:3-87
#[rstest]
#[case::within_its_bounds(json!(7_000), true)]
#[case::at_its_maximum(json!(10_000), true)]
#[case::below_its_minimum(json!(500), false)]
#[case::above_its_maximum(json!(20_000), false)]
fn a_recipes_exposed_setting_takes_a_value_within_its_bounds(
    #[case] value: Value,
    #[case] accepted: bool,
) {
    let layers = Layers {
        recipe: Some(recipe(
            json!({ "timeout": { "navigation": 4_000 } }),
            &[("timeout.navigation", bounded())],
        )),
        run: values(
            SettingsLayer::Run,
            json!({ "timeout": { "navigation": value } }),
        ),
        ..Layers::default()
    };

    let outcome = resolve(&layers, Product::Foxhound, Product::Foxhound).map(|_| ());

    assert_eq!(outcome.is_ok(), accepted);
}

/// Covers: story:3-87
#[test]
fn a_value_outside_a_recipes_bounds_names_the_bounds_as_its_limit() {
    let layers = Layers {
        recipe: Some(recipe(json!({}), &[("timeout.navigation", bounded())])),
        run: values(
            SettingsLayer::Run,
            json!({ "timeout": { "navigation": 20_000 } }),
        ),
        ..Layers::default()
    };

    let refused = refusal(&layers);

    let SettingsError::Locked { limit, .. } = refused else {
        panic!("expected setting_locked, got {refused:?}");
    };
    assert_eq!(
        limit,
        SettingLimit {
            key: "timeout.navigation".to_owned(),
            value: json!({ "minimum": 1_000, "maximum": 10_000 }),
            layer: SettingsLayer::Recipe,
            origin: None,
        }
    );
}

/// A Recipe that declares a security control on, or exposes it held to
/// on, keeps its runs from switching it off; one that exposes it unbounded
/// lets the run switch it off, which then weakens the run (positive
/// control).
/// Covers: story:4-75, setting:security.audit_chain, error:setting_locked
#[rstest]
#[case::declared_on(recipe(json!({ "security": { "audit_chain": true } }), &[]), Err("setting_locked"))]
#[case::exposed_held_on(
    recipe(
        json!({}),
        &[("security.audit_chain", Exposure { one_of: vec![json!(true)], ..Exposure::default() })],
    ),
    Err("setting_locked")
)]
#[case::exposed_unbounded(
    recipe(json!({}), &[("security.audit_chain", Exposure::default())]),
    Ok(vec!["security.audit_chain".to_owned()])
)]
fn a_recipe_caps_a_security_control_its_runs_would_switch_off(
    #[case] recipe: Recipe,
    #[case] expected: Result<Vec<String>, &str>,
) {
    let layers = Layers {
        recipe: Some(recipe),
        run: values(
            SettingsLayer::Run,
            json!({ "security": { "audit_chain": false } }),
        ),
        ..Layers::default()
    };

    let outcome = resolve(&layers, Product::Foxhound, Product::Foxhound);

    assert_eq!(
        outcome
            .map(|resolved| {
                resolved
                    .weakened()
                    .into_iter()
                    .map(|weakened| weakened.key)
                    .collect()
            })
            .map_err(|refused| refused.code()),
        expected
    );
}

/// An Operator policy that locks `site_preferences.respect_robots_txt`
/// on, or a Recipe that declares it on or exposes it held to on, keeps a
/// run from turning robots.txt off (ADRs 0028 and 0086), so a shared
/// Daemon can forbid it; a Recipe that exposes it unbounded lets the run
/// turn it off, which then weakens the run (positive control).
/// Covers: story:4-119, setting:site_preferences.respect_robots_txt, error:setting_locked
#[rstest]
#[case::locked_by_the_operator_policy(
    Some(policy(json!({}), json!({ "site_preferences": { "respect_robots_txt": true } }), json!({}))),
    None,
    Err("setting_locked")
)]
#[case::declared_on_by_a_recipe(
    None,
    Some(recipe(json!({ "site_preferences": { "respect_robots_txt": true } }), &[])),
    Err("setting_locked")
)]
#[case::exposed_held_on_by_a_recipe(
    None,
    Some(recipe(
        json!({}),
        &[("site_preferences.respect_robots_txt", Exposure { one_of: vec![json!(true)], ..Exposure::default() })],
    )),
    Err("setting_locked")
)]
#[case::exposed_unbounded_by_a_recipe(
    None,
    Some(recipe(json!({}), &[("site_preferences.respect_robots_txt", Exposure::default())])),
    Ok(vec!["site_preferences.respect_robots_txt".to_owned()])
)]
fn a_policy_or_a_recipe_keeps_a_run_from_turning_robots_txt_off(
    #[case] locking: Option<Policy>,
    #[case] recipe: Option<Recipe>,
    #[case] expected: Result<Vec<String>, &str>,
) {
    let layers = Layers {
        operator_policy: locking.unwrap_or_default(),
        recipe,
        run: values(
            SettingsLayer::Run,
            json!({ "site_preferences": { "respect_robots_txt": false } }),
        ),
        ..Layers::default()
    };

    let outcome = resolve(&layers, Product::Foxhound, Product::Foxhound);

    assert_eq!(
        outcome
            .map(|resolved| {
                resolved
                    .weakened()
                    .into_iter()
                    .map(|weakened| weakened.key)
                    .collect()
            })
            .map_err(|refused| refused.code()),
        expected
    );
}

/// The keys a Recipe and the run only narrow, or never set (spec #213's
/// story 90): each refused when it widens or sets what is reserved.
/// Covers: story:3-90, story:5-101, setting:paid_fetch.domains, setting:paid_fetch.max_price_per_page, setting:consent.domains, setting:wall.domains, setting:wall.refetch_domains, setting:wall.denied_domains, setting:tls.trusted_cas, setting:tls.accept_invalid_certs, setting:paid_fetch.exchange_rates, setting:paid_fetch.chain_rpc_endpoints
#[rstest]
#[case::paid_fetch_domains_widened_by_the_run(
    "paid_fetch.domains", json!(["shop.example"]), SettingsLayer::Run, json!(["shop.example", "other.example"])
)]
#[case::paid_fetch_domains_widened_by_a_recipe(
    "paid_fetch.domains", json!(["shop.example"]), SettingsLayer::Recipe, json!(["other.example"])
)]
#[case::max_price_per_page_raised_by_the_run(
    "paid_fetch.max_price_per_page", json!("0.05 USD"), SettingsLayer::Run, json!("0.10 USD")
)]
#[case::consent_domains_widened_by_the_run(
    "consent.domains", json!(["news.example"]), SettingsLayer::Run, json!(["news.example", "blog.example"])
)]
#[case::consent_domains_widened_by_a_recipe(
    "consent.domains", json!(["news.example"]), SettingsLayer::Recipe, json!(["blog.example"])
)]
#[case::wall_domains_widened_by_the_run(
    "wall.domains", json!(["paper.example"]), SettingsLayer::Run, json!(["other.example"])
)]
#[case::wall_refetch_domains_widened_by_the_run(
    "wall.refetch_domains", json!([]), SettingsLayer::Run, json!(["paper.example"])
)]
#[case::wall_denied_domains_from_user_configuration(
    "wall.denied_domains", json!(["paper.example"]), SettingsLayer::UserConfiguration, json!([])
)]
#[case::trusted_cas_from_the_run(
    "tls.trusted_cas", json!({}), SettingsLayer::Run, json!({ "intranet.example": ["ca.pem"] })
)]
#[case::trusted_cas_from_a_recipe(
    "tls.trusted_cas", json!({}), SettingsLayer::Recipe, json!({ "intranet.example": ["ca.pem"] })
)]
#[case::a_pin_from_the_run(
    "tls.accept_invalid_certs", json!({}), SettingsLayer::Run, json!({ "intranet.example": "sha256/AAAA" })
)]
#[case::any_from_a_recipe(
    "tls.accept_invalid_certs", json!({}), SettingsLayer::Recipe, json!({ "intranet.example": "any" })
)]
#[case::exchange_rates_from_the_run(
    "paid_fetch.exchange_rates", json!({}), SettingsLayer::Run, json!({ "EUR/USD": "1.1" })
)]
#[case::chain_rpc_endpoints_from_a_recipe(
    "paid_fetch.chain_rpc_endpoints", json!({}), SettingsLayer::Recipe, json!({ "base": "https://rpc.example" })
)]
fn a_key_reserved_to_lower_layers_refuses_a_wider_value_from_above(
    #[case] key: &str,
    #[case] configured: Value,
    #[case] layer: SettingsLayer,
    #[case] value: Value,
) {
    let mut layers = only(layer, key, &value);
    layers.operator_policy = policy(json!({ key: configured }), json!({}), json!({}));

    let refused = refusal(&layers);

    assert_eq!(refused.code(), "setting_locked");
}

/// Story 4-37 (ADR 0042): `any` is taken from configuration only: a Recipe
/// and a run that list a domain with it are refused, whatever the lower
/// layers set, and user and project configuration set it (positive
/// control).
/// Covers: story:4-37, setting:tls.accept_invalid_certs
#[rstest]
#[case::from_a_recipe(SettingsLayer::Recipe, false)]
#[case::from_the_run(SettingsLayer::Run, false)]
#[case::from_user_configuration(SettingsLayer::UserConfiguration, true)]
#[case::from_project_configuration(SettingsLayer::ProjectConfiguration, true)]
fn any_is_taken_from_configuration_only(#[case] layer: SettingsLayer, #[case] taken: bool) {
    let any = json!({ "fixture.test": "any" });
    let layers = only(layer, "tls.accept_invalid_certs", &any);

    let resolved = resolve(&layers, Product::Foxhound, Product::Foxhound);

    match resolved {
        Ok(effective) => assert_eq!(
            (
                taken,
                effective.value(catalog_key("tls.accept_invalid_certs"))
            ),
            (true, &any)
        ),
        Err(refused) => assert_eq!((taken, refused.code()), (false, "setting_locked")),
    }
}

/// Story 4-34 (ADR 0042): a Recipe and a run set a domain's client
/// certificate as configuration does, unlike `tls.trusted_cas`.
/// Covers: story:4-34, setting:tls.client_certificates
#[rstest]
#[case::from_a_recipe(SettingsLayer::Recipe)]
#[case::from_the_run(SettingsLayer::Run)]
#[case::from_user_configuration(SettingsLayer::UserConfiguration)]
fn a_client_certificate_is_taken_from_a_recipe_and_the_run(#[case] layer: SettingsLayer) {
    let certificates =
        json!({ "mtls.example": { "certificate": "client.pem", "key": "env:CLIENT_KEY" } });
    let layers = only(layer, "tls.client_certificates", &certificates);

    let resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();

    assert_eq!(
        resolved.value(catalog_key("tls.client_certificates")),
        &certificates
    );
}

/// Story 4-34: a client certificate's key given as a raw value, from any
/// layer, is refused with `setting_invalid` naming the setting, and
/// neither the error's message nor its reason repeats the key.
/// Covers: story:4-34, setting:tls.client_certificates, error:setting_invalid
#[rstest]
#[case::from_the_run(SettingsLayer::Run)]
#[case::from_user_configuration(SettingsLayer::UserConfiguration)]
fn a_raw_client_key_is_refused_with_setting_invalid_without_its_value(
    #[case] layer: SettingsLayer,
) {
    let raw = "-----BEGIN PRIVATE KEY-----\nc4n4ryk3yAgEAMBMGByqGSM49AgEGCCqGSM49AwEH\n-----END PRIVATE KEY-----";
    let set = json!({
        "tls.client_certificates": { "mtls.example": { "certificate": "client.pem", "key": raw } },
    });

    let refused = Values::read(layer, &nested(set), Product::Collie).unwrap_err();

    let shown = format!("{refused} {refused:?}");
    let SettingsError::Invalid {
        key,
        layer: refused_in,
        ..
    } = &refused
    else {
        panic!("not setting_invalid: {shown}");
    };
    assert_eq!(
        (key.as_str(), *refused_in, shown.contains("c4n4ryk3y")),
        ("tls.client_certificates", Some(layer), false),
        "{shown}"
    );
}

/// Covers: story:3-90, setting:paid_fetch.domains, setting:paid_fetch.max_price_per_page, setting:consent.domains, setting:wall.domains, setting:wall.refetch_domains, setting:tls.accept_invalid_certs
#[rstest]
#[case::paid_fetch_domains_narrowed(
    "paid_fetch.domains", json!(["shop.example", "other.example"]), SettingsLayer::Run, json!(["shop.example"])
)]
#[case::max_price_per_page_lowered(
    "paid_fetch.max_price_per_page", json!("0.05 USD"), SettingsLayer::Recipe, json!("0.01 USD")
)]
#[case::consent_domains_narrowed(
    "consent.domains", json!(["news.example"]), SettingsLayer::Run, json!([])
)]
#[case::wall_domains_narrowed(
    "wall.domains", json!(["paper.example", "daily.example"]), SettingsLayer::Run, json!(["daily.example"])
)]
#[case::wall_refetch_domains_narrowed(
    "wall.refetch_domains", json!(["paper.example"]), SettingsLayer::Recipe, json!(["paper.example"])
)]
#[case::trust_on_first_use_from_the_run(
    "tls.accept_invalid_certs", json!({}), SettingsLayer::Run, json!({ "intranet.example": null })
)]
fn a_key_reserved_to_lower_layers_takes_a_narrower_value_from_above(
    #[case] key: &str,
    #[case] configured: Value,
    #[case] layer: SettingsLayer,
    #[case] value: Value,
) {
    let mut layers = only(layer, key, &value);
    layers.operator_policy = policy(json!({ key: configured }), json!({}), json!({}));

    let effective = resolved(&layers);

    assert_eq!(
        (
            effective.value(catalog_key(key)),
            effective.effective(catalog_key(key)).unwrap().layer
        ),
        (&value, layer)
    );
}

/// Covers: story:3-90, setting:filter_lists.extra
#[rstest]
#[case::from_configuration(SettingsLayer::UserConfiguration, true)]
#[case::from_the_run(SettingsLayer::Run, false)]
fn a_filter_lists_scriptlets_run_only_when_configuration_set_it(
    #[case] layer: SettingsLayer,
    #[case] scriptlets: bool,
) {
    let effective = resolved(&only(
        layer,
        "filter_lists.extra",
        &json!(["lists/extra.txt"]),
    ));

    assert_eq!(
        effective
            .effective(catalog_key("filter_lists.extra"))
            .unwrap()
            .layer
            .is_configuration(),
        scriptlets
    );
}

/// Covers: story:3-90, setting:session.source
#[test]
fn a_recipe_never_sets_session_source() {
    let layers = only(
        SettingsLayer::Recipe,
        "session.source",
        &json!({ "kind": "saved_identity", "name": "work" }),
    );

    let refused = refusal(&layers);

    assert_eq!(refused.code(), "setting_locked");
}

/// Covers: story:3-90, setting:session.source
#[test]
fn a_recipe_exposes_session_source_for_its_users_to_set() {
    let layers = Layers {
        recipe: Some(recipe(
            json!({}),
            &[("session.source", Exposure::default())],
        )),
        run: values(
            SettingsLayer::Run,
            json!({ "session": { "source": { "kind": "saved_identity", "name": "work" } } }),
        ),
        ..Layers::default()
    };

    let effective = resolved(&layers);

    assert_eq!(
        (
            effective.value(catalog_key("session.source")),
            effective
                .effective(catalog_key("session.source"))
                .unwrap()
                .locked_by
        ),
        (&json!({ "kind": "saved_identity", "name": "work" }), None)
    );
}

/// Covers: story:3-91, setting:delivery.write_ids_and_epochs, setting:delivery.report_guarantee_class, setting:webhook.signing, setting:secrets.by_reference
#[rstest]
fn an_honesty_guarantee_stays_locked_whatever_the_layer(
    #[values(
        "delivery.write_ids_and_epochs",
        "delivery.report_guarantee_class",
        "webhook.signing",
        "secrets.by_reference"
    )]
    key: &str,
    #[values(
        SettingsLayer::OperatorPolicy,
        SettingsLayer::UserConfiguration,
        SettingsLayer::ProjectConfiguration,
        SettingsLayer::Recipe,
        SettingsLayer::Run
    )]
    layer: SettingsLayer,
) {
    let refused = refusal(&only(layer, key, &json!(false)));

    assert_eq!(
        refused,
        SettingsError::Locked {
            key: key.to_owned(),
            layer,
            limit: SettingLimit {
                key: key.to_owned(),
                value: json!(true),
                layer: SettingsLayer::Default,
                origin: None,
            },
            fix: format!("`{key}` is an honesty guarantee no layer changes"),
        }
    );
}

/// Covers: story:3-91
#[test]
fn an_honesty_guarantee_says_the_default_locked_it() {
    let effective = resolved(&Layers::default());

    let signing = effective.effective(catalog_key("webhook.signing")).unwrap();
    assert_eq!(
        (&signing.value, signing.locked_by),
        (&json!(true), Some(SettingsLayer::Default))
    );
}

/// Covers: story:3-136, setting:browser_cache.max_unused_age
#[rstest]
#[case::the_operator_policy(SettingsLayer::OperatorPolicy, None)]
#[case::user_configuration(SettingsLayer::UserConfiguration, None)]
#[case::project_configuration(SettingsLayer::ProjectConfiguration, Some("setting_not_applicable"))]
#[case::the_recipe(SettingsLayer::Recipe, Some("setting_not_applicable"))]
#[case::the_run(SettingsLayer::Run, Some("setting_not_applicable"))]
fn browser_cache_max_unused_age_is_set_only_by_the_machines_own_layers(
    #[case] layer: SettingsLayer,
    #[case] refused: Option<&str>,
) {
    let layers = only(layer, "browser_cache.max_unused_age", &json!(86_400_000));

    let outcome = resolve(&layers, Product::Foxhound, Product::Foxhound);

    assert_eq!(outcome.err().map(|error| error.code()), refused);
}

/// Covers: setting:stealth.allowed_builds
#[rstest]
#[case::the_operator_policy(SettingsLayer::OperatorPolicy, None)]
#[case::user_configuration(SettingsLayer::UserConfiguration, Some("setting_not_applicable"))]
#[case::project_configuration(SettingsLayer::ProjectConfiguration, Some("setting_not_applicable"))]
#[case::the_recipe(SettingsLayer::Recipe, Some("setting_not_applicable"))]
#[case::the_run(SettingsLayer::Run, Some("setting_not_applicable"))]
fn stealth_allowed_builds_is_set_only_in_an_operator_policy(
    #[case] layer: SettingsLayer,
    #[case] refused: Option<&str>,
) {
    let layers = only(
        layer,
        "stealth.allowed_builds",
        &json!(["sha256:0123456789abcdef"]),
    );

    let outcome = resolve(&layers, Product::Foxhound, Product::Foxhound);

    assert_eq!(outcome.err().map(|error| error.code()), refused);
}

/// `timeout.connect`, which `connect()` and the CLI read from the project
/// file before they connect, is taken from the project file alone: the
/// machine's own layers, which only the engine reads once reached, a
/// Recipe and a `settings` option are refused naming the project file
/// (ADR 0083).
///
/// Covers: setting:timeout.connect
#[rstest]
#[case::the_operator_policy(SettingsLayer::OperatorPolicy, Some("setting_not_applicable"))]
#[case::user_configuration(SettingsLayer::UserConfiguration, Some("setting_not_applicable"))]
#[case::project_configuration(SettingsLayer::ProjectConfiguration, None)]
#[case::the_recipe(SettingsLayer::Recipe, Some("setting_not_applicable"))]
#[case::the_run(SettingsLayer::Run, Some("setting_not_applicable"))]
fn timeout_connect_is_set_only_in_the_project_file(
    #[case] layer: SettingsLayer,
    #[case] refused: Option<&str>,
) {
    let layers = only(layer, "timeout.connect", &json!(5_000));

    let outcome = resolve(&layers, Product::Collie, Product::Collie);

    let (code, fix) = match &outcome {
        Ok(_) => (None, None),
        Err(error @ SettingsError::NotApplicable { fix, .. }) => {
            (Some(error.code()), Some(fix.as_str()))
        }
        Err(error) => panic!("{error:?}"),
    };
    assert_eq!(code, refused);
    assert!(
        fix.is_none_or(|fix| fix.contains("the project file")),
        "{fix:?}"
    );
}

/// A Client's own bound is taken from `connect()`'s own `settings`, the
/// run layer, alone: configuration, an Operator policy and a Recipe never
/// reach the Client's transport, so each is refused naming `connect()`
/// (issue 2518). With nothing set, each keeps its default.
///
/// Covers: setting:connection.max_pending_calls, setting:connection.max_queued_write_bytes, setting:connection.max_listener_queue, error:setting_not_applicable
#[rstest]
#[case::the_operator_policy(SettingsLayer::OperatorPolicy, Some("setting_not_applicable"))]
#[case::user_configuration(SettingsLayer::UserConfiguration, Some("setting_not_applicable"))]
#[case::project_configuration(SettingsLayer::ProjectConfiguration, Some("setting_not_applicable"))]
#[case::the_recipe(SettingsLayer::Recipe, Some("setting_not_applicable"))]
#[case::the_connection(SettingsLayer::Run, None)]
fn a_clients_own_bound_is_set_only_in_connect(
    #[case] layer: SettingsLayer,
    #[case] refused: Option<&str>,
) {
    for (key, default) in [
        ("connection.max_pending_calls", 10_000),
        ("connection.max_queued_write_bytes", 67_108_864),
        ("connection.max_listener_queue", 1_024),
    ] {
        let set = resolve(
            &only(layer, key, &json!(5)),
            Product::Collie,
            Product::Collie,
        );
        let unset = resolved(&Layers::default());

        let (code, fix) = match &set {
            Ok(_) => (None, None),
            Err(error @ SettingsError::NotApplicable { fix, .. }) => {
                (Some(error.code()), Some(fix.as_str()))
            }
            Err(error) => panic!("{error:?}"),
        };
        let default_value = unset.effective(catalog_key(key)).unwrap();
        assert_eq!(
            (code, &default_value.value, default_value.layer),
            (refused, &json!(default), SettingsLayer::Default),
            "{key}"
        );
        assert!(fix.is_none_or(|fix| fix.contains("connect()")), "{fix:?}");
    }
}

/// `timeout.connect` is 30 s by default, which `config explain` shows set
/// by the default layer (ADR 0083).
///
/// Covers: setting:timeout.connect
#[test]
fn timeout_connect_is_thirty_seconds_by_default() {
    let effective = resolved(&Layers::default());

    let connect = effective.effective(catalog_key("timeout.connect")).unwrap();

    assert_eq!(
        (&connect.value, connect.layer),
        (&json!(30_000), SettingsLayer::Default)
    );
}

/// The layers with `set` for the user configuration and `run` for the Run.
fn user_and_run(set: Value, run: Value) -> Layers {
    Layers {
        user_configuration: values(SettingsLayer::UserConfiguration, set),
        run: values(SettingsLayer::Run, run),
        ..Layers::default()
    }
}

/// Covers: story:8-5, story:8-6, setting:stealth.human_input
#[rstest]
#[case::set_for_the_run_of_a_standard_session(
    user_and_run(json!({}), json!({ "stealth.human_input": true })),
    Some(json!({ "key": "browser.access_mode", "value": "standard", "layer": "default" }))
)]
#[case::set_by_the_user_for_an_identified_run(
    user_and_run(
        json!({ "stealth.human_input": true }),
        json!({ "browser.access_mode": "identified" }),
    ),
    Some(json!({ "key": "browser.access_mode", "value": "identified", "layer": "run" }))
)]
#[case::set_by_the_user_for_a_stealth_run(
    user_and_run(
        json!({ "stealth.human_input": true }),
        json!({ "browser.access_mode": "stealth" }),
    ),
    None
)]
#[case::beside_a_fingerprint_method_set_closer_than_the_mode(
    user_and_run(
        json!({ "browser.access_mode": "standard" }),
        json!({ "stealth.human_input": true, "stealth.fingerprint_method": "basic" }),
    ),
    None
)]
#[case::beside_a_fingerprint_method_set_farther_than_the_mode(
    user_and_run(
        json!({ "stealth.human_input": true, "stealth.fingerprint_method": "basic" }),
        json!({ "browser.access_mode": "identified" }),
    ),
    Some(json!({ "key": "browser.access_mode", "value": "identified", "layer": "run" }))
)]
#[case::left_to_its_default(user_and_run(json!({}), json!({})), None)]
fn an_inherited_stealth_item_yields_to_an_access_mode_that_is_not_stealth(
    #[case] layers: Layers,
    #[case] yields_to: Option<Value>,
) {
    let explained = resolved(&layers).explain(&["stealth.human_input".to_owned()], Vec::new());

    let setting = serde_json::to_value(&explained).unwrap()["settings"][0].clone();
    assert_eq!(setting.get("yields_to").cloned(), yields_to);
}

/// Covers: story:3-88, setting:crawl.handler.concurrency
#[test]
fn a_default_above_an_operator_policy_cap_takes_the_cap() {
    let layers = Layers {
        operator_policy: policy(
            json!({}),
            json!({}),
            json!({ "crawl": { "handler": { "concurrency": 8 } } }),
        ),
        ..Layers::default()
    };

    let effective = resolved(&layers);

    let concurrency = effective
        .effective(catalog_key("crawl.handler.concurrency"))
        .unwrap();
    assert_eq!(
        (&concurrency.value, concurrency.layer, &concurrency.cap),
        (&json!(8), SettingsLayer::OperatorPolicy, &Some(json!(8)))
    );
}

/// An unset clearable key is no bound, so an Operator policy cap bounds it.
///
/// Covers: story:3-51, setting:admission.max_requests_per_minute
#[test]
fn an_unset_rate_cap_under_an_operator_policy_cap_takes_the_cap() {
    let layers = Layers {
        operator_policy: policy(
            json!({}),
            json!({}),
            json!({ "admission": { "max_requests_per_minute": 120 } }),
        ),
        ..Layers::default()
    };

    let effective = resolved(&layers);

    let rate = effective
        .effective(catalog_key("admission.max_requests_per_minute"))
        .unwrap();
    assert_eq!(
        (&rate.value, rate.layer),
        (&json!(120), SettingsLayer::OperatorPolicy)
    );
}

/// Covers: story:3-92, setting:tls.accept_invalid_certs
#[test]
fn a_setting_that_weakens_a_guarantee_is_marked_on_the_results() {
    let effective = resolved(&only(
        SettingsLayer::Run,
        "tls.accept_invalid_certs",
        &json!({ "intranet.example": null }),
    ));

    assert_eq!(
        effective.weakened(),
        [WeakenedSetting {
            key: "tls.accept_invalid_certs".to_owned(),
            value: json!({ "intranet.example": null }),
            layer: SettingsLayer::Run,
            origin: None,
            preset: None,
        }]
    );
}

#[test]
fn an_unknown_key_names_the_closest_known_one() {
    let read = Values::read(
        SettingsLayer::Run,
        &nested(json!({ "timeout": { "navigation_": 1_000 } })),
        Product::Collie,
    );

    assert_eq!(
        read,
        Err(SettingsError::Unknown {
            key: "timeout.navigation_".to_owned(),
            layer: Some(SettingsLayer::Run),
            closest: Some("timeout.navigation".to_owned()),
        })
    );
}

#[rstest]
#[case::text_for_a_duration(json!({ "timeout": { "navigation": "60s" } }))]
#[case::null_for_a_key_that_cannot_be_cleared(json!({ "timeout": { "navigation": null } }))]
#[case::a_host_mapped_to_a_name(json!({ "network": { "resolve": { "a.example": ["b.example"] } } }))]
fn a_value_not_of_its_keys_kind_is_invalid(#[case] settings: Value) {
    let read = Values::read(SettingsLayer::Run, &nested(settings), Product::Collie);

    assert_eq!(read.map(|_| ()).unwrap_err().code(), "setting_invalid");
}

/// Covers: story:3-142
#[rstest]
#[case::user_configuration(SettingsLayer::UserConfiguration)]
#[case::project_configuration(SettingsLayer::ProjectConfiguration)]
fn a_configuration_file_of_a_collie_engine_keeps_foxhounds_keys_unapplied(
    #[case] layer: SettingsLayer,
) {
    let read = Values::read(
        layer,
        &nested(json!({ "crawl": { "retry": { "max": 5 } }, "timeout": { "navigation": 1_000 } })),
        Product::Collie,
    )
    .unwrap();
    let mut layers = Layers::default();
    match layer {
        SettingsLayer::UserConfiguration => layers.user_configuration = read,
        _ => layers.project_configuration = read,
    }

    let explained = resolve(&layers, Product::Collie, Product::Collie)
        .unwrap()
        .explain(&["crawl".to_owned()], Vec::new());

    assert_eq!(
        serde_json::to_value(&explained.settings).unwrap(),
        json!([{
            "key": "crawl.retry.max",
            "products": ["foxhound"],
            "applied": false,
            "value": 5,
            "layer": layer.as_str(),
            "weakens": false,
        }])
    );
}

/// Covers: story:3-142
#[rstest]
#[case::the_recipe(SettingsLayer::Recipe)]
#[case::the_run(SettingsLayer::Run)]
fn a_foxhound_key_given_explicitly_to_a_collie_engine_is_not_in_product(
    #[case] layer: SettingsLayer,
) {
    let read = Values::read(
        layer,
        &nested(json!({ "crawl": { "retry": { "max": 5 } } })),
        Product::Collie,
    );

    assert_eq!(
        read,
        Err(SettingsError::NotInProduct {
            key: "crawl.retry.max".to_owned(),
            needs: Product::Foxhound,
        })
    );
}

/// A `Retry-After`'s bound is a crawl's key (ADR 0118), which a Collie
/// engine refuses.
///
/// Covers: story:3-160, setting:politeness.retry_after.max, error:not_in_product
#[test]
fn the_bound_on_a_sites_retry_after_is_not_in_product_on_a_collie_engine() {
    let read = Values::read(
        SettingsLayer::Run,
        &nested(json!({ "politeness": { "retry_after": { "max": 60_000 } } })),
        Product::Collie,
    );

    assert_eq!(
        read,
        Err(SettingsError::NotInProduct {
            key: "politeness.retry_after.max".to_owned(),
            needs: Product::Foxhound,
        })
    );
}

/// A crawl's pacing keys (spec #582's Settings) are a crawl's, which a
/// Collie engine refuses.
///
/// Covers: error:not_in_product
#[rstest]
/// Covers: story:6-247, setting:politeness.domain.adaptive_delay
#[case::the_adaptive_delay(json!({ "adaptive_delay": true }), "politeness.domain.adaptive_delay")]
/// Covers: story:6-248, setting:politeness.domain.rate
#[case::the_rate(json!({ "rate": 30 }), "politeness.domain.rate")]
/// Covers: story:6-248, setting:politeness.domain.burst
#[case::the_burst(json!({ "burst": 2 }), "politeness.domain.burst")]
fn a_crawls_pacing_key_is_not_in_product_on_a_collie_engine(
    #[case] domain: Value,
    #[case] key: &str,
) {
    let read = Values::read(
        SettingsLayer::Run,
        &nested(json!({ "politeness": { "domain": domain } })),
        Product::Collie,
    );

    assert_eq!(
        read,
        Err(SettingsError::NotInProduct {
            key: key.to_owned(),
            needs: Product::Foxhound,
        })
    );
}

/// An Operator policy caps a crawl's per-domain rate and burst as its Run
/// opens, a run asking for none taking the cap and one asking for more
/// refused with `setting_locked` (spec #582's Settings; ADRs 0028 and 0138).
///
/// Covers: story:3-88
#[rstest]
/// Covers: story:6-248, setting:politeness.domain.rate
#[case::an_unset_rate(
    json!({ "politeness": { "domain": { "rate": 30 } } }),
    json!({}),
    "politeness.domain.rate",
    Ok(json!(30))
)]
/// Covers: story:6-248, setting:politeness.domain.rate
#[case::a_higher_rate(
    json!({ "politeness": { "domain": { "rate": 30 } } }),
    json!({ "politeness": { "domain": { "rate": 600 } } }),
    "politeness.domain.rate",
    Err("setting_locked")
)]
/// Covers: story:6-248, setting:politeness.domain.burst
#[case::a_larger_burst(
    json!({ "politeness": { "domain": { "burst": 1 } } }),
    json!({ "politeness": { "domain": { "burst": 10 } } }),
    "politeness.domain.burst",
    Err("setting_locked")
)]
fn an_operator_policy_caps_a_crawls_pacing(
    #[case] cap: Value,
    #[case] run: Value,
    #[case] key: &str,
    #[case] held_to: Result<Value, &str>,
) {
    let opened = resolve(
        &Layers {
            operator_policy: policy(json!({}), json!({}), cap),
            run: values(SettingsLayer::Run, run),
            ..Layers::default()
        },
        Product::Foxhound,
        Product::Foxhound,
    );

    assert_eq!(
        opened
            .map(|opened| opened.value(catalog_key(key)).clone())
            .map_err(|refused| refused.code()),
        held_to
    );
}

/// Covers: story:6-247, setting:politeness.domain.adaptive_delay
#[test]
fn an_operator_policy_locks_the_adaptive_delay_on() {
    let locked = json!({ "politeness": { "domain": { "adaptive_delay": true } } });
    let layers = Layers {
        operator_policy: policy(locked.clone(), locked, json!({})),
        run: values(
            SettingsLayer::Run,
            json!({ "politeness": { "domain": { "adaptive_delay": false } } }),
        ),
        ..Layers::default()
    };

    assert_eq!(refusal(&layers).code(), "setting_locked");
}

/// Covers: story:3-143
#[test]
fn a_crawls_key_on_a_collie_run_of_a_foxhound_engine_is_not_applicable() {
    let layers = only(SettingsLayer::Run, "crawl.retry.max", &json!(5));

    let refused = resolve(&layers, Product::Foxhound, Product::Collie).unwrap_err();

    assert_eq!(refused.code(), "setting_not_applicable");
}

/// Covers: story:3-143
#[test]
fn a_collie_engine_refuses_a_recipe_that_names_foxhound() {
    let manifest = RecipeManifest {
        name: "catalog-crawl".to_owned(),
        product: Product::Foxhound,
        settings: Map::new(),
        expose: std::collections::BTreeMap::new(),
    };

    let refused = Recipe::read(&manifest, Product::Collie);

    assert_eq!(
        refused,
        Err(SettingsError::NotInProduct {
            key: "catalog-crawl".to_owned(),
            needs: Product::Foxhound,
        })
    );
}

/// Covers: story:3-89
#[test]
fn an_explanation_lists_each_effective_value_with_its_layers() {
    let explained = resolved(&every_layer_sets_navigation()).explain(
        &["timeout.nav".to_owned(), "timeout.navigation".to_owned()],
        Vec::new(),
    );

    assert_eq!(
        serde_json::to_value(&explained).unwrap(),
        json!({
            "policy_version": "sha256:0123456789abcdef",
            "files": [],
            "settings": [{
                "key": "timeout.navigation",
                "products": ["collie", "foxhound"],
                "applied": true,
                "value": 5_000,
                "layer": "run",
                "origin": "settings",
                "weakens": false,
                "user_settable": true,
            }],
        })
    );
}

#[test]
fn a_configuration_file_reads_its_settings_nested_by_key() {
    let text = "version = \"0.1\"\n\n[settings.timeout]\nnavigation = 2000\n\n[settings.network.resolve]\n\"staging.example\" = [\"127.0.0.1\"]\n";

    let read = configuration(SettingsLayer::UserConfiguration, text, Product::Collie).unwrap();

    assert_eq!(
        (read.get("timeout.navigation"), read.get("network.resolve")),
        (
            Some(&json!(2000)),
            Some(&json!({ "staging.example": ["127.0.0.1"] }))
        )
    );
}

/// This crate's Product version, the engine's.
fn engine() -> ProductVersion {
    ProductVersion::parse(env!("CARGO_PKG_VERSION")).unwrap()
}

/// Covers: format:user-configuration
#[rstest]
#[case::no_version("[settings.timeout]\nnavigation = 2000\n")]
#[case::an_entry_outside_settings("version = \"0.1\"\ntimeout = 1\n")]
#[case::a_lowest_minor_that_is_no_minor("version = \"0.1\"\nlowest_minor = \"1\"\n")]
#[case::no_toml("version = ")]
fn a_settings_file_that_is_not_one_this_engine_reads_is_invalid(#[case] text: &str) {
    let read = configuration(SettingsLayer::UserConfiguration, text, Product::Collie);

    assert_eq!(read.map(|_| ()).unwrap_err().code(), "setting_invalid");
}

#[test]
fn an_operator_policy_cap_is_on_a_count_or_a_duration() {
    let text = "version = \"0.1\"\n[cap.egress]\noffline = true\n";

    let read = operator_policy(text, Product::Collie, &engine());

    assert_eq!(read.map(|_| ()).unwrap_err().code(), "setting_invalid");
}

#[test]
fn an_operator_policy_version_is_its_digest() {
    let read = operator_policy("version = \"0.1\"\n", Product::Collie, &engine()).unwrap();

    assert_eq!(read.version.as_deref(), Some("sha256:0227dc879fda140d"));
}

/// Covers: story:3-77
#[rstest]
#[case::unmanaged("version = \"0.1\"\n", None)]
#[case::managed(
    "version = \"0.1\"\n\n[managed]\ninstall_limits = [{ usage_kind = \"paid_solves\", currency = \"USD\", period = { every = \"month\", time_zone = \"Europe/Paris\" }, amount = 20000000 }]\n",
    Some(json!({ "install_limits": [{
        "usage_kind": "paid_solves",
        "currency": "USD",
        "period": { "every": "month", "time_zone": "Europe/Paris" },
        "amount": 20_000_000,
    }] })),
)]
#[case::managed_with_no_limit("version = \"0.1\"\n\n[managed]\n", Some(json!({})))]
fn a_managed_operator_policy_names_its_install_limits(
    #[case] text: &str,
    #[case] managed: Option<Value>,
) {
    let read = operator_policy(text, Product::Collie, &engine()).unwrap();

    assert_eq!(
        read.managed
            .map(|managed| serde_json::to_value(managed).unwrap()),
        managed
    );
}

#[test]
fn a_managed_limit_that_does_not_read_is_refused() {
    let refused = operator_policy(
        "version = \"0.1\"\n\n[managed]\ninstall_limits = [{ usage_kind = \"requests\" }]\n",
        Product::Collie,
        &engine(),
    );

    assert_eq!(
        refused.map(drop).map_err(|error| error.code()),
        Err("setting_invalid")
    );
}

/// Covers: story:3-142
#[test]
fn rewriting_a_shared_file_keeps_every_other_entry_byte_for_byte() {
    let text = "version = \"0.1\"\n# Foxhound's, kept by Collie.\n[settings.crawl.retry]\nmax = 5 # five\n\n[settings.timeout]\nnavigation = 2000 # two seconds\n";

    let rewritten = rewrite(text, "settings", "timeout.navigation", &json!(3000)).unwrap();

    assert_eq!(
        rewritten,
        "version = \"0.1\"\n# Foxhound's, kept by Collie.\n[settings.crawl.retry]\nmax = 5 # five\n\n[settings.timeout]\nnavigation = 3000 # two seconds\n"
    );
}

/// Every test home's user configuration (`TEST_SETTINGS` and the test's
/// `log.level`) is already in the order the writer keeps: a setting the
/// person surface or `config set` writes leaves every other byte as it was.
#[rstest]
#[case::as_created(None)]
#[case::with_a_log_level_set_since(Some("debug"))]
fn a_write_to_the_test_homes_user_configuration_keeps_every_other_byte(
    #[case] log_level: Option<&str>,
) {
    let env = goodfetch_test_support::TestEnv::new();
    if let Some(filter) = log_level {
        env.log_level(filter);
    }
    let files = SettingsFiles::of(&Home::new(env.home().to_owned()), &engine());
    #[expect(
        clippy::disallowed_methods,
        reason = "a synchronous test reads the file, outside any async worker"
    )]
    let text = std::fs::read_to_string(files.user_configuration()).unwrap();

    let rewritten = rewrite(&text, "settings", "timeout.navigation", &json!(45000)).unwrap();

    assert_eq!(
        rewritten,
        format!("{text}\n[settings.timeout]\nnavigation = 45000\n")
    );
}

/// Covers: story:3-133
#[rstest]
#[case::the_home("GOODFETCH_HOME")]
#[case::collies_engine_path("GOODFETCH_COLLIE_ENGINE_PATH")]
#[case::foxhounds_engine_path("GOODFETCH_FOXHOUND_ENGINE_PATH")]
#[case::the_browsers_path("GOODFETCH_BROWSERS_PATH")]
fn a_deployment_variable_of_either_product_is_on_the_closed_list(#[case] name: &str) {
    let environment = std::collections::BTreeMap::from([(name.into(), "/opt/goodfetch".into())]);

    assert_eq!(check_environment(&environment), Ok(()));
}

/// Covers: story:3-133, error:setting_unknown
#[test]
fn an_unknown_goodfetch_variable_names_the_closest_deployment_variable() {
    let environment = std::collections::BTreeMap::from([("GOODFETCH_HOMES".into(), "/opt".into())]);

    assert_eq!(
        check_environment(&environment),
        Err(SettingsError::Unknown {
            key: "GOODFETCH_HOMES".to_owned(),
            layer: None,
            closest: Some("GOODFETCH_HOME".to_owned()),
        })
    );
}

/// Covers: story:3-133, error:setting_invalid
#[test]
fn an_empty_deployment_variable_is_invalid() {
    let environment = std::collections::BTreeMap::from([("GOODFETCH_HOME".into(), "".into())]);

    assert_eq!(
        check_environment(&environment).unwrap_err().code(),
        "setting_invalid"
    );
}

#[test]
fn a_variable_outside_the_reserved_prefix_is_not_checked() {
    let environment = std::collections::BTreeMap::from([("GOODFETCHER".into(), "".into())]);

    assert_eq!(check_environment(&environment), Ok(()));
}

mod settings_file {
    goodfetch_test_support::fuzz_replay!(crate::settings::fuzz_settings_file, "fuzz/settings_file");
}

proptest! {
    /// A higher layer wins only where no lower layer locked the value
    /// (ADR 0028), for any layers that set, lock or leave a duration.
    /// Covers: story:3-86
    #[test]
    fn a_higher_layer_wins_only_where_no_lower_layer_locked(
        policy_value in proptest::option::of(0_u64..100_000),
        locked in any::<bool>(),
        user in proptest::option::of(0_u64..100_000),
        project in proptest::option::of(0_u64..100_000),
        run in proptest::option::of(0_u64..100_000),
    ) {
        let table = |value: Option<u64>| value.map_or_else(|| json!({}), |value| json!({ "timeout.run": value }));
        let layers = Layers {
            host: HostDefaults::default(),
            operator_policy: if locked {
                policy(json!({}), table(policy_value), json!({}))
            } else {
                policy(table(policy_value), json!({}), json!({}))
            },
            user_configuration: values(SettingsLayer::UserConfiguration, table(user)),
            project_configuration: values(SettingsLayer::ProjectConfiguration, table(project)),
            recipe: None,
            run: values(SettingsLayer::Run, table(run)),
        };
        let lock_holds = locked && policy_value.is_some();
        let above = [user, project, run].iter().any(Option::is_some);

        let outcome = resolve(&layers, Product::Collie, Product::Collie);

        if lock_holds && above {
            prop_assert_eq!(outcome.unwrap_err().code(), "setting_locked");
        } else {
            let expected = run.or(project).or(user).or(policy_value);
            let effective = outcome.unwrap();
            prop_assert_eq!(effective.value(catalog_key("timeout.run")).as_u64(), expected);
        }
    }
}

mod operator_policy;
mod presets;
mod scopes;
mod unbuilt;
mod versions;

/// Covers: setting:run_state.key_reference
#[rstest]
#[case::the_catalogs_where_the_host_gives_none(None, None, json!("keychain:goodfetch/run-state-key"), SettingsLayer::Default)]
#[case::the_hosts_in_its_place(Some("secretfile:run-state-key"), None, json!("secretfile:run-state-key"), SettingsLayer::Default)]
#[case::a_layers_over_the_hosts(Some("secretfile:run-state-key"), Some("env:SHOP_KEY"), json!("env:SHOP_KEY"), SettingsLayer::UserConfiguration)]
fn a_default_is_the_hosts_where_it_gives_one_and_still_the_default_layers(
    #[case] host: Option<&str>,
    #[case] user: Option<&str>,
    #[case] value: Value,
    #[case] layer: SettingsLayer,
) {
    let layers = Layers {
        host: host.map_or_else(HostDefaults::default, |host| {
            HostDefaults::default().with(&RUN_STATE_KEY_REFERENCE, json!(host))
        }),
        user_configuration: values(
            SettingsLayer::UserConfiguration,
            user.map_or_else(
                || json!({}),
                |user| json!({ "run_state": { "key_reference": user } }),
            ),
        ),
        ..Layers::default()
    };

    let resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();

    let effective = resolved.effective(&RUN_STATE_KEY_REFERENCE).unwrap();
    assert_eq!((&effective.value, effective.layer), (&value, layer));
}

/// A Recipe's Run is known as one, so the rules that hold a Recipe to less
/// than the program's own code, such as the `file:` rule, can read it.
/// Covers: story:4-6
#[rstest]
#[case::a_recipes_run(Some(recipe(json!({}), &[])), true)]
#[case::the_programs_own_run(None, false)]
fn the_resolved_settings_say_whether_the_run_is_a_recipes(
    #[case] recipe: Option<Recipe>,
    #[case] expected: bool,
) {
    let layers = Layers {
        recipe,
        ..Layers::default()
    };

    assert_eq!(resolved(&layers).recipe_run(), expected);
}

/// Story 4-16 (ADR 0017): `egress.no_private_endpoints` is the Operator
/// policy's alone, as Goodfetch Cloud sets it: from any other layer it is
/// refused with `setting_locked`, and the policy's own value is taken
/// (positive control).
/// Covers: story:4-16, error:setting_locked, setting:egress.no_private_endpoints
#[rstest]
#[case::from_a_recipe(SettingsLayer::Recipe, false)]
#[case::from_the_run(SettingsLayer::Run, false)]
#[case::from_user_configuration(SettingsLayer::UserConfiguration, false)]
#[case::from_project_configuration(SettingsLayer::ProjectConfiguration, false)]
#[case::from_the_operator_policy(SettingsLayer::OperatorPolicy, true)]
fn no_private_endpoints_is_taken_from_the_operator_policy_alone(
    #[case] layer: SettingsLayer,
    #[case] taken: bool,
) {
    let layers = only(layer, "egress.no_private_endpoints", &json!(true));

    let resolved = resolve(&layers, Product::Foxhound, Product::Foxhound);

    match resolved {
        Ok(effective) => assert_eq!(
            (
                taken,
                effective.value(catalog_key("egress.no_private_endpoints"))
            ),
            (true, &json!(true))
        ),
        Err(refused) => assert_eq!((taken, refused.code()), (false, "setting_locked")),
    }
}

/// Story 4-11 (spec #563): `egress.private_ranges` is set by the program's
/// own code, configuration and the Operator policy; a Client acting for a
/// model is refused with `setting_locked`, and so is a Recipe unless the
/// Operator policy's `egress.recipe_private_ranges` allows it.
/// Covers: story:4-11, error:setting_locked, setting:egress.private_ranges, setting:egress.recipe_private_ranges
#[rstest]
#[case::from_the_programs_own_code(SettingsLayer::Run, false, false, true)]
#[case::from_a_model(SettingsLayer::Run, true, false, false)]
#[case::from_a_model_even_where_recipes_may(SettingsLayer::Run, true, true, false)]
#[case::from_user_configuration(SettingsLayer::UserConfiguration, false, false, true)]
#[case::from_project_configuration(SettingsLayer::ProjectConfiguration, false, false, true)]
#[case::from_a_models_project_configuration(
    SettingsLayer::ProjectConfiguration,
    true,
    false,
    false
)]
#[case::from_the_operator_policy(SettingsLayer::OperatorPolicy, false, false, true)]
#[case::from_a_recipe(SettingsLayer::Recipe, false, false, false)]
#[case::from_a_recipe_the_operator_policy_allows(SettingsLayer::Recipe, false, true, true)]
fn private_ranges_are_never_a_models_and_a_recipes_only_where_the_operator_policy_allows(
    #[case] layer: SettingsLayer,
    #[case] for_model: bool,
    #[case] recipes_may: bool,
    #[case] taken: bool,
) {
    let ranges = json!(["127.0.0.0/8"]);
    let mut layers = only(layer, "egress.private_ranges", &ranges);
    if for_model {
        layers.run = layers.run.from_model();
        layers.project_configuration = layers.project_configuration.from_model();
    }
    if recipes_may {
        layers
            .operator_policy
            .settings
            .applied
            .insert("egress.recipe_private_ranges", json!(true));
    }

    let resolved = resolve(&layers, Product::Foxhound, Product::Foxhound);

    match resolved {
        Ok(effective) => assert_eq!(
            (taken, effective.value(catalog_key("egress.private_ranges"))),
            (true, &ranges)
        ),
        Err(refused) => assert_eq!(
            (
                taken,
                refused.code(),
                matches!(&refused, SettingsError::Locked { key, .. } if key == "egress.private_ranges")
            ),
            (false, "setting_locked", true),
            "{refused:?}"
        ),
    }
}

/// A run that lists private ranges is weakened: its Run report's
/// `weakened` names `egress.private_ranges` and the layer that set it,
/// and one that lists none is not (positive control).
/// Covers: story:4-11, setting:egress.private_ranges
#[test]
fn listing_private_ranges_weakens_the_run_and_listing_none_does_not() {
    let listed = resolved(&only(
        SettingsLayer::Run,
        "egress.private_ranges",
        &json!(["127.0.0.0/8"]),
    ));
    let none = resolved(&only(
        SettingsLayer::Run,
        "egress.private_ranges",
        &json!([]),
    ));

    let names = |resolved: &Resolved| -> Vec<(String, SettingsLayer)> {
        resolved
            .weakened()
            .into_iter()
            .map(|weakened| (weakened.key, weakened.layer))
            .filter(|(key, _)| key.starts_with("egress."))
            .collect()
    };
    assert_eq!(
        (names(&listed), names(&none)),
        (
            vec![("egress.private_ranges".to_owned(), SettingsLayer::Run)],
            vec![]
        )
    );
}

/// Each entry is a CIDR range, an address or a host name; any other value
/// is `setting_invalid` naming the key.
/// Covers: story:4-11, error:setting_invalid, setting:egress.private_ranges
#[rstest]
#[case::a_range_too_long(json!(["10.0.0.0/33"]))]
#[case::a_path(json!(["intranet.example/admin"]))]
#[case::a_url(json!(["http://intranet.example"]))]
#[case::a_wildcard(json!(["*.example"]))]
#[case::no_list(json!("127.0.0.0/8"))]
fn a_private_range_that_is_no_range_address_or_host_is_invalid(#[case] value: Value) {
    let refused = Values::read(
        SettingsLayer::Run,
        &nested(json!({ "egress": { "private_ranges": value } })),
        Product::Collie,
    )
    .unwrap_err();

    assert_eq!(
        (
            refused.code(),
            matches!(&refused, SettingsError::Invalid { key, .. } if key == "egress.private_ranges")
        ),
        ("setting_invalid", true),
        "{refused:?}"
    );
}

/// What `key` resolves to when user configuration sets it to `configured`
/// and `layer` gives `value`, from a Client acting for a model where
/// `for_model`: the value taken, or the refusal's code and key.
fn over_configuration(
    key: &str,
    configured: &Value,
    layer: SettingsLayer,
    for_model: bool,
    value: &Value,
) -> Result<Value, (&'static str, String)> {
    let mut layers = only(layer, key, value);
    if layer != SettingsLayer::UserConfiguration {
        layers.user_configuration =
            values(SettingsLayer::UserConfiguration, json!({ key: configured }));
    }
    if for_model {
        layers.run = layers.run.from_model();
        layers.project_configuration = layers.project_configuration.from_model();
    }
    over_layers(&layers, key)
}

/// Story 4-4 (spec #563, G5): a Client acting for a model and a Recipe
/// only narrow the domain policy the layers below them set: fewer hosts in
/// `egress.allowed_domains` (an entry the lower value lists, or a
/// subdomain of one, as the URL parser reads a host), more in
/// `egress.blocked_domains`. A widening value from them is
/// `setting_locked` naming the key, and the program's own code and
/// configuration still widen it (positive control).
/// Covers: story:4-4, error:setting_locked, setting:egress.allowed_domains, setting:egress.blocked_domains
#[rstest]
#[case::allowed_every_host_narrowed_by_a_model("egress.allowed_domains", json!(["*"]), SettingsLayer::Run, true, json!(["scope.test"]), Ok(json!(["scope.test"])))]
#[case::allowed_narrowed_to_a_subdomain_by_a_model("egress.allowed_domains", json!(["scope.test", "elsewhere.test"]), SettingsLayer::Run, true, json!(["www.scope.test"]), Ok(json!(["www.scope.test"])))]
#[case::allowed_narrowed_in_another_spelling_by_a_model("egress.allowed_domains", json!(["scope.test"]), SettingsLayer::Run, true, json!(["WWW.Scope.Test."]), Ok(json!(["WWW.Scope.Test."])))]
#[case::allowed_emptied_by_a_model("egress.allowed_domains", json!(["scope.test"]), SettingsLayer::Run, true, json!([]), Ok(json!([])))]
#[case::allowed_widened_by_a_model("egress.allowed_domains", json!(["scope.test"]), SettingsLayer::Run, true, json!(["scope.test", "elsewhere.test"]), locked("egress.allowed_domains"))]
#[case::allowed_widened_to_a_parent_by_a_model("egress.allowed_domains", json!(["www.scope.test"]), SettingsLayer::Run, true, json!(["scope.test"]), locked("egress.allowed_domains"))]
#[case::allowed_widened_to_every_host_by_a_model("egress.allowed_domains", json!(["scope.test"]), SettingsLayer::Run, true, json!(["*"]), locked("egress.allowed_domains"))]
#[case::allowed_widened_by_a_name_sharing_a_suffix("egress.allowed_domains", json!(["scope.test"]), SettingsLayer::Run, true, json!(["evilscope.test"]), locked("egress.allowed_domains"))]
#[case::allowed_widened_by_an_entry_that_is_no_host("egress.allowed_domains", json!(["scope.test"]), SettingsLayer::Run, true, json!(["scope.test/path"]), locked("egress.allowed_domains"))]
#[case::allowed_widened_by_a_models_project_configuration("egress.allowed_domains", json!(["scope.test"]), SettingsLayer::ProjectConfiguration, true, json!(["elsewhere.test"]), locked("egress.allowed_domains"))]
#[case::allowed_narrowed_by_a_recipe("egress.allowed_domains", json!(["*"]), SettingsLayer::Recipe, false, json!(["scope.test"]), Ok(json!(["scope.test"])))]
#[case::allowed_widened_by_a_recipe("egress.allowed_domains", json!(["scope.test"]), SettingsLayer::Recipe, false, json!(["elsewhere.test"]), locked("egress.allowed_domains"))]
#[case::allowed_widened_by_the_programs_own_code("egress.allowed_domains", json!(["scope.test"]), SettingsLayer::Run, false, json!(["scope.test", "elsewhere.test"]), Ok(json!(["scope.test", "elsewhere.test"])))]
#[case::allowed_widened_by_project_configuration("egress.allowed_domains", json!(["scope.test"]), SettingsLayer::ProjectConfiguration, false, json!(["*"]), Ok(json!(["*"])))]
#[case::blocked_added_to_by_a_model("egress.blocked_domains", json!(["ads.test"]), SettingsLayer::Run, true, json!(["ads.test", "tracker.test"]), Ok(json!(["ads.test", "tracker.test"])))]
#[case::blocked_widened_to_a_parent_by_a_model("egress.blocked_domains", json!(["www.ads.test"]), SettingsLayer::Run, true, json!(["ads.test"]), Ok(json!(["ads.test"])))]
#[case::blocked_entry_removed_by_a_model("egress.blocked_domains", json!(["ads.test", "tracker.test"]), SettingsLayer::Run, true, json!(["ads.test"]), locked("egress.blocked_domains"))]
#[case::blocked_entry_replaced_by_its_subdomain_by_a_model("egress.blocked_domains", json!(["ads.test"]), SettingsLayer::Run, true, json!(["www.ads.test"]), locked("egress.blocked_domains"))]
#[case::blocked_every_host_dropped_by_a_model("egress.blocked_domains", json!(["*"]), SettingsLayer::Run, true, json!(["ads.test"]), locked("egress.blocked_domains"))]
#[case::blocked_entry_removed_by_a_recipe("egress.blocked_domains", json!(["ads.test"]), SettingsLayer::Recipe, false, json!([]), locked("egress.blocked_domains"))]
#[case::blocked_added_to_by_a_recipe("egress.blocked_domains", json!([]), SettingsLayer::Recipe, false, json!(["ads.test"]), Ok(json!(["ads.test"])))]
#[case::blocked_entry_removed_by_the_programs_own_code("egress.blocked_domains", json!(["ads.test"]), SettingsLayer::Run, false, json!([]), Ok(json!([])))]
fn a_model_and_a_recipe_only_narrow_the_domain_policy(
    #[case] key: &str,
    #[case] configured: Value,
    #[case] layer: SettingsLayer,
    #[case] for_model: bool,
    #[case] value: Value,
    #[case] expected: Result<Value, (&'static str, String)>,
) {
    let resolved = over_configuration(key, &configured, layer, for_model, &value);

    assert_eq!(resolved, expected);
}

/// #2434 (owner, 2026-11-11): a Recipe, and a Client acting for a model
/// through its run layer or its project configuration, never set
/// `consent.handling: off`, the value that weakens the run, unless the
/// layers below already give it, which changes nothing; `answer` and
/// `detect` stay open to them, and the program's own code and
/// configuration set `off` (positive controls).
/// Covers: story:5-100, error:setting_locked, setting:consent.handling
#[rstest]
#[case::off_from_a_recipe(
    "answer",
    SettingsLayer::Recipe,
    false,
    "off",
    locked("consent.handling")
)]
#[case::off_from_a_model("answer", SettingsLayer::Run, true, "off", locked("consent.handling"))]
#[case::off_from_a_models_project_configuration(
    "answer",
    SettingsLayer::ProjectConfiguration,
    true,
    "off",
    locked("consent.handling")
)]
#[case::detect_from_a_recipe("answer", SettingsLayer::Recipe, false, "detect", Ok(json!("detect")))]
#[case::detect_from_a_model("answer", SettingsLayer::Run, true, "detect", Ok(json!("detect")))]
#[case::answer_from_a_model("detect", SettingsLayer::Run, true, "answer", Ok(json!("answer")))]
#[case::off_from_the_programs_own_code("answer", SettingsLayer::Run, false, "off", Ok(json!("off")))]
#[case::off_from_project_configuration("answer", SettingsLayer::ProjectConfiguration, false, "off", Ok(json!("off")))]
#[case::off_from_user_configuration("answer", SettingsLayer::UserConfiguration, false, "off", Ok(json!("off")))]
#[case::off_a_model_repeats("off", SettingsLayer::Run, true, "off", Ok(json!("off")))]
#[case::off_a_recipe_repeats("off", SettingsLayer::Recipe, false, "off", Ok(json!("off")))]
fn a_model_and_a_recipe_never_switch_consent_handling_off(
    #[case] configured: &str,
    #[case] layer: SettingsLayer,
    #[case] for_model: bool,
    #[case] value: &str,
    #[case] expected: Result<Value, (&'static str, String)>,
) {
    let resolved = over_configuration(
        "consent.handling",
        &json!(configured),
        layer,
        for_model,
        &json!(value),
    );

    assert_eq!(resolved, expected);
}

/// #2434: `consent.handling: off` from the program's own code, from
/// configuration and from an Operator policy resolves, and the Run report's
/// `weakened` names the key and the layer that set it; `detect` weakens
/// nothing (positive control).
/// Covers: story:5-100, setting:consent.handling
#[rstest]
#[case::the_programs_own_code(SettingsLayer::Run)]
#[case::user_configuration(SettingsLayer::UserConfiguration)]
#[case::project_configuration(SettingsLayer::ProjectConfiguration)]
#[case::an_operator_policy(SettingsLayer::OperatorPolicy)]
fn consent_handling_off_weakens_the_run_and_detect_does_not(#[case] layer: SettingsLayer) {
    let weakened = |value: &str| -> Vec<(String, SettingsLayer)> {
        resolved(&only(layer, "consent.handling", &json!(value)))
            .weakened()
            .into_iter()
            .map(|weakened| (weakened.key, weakened.layer))
            .filter(|(key, _)| key == "consent.handling")
            .collect()
    };

    assert_eq!(
        (weakened("off"), weakened("detect")),
        (vec![("consent.handling".to_owned(), layer)], vec![])
    );
}

/// Story 4-4: `egress.private_ranges`, the third key of the domain policy,
/// is never a model's even when it lists fewer ranges than configuration
/// does; the same narrower value from the program's own code is taken
/// (positive control).
/// Covers: story:4-4, error:setting_locked, setting:egress.private_ranges
#[rstest]
#[case::fewer_from_a_model(true, locked("egress.private_ranges"))]
#[case::fewer_from_the_programs_own_code(false, Ok(json!(["127.0.0.0/8"])))]
fn a_model_never_sets_the_private_ranges_even_to_fewer(
    #[case] for_model: bool,
    #[case] expected: Result<Value, (&'static str, String)>,
) {
    let resolved = over_configuration(
        "egress.private_ranges",
        &json!(["127.0.0.0/8", "10.0.0.0/8"]),
        SettingsLayer::Run,
        for_model,
        &json!(["127.0.0.0/8"]),
    );

    assert_eq!(resolved, expected);
}

/// Story 4-4 and #225: a Recipe's narrowed `egress.allowed_domains` stays
/// locked for a Client acting for a model, even to a narrower value,
/// unless the Recipe exposes it; exposed, the model narrows it further
/// and still never widens it.
/// Covers: story:4-4, story:3-87, error:setting_locked, setting:egress.allowed_domains
#[rstest]
#[case::narrower_where_unexposed(false, json!(["www.scope.test"]), locked("egress.allowed_domains"))]
#[case::narrower_where_exposed(true, json!(["www.scope.test"]), Ok(json!(["www.scope.test"])))]
#[case::wider_where_exposed(true, json!(["elsewhere.test"]), locked("egress.allowed_domains"))]
fn a_recipes_domain_policy_stays_locked_unless_exposed(
    #[case] exposed: bool,
    #[case] value: Value,
    #[case] expected: Result<Value, (&'static str, String)>,
) {
    let key = "egress.allowed_domains";
    let exposure = Exposure {
        minimum: None,
        maximum: None,
        one_of: Vec::new(),
    };
    let expose = if exposed {
        vec![(key, exposure)]
    } else {
        Vec::new()
    };
    let layers = Layers {
        recipe: Some(recipe(json!({ key: ["scope.test"] }), &expose)),
        run: values(SettingsLayer::Run, json!({ key: value })).from_model(),
        ..Layers::default()
    };

    let resolved = over_layers(&layers, key);

    assert_eq!(resolved, expected);
}

/// The refusal [`over_layers`] gives for a locked `key`.
fn locked(key: &str) -> Result<Value, (&'static str, String)> {
    Err(("setting_locked", key.to_owned()))
}

/// What `key` resolves to under `layers`: the value taken, or the
/// refusal's code and key.
fn over_layers(layers: &Layers, key: &str) -> Result<Value, (&'static str, String)> {
    resolve(layers, Product::Foxhound, Product::Foxhound)
        .map(|effective| effective.value(catalog_key(key)).clone())
        .map_err(|refused| {
            let SettingsError::Locked { key, .. } = &refused else {
                panic!("{refused:?}");
            };
            (refused.code(), key.clone())
        })
}

/// O11 of the deep review (accepted by the owner on 2026-10-28): a Client
/// acting for a model never sets `proxy.kind`, whose kinds' prices differ
/// by an order of magnitude, so a model never picks a dearer kind with the
/// person's money; a Recipe still sets it, as the spec has it, and the
/// program's own code and configuration do (positive controls).
/// Covers: story:8-93, error:setting_locked, setting:proxy.kind
#[rstest]
#[case::from_the_programs_own_code(SettingsLayer::Run, false, Ok(json!("mobile")))]
#[case::from_a_model(SettingsLayer::Run, true, locked("proxy.kind"))]
#[case::from_a_models_project_configuration(
    SettingsLayer::ProjectConfiguration,
    true,
    locked("proxy.kind")
)]
#[case::from_project_configuration(SettingsLayer::ProjectConfiguration, false, Ok(json!("mobile")))]
#[case::from_a_recipe(SettingsLayer::Recipe, false, Ok(json!("mobile")))]
#[case::from_user_configuration(SettingsLayer::UserConfiguration, false, Ok(json!("mobile")))]
fn a_model_never_sets_the_proxy_kind_a_recipe_and_the_program_do(
    #[case] layer: SettingsLayer,
    #[case] for_model: bool,
    #[case] expected: Result<Value, (&'static str, String)>,
) {
    let mut layers = only(layer, "proxy.kind", &json!("mobile"));
    if for_model {
        layers.run = layers.run.from_model();
        layers.project_configuration = layers.project_configuration.from_model();
    }

    let resolved = over_layers(&layers, "proxy.kind");

    assert_eq!(resolved, expected);
}

/// O12 of the deep review (accepted by the owner on 2026-10-28): a Proxy
/// provider's `price` comes only from user configuration and an Operator
/// policy, since a currency Limit on `proxy_bytes` trusts it and a
/// project's configuration is a file of its repository. A project's entry
/// with a `price` is `setting_locked`, its fix naming `price`; one without
/// is taken as before.
/// Covers: setting:proxy.providers, error:setting_locked
#[rstest]
#[case::priced_in_user_configuration(SettingsLayer::UserConfiguration, true, true)]
#[case::priced_in_the_operator_policy(SettingsLayer::OperatorPolicy, true, true)]
#[case::priced_in_project_configuration(SettingsLayer::ProjectConfiguration, true, false)]
#[case::unpriced_in_project_configuration(SettingsLayer::ProjectConfiguration, false, true)]
fn a_proxy_providers_price_comes_only_from_user_configuration_and_the_operator_policy(
    #[case] layer: SettingsLayer,
    #[case] priced: bool,
    #[case] taken: bool,
) {
    let mut entry =
        json!({ "servers": [{ "server": "http://203.0.113.7:3128", "kind": "datacenter" }] });
    if priced {
        entry["price"] = json!("0.01 USD");
    }
    let providers = json!({ "list": entry });
    let layers = only(layer, "proxy.providers", &providers);

    let resolved = resolve(&layers, Product::Foxhound, Product::Foxhound);

    match resolved {
        Ok(effective) => assert_eq!(
            (taken, effective.value(catalog_key("proxy.providers"))),
            (true, &providers)
        ),
        Err(refused) => assert_eq!(
            (
                taken,
                refused.code(),
                matches!(&refused, SettingsError::Locked { fix, .. } if fix.contains("`price`"))
            ),
            (false, "setting_locked", true),
            "{refused:?}"
        ),
    }
}

/// O8 of the deep review (accepted by the owner on 2026-10-28): a value
/// keeps who gave it, so a `session.source` from the project configuration
/// a Client acting for a model sent is the model's wherever the Run reads
/// it, whoever opens the Session; the same value from the program's own
/// project configuration, from user configuration and from the Operator
/// policy is not (positive controls).
/// Covers: story:4-15, setting:session.source
#[rstest]
#[case::a_models_project_configuration(SettingsLayer::ProjectConfiguration, true, true)]
#[case::a_models_run_layer(SettingsLayer::Run, true, true)]
#[case::the_programs_project_configuration(SettingsLayer::ProjectConfiguration, false, false)]
#[case::the_programs_run_layer(SettingsLayer::Run, false, false)]
#[case::user_configuration_in_a_models_run(SettingsLayer::UserConfiguration, true, false)]
#[case::the_operator_policy_in_a_models_run(SettingsLayer::OperatorPolicy, true, false)]
fn an_effective_value_names_whether_a_model_gave_it(
    #[case] layer: SettingsLayer,
    #[case] models_run: bool,
    #[case] for_model: bool,
) {
    let source = json!({ "kind": "cdp_url", "url": "ws://127.0.0.1:9222/devtools/browser/1" });
    let mut layers = only(layer, "session.source", &source);
    if models_run {
        layers.run = layers.run.from_model();
        layers.project_configuration = layers.project_configuration.from_model();
    }

    let resolved = resolve(&layers, Product::Collie, Product::Collie).unwrap();

    let effective = resolved.effective(catalog_key("session.source")).unwrap();
    assert_eq!(
        (effective.layer, &effective.value, effective.for_model),
        (layer, &source, for_model)
    );
}
