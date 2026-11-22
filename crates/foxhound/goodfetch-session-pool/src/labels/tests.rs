//! Which pool each Label draws from (spec #579's `session_pools`, in
//! process): each case declares Labels as `crawl.start` does and reads the
//! pool of each and the source its Sessions open from.

use std::collections::BTreeMap;

use goodfetch_protocol::foxhound::crawl::LabelDeclaration;
use goodfetch_protocol::settings::SETTING_INVALID;
use rstest::rstest;
use serde_json::{Value, json};

use super::{LabelPools, PoolKey};

fn declared(settings: &Value) -> BTreeMap<String, LabelDeclaration> {
    BTreeMap::from([(
        "account".to_owned(),
        LabelDeclaration {
            payload: None,
            settings: settings.as_object().unwrap().clone(),
        },
    )])
}

/// Covers: story:5-136
#[rstest]
#[case::session_source(json!({ "session": { "source": null } }), true)]
#[case::http_credentials(json!({ "session": { "http_credentials": { "user": "a" } } }), true)]
#[case::a_secret(json!({ "secrets": { "api": { "ref": "env:API" } } }), true)]
#[case::a_login_step(json!({ "login": { "url": "https://shop.test/login" } }), true)]
#[case::a_page_check(json!({ "session": { "page_check": { "expected_page": "main" } } }), false)]
#[case::a_test_id_attribute(json!({ "locator": { "test_id_attribute": "data-qa" } }), false)]
#[case::a_fetch_tier(json!({ "fetch": { "tier": "browser" } }), false)]
#[case::nothing(json!({}), false)]
fn a_label_that_sets_what_a_login_rests_on_gets_a_pool_of_its_own(
    #[case] settings: Value,
    #[case] own: bool,
) {
    let pools = LabelPools::of(&declared(&settings)).unwrap();

    let pool = (pools.pool("account"), pools.pool("public"));

    let expected = if own {
        PoolKey::Label("account".to_owned())
    } else {
        PoolKey::Run
    };
    assert_eq!(pool, (expected, PoolKey::Run));
}

/// Covers: story:5-136
#[rstest]
#[case::fresh(json!(null), json!({ "own": true, "source": null }))]
#[case::a_cdp_url(
    json!({ "kind": "cdp_url", "url": "http://127.0.0.1:9222" }),
    json!({ "own": true, "source": { "kind": "cdp_url", "url": "http://127.0.0.1:9222" } })
)]
fn a_labels_pool_opens_its_sessions_from_its_own_session_source(
    #[case] source: Value,
    #[case] expected: Value,
) {
    let pools = LabelPools::of(&declared(&json!({ "session": { "source": source } }))).unwrap();

    let (own, runs) = (
        pools.source(&PoolKey::Label("account".to_owned())),
        pools.source(&PoolKey::Run),
    );

    let own = own.map(|source| json!({ "own": true, "source": source }));
    assert_eq!((own, runs.is_none()), (Some(expected), true));
}

/// Covers: story:5-136
#[test]
fn a_labels_session_source_that_is_no_source_is_invalid() {
    let refused = LabelPools::of(&declared(
        &json!({ "session": { "source": { "kind": "nowhere" } } }),
    ));

    assert_eq!(
        refused.map(drop).map_err(|error| error.code().to_owned()),
        Err(SETTING_INVALID.name.to_owned())
    );
}

/// A Label's own pool holds a login when its Sessions open from a
/// `session.source` or it declares a `login` of its own, which logs in
/// through the browser; a fresh source, HTTP credentials or a secret alone
/// hold none, and the Run's pool is the Run's settings' to say (#2418).
/// The Label's `settings` set it, so its origin is `settings` (ADR 0083).
///
/// Covers: setting:session.source
#[rstest]
#[case::a_saved_identity(
    json!({ "session": { "source": { "kind": "saved_identity", "name": "shop" } } }),
    Some(json!({ "key": "session.source", "value": { "kind": "saved_identity", "name": "shop" }, "layer": "run", "origin": "settings" }))
)]
#[case::a_login_step(
    json!({ "login": { "url": "https://shop.test/login" } }),
    Some(json!({ "key": "login", "value": { "url": "https://shop.test/login" }, "layer": "run", "origin": "settings" }))
)]
#[case::a_fresh_source(json!({ "session": { "source": null } }), None)]
#[case::http_credentials(json!({ "session": { "http_credentials": { "user": "a" } } }), None)]
#[case::a_secret(json!({ "secrets": { "api": { "ref": "env:API" } } }), None)]
fn a_labels_pool_holds_a_login_from_its_source_or_its_login_step(
    #[case] settings: Value,
    #[case] expected: Option<Value>,
) {
    let pools = LabelPools::of(&declared(&settings)).unwrap();

    let held = (
        pools.login(&PoolKey::Label("account".to_owned())),
        pools.login(&PoolKey::Run),
    );

    let own = held.0.map(|login| serde_json::to_value(login).unwrap());
    assert_eq!((own, held.1), (expected, None));
}
