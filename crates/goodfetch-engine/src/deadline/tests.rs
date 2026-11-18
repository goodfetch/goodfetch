//! A call's deadline through its interface (spec #213's Seam 4), on the
//! paused `Clock`: a phase's timeout as its setting resolved, and the
//! `deadline_exceeded` a call past it ends with.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059)"
)]
#![expect(
    clippy::option_option,
    reason = "a call's own `timeout` has three states as the protocol carries them (ADR 0080)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::{Duration, UNIX_EPOCH};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::WaitedFor;
use goodfetch_protocol::settings::{
    RunOrigin, SettingLimit, SettingsLayer, TIMEOUT_ACTION, TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION,
};
use serde_json::{Value, json};

use super::Deadline;
use crate::clock::Clock;
use crate::settings::{Layers, Values, resolve};

fn settings(user_configuration: Value) -> crate::settings::Resolved {
    let Value::Object(set) = user_configuration else {
        panic!("settings are a table");
    };
    let layers = Layers {
        user_configuration: Values::read(SettingsLayer::UserConfiguration, &set, Product::Collie)
            .unwrap(),
        ..Layers::default()
    };
    resolve(&layers, Product::Collie, Product::Collie).unwrap()
}

/// Covers: story:3-52, story:11-43, error:deadline_exceeded, setting:timeout.first_byte
#[test]
fn a_wait_past_its_first_byte_timeout_ends_with_deadline_exceeded_naming_the_setting() {
    let settings = settings(json!({ "timeout": { "first_byte": 5_000 } }));
    let deadline = Deadline::of(&settings, &TIMEOUT_FIRST_BYTE).unwrap();

    let ended = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        deadline
            .within(&clock, WaitedFor::FIRST_BYTE, std::future::pending::<()>())
            .await
    });

    let error = ended.unwrap_err();
    let (class, retryable, details) = error.wire_parts();
    assert_eq!(
        (error.code(), class, retryable, Value::Object(details)),
        (
            "deadline_exceeded",
            -32005,
            true,
            json!({
                "limit": { "key": "timeout.first_byte", "value": 5_000, "layer": "user_configuration" },
                "elapsed": 5_000,
                "waited_for": "first_byte",
            })
        )
    );
}

/// Covers: story:3-52, setting:timeout.navigation
#[test]
fn a_navigation_answered_within_its_default_thirty_seconds_ends_well() {
    let settings = settings(json!({}));
    let deadline = Deadline::of(&settings, &TIMEOUT_NAVIGATION).unwrap();

    let answered = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        deadline
            .within(&clock, WaitedFor::NAVIGATION, async {
                tokio::time::sleep(Duration::from_secs(29)).await;
                "loaded"
            })
            .await
    });

    assert_eq!(
        (deadline.duration(), answered.ok()),
        (Duration::from_secs(30), Some("loaded"))
    );
}

/// A Run with no settings waits under the timeout's declared default, which
/// the catalog gives, set by the default layer.
/// Covers: setting:timeout.action
#[test]
fn a_timeout_no_layer_set_takes_its_declared_default() {
    let deadline = Deadline::declared(&TIMEOUT_ACTION);

    assert_eq!(
        (deadline.duration(), deadline.limit().clone()),
        (
            Duration::from_secs(30),
            SettingLimit {
                key: "timeout.action".to_owned(),
                value: json!(30_000),
                layer: SettingsLayer::Default,
                origin: None,
            }
        )
    );
}

/// A call's own `timeout` (ADR 0080): absent, the scope's deadline; `null`,
/// none; a count of milliseconds, that setting at the run layer.
/// Covers: story:11-40, setting:timeout.navigation
#[rstest::rstest]
#[case::absent(None, Some((60_000, SettingsLayer::UserConfiguration, None)))]
#[case::null(Some(None), None)]
#[case::given(
    Some(Some(500)),
    Some((500, SettingsLayer::Run, Some(RunOrigin::Argument)))
)]
fn a_calls_own_timeout_replaces_its_scopes_deadline(
    #[case] timeout: Option<Option<u64>>,
    #[case] expected: Option<(u64, SettingsLayer, Option<RunOrigin>)>,
) {
    let settings = settings(json!({ "timeout": { "navigation": 60_000 } }));
    let scope = Deadline::of(&settings, &TIMEOUT_NAVIGATION).unwrap();

    let deadline = Deadline::of_call(&scope, &TIMEOUT_NAVIGATION, timeout).unwrap();

    assert_eq!(
        deadline.map(|deadline| deadline.limit().clone()),
        expected.map(|(value, layer, origin)| SettingLimit {
            key: "timeout.navigation".to_owned(),
            value: json!(value),
            layer,
            origin,
        })
    );
}

/// A `timeout` of 0 would end the call at once: it is refused as the
/// setting it would have set, at the run layer from the call's argument,
/// and the fix names the null form that sets none (ADR 0080).
/// Covers: story:11-42, error:setting_invalid, setting:timeout.navigation
#[test]
fn a_calls_own_timeout_of_zero_is_setting_invalid_naming_the_null_form() {
    let scope = Deadline::declared(&TIMEOUT_NAVIGATION);

    let refused = Deadline::of_call(&scope, &TIMEOUT_NAVIGATION, Some(Some(0))).unwrap_err();

    let (class, retryable, details) = refused.wire_parts();
    assert_eq!(
        (refused.code(), class, retryable, Value::Object(details)),
        (
            "setting_invalid",
            -32001,
            false,
            json!({
                "key": "timeout.navigation",
                "layer": "run",
                "reason": "a timeout of 0 ms would end the call at once",
                "limit": { "key": "timeout.navigation", "value": 0, "layer": "run", "origin": "argument" },
                "fix": "give at least 1 ms, or `timeout: null` for no timeout",
            })
        )
    );
}
