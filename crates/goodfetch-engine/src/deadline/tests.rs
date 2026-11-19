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
use goodfetch_protocol::connection::{EnclosingCall, WaitedFor};
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::settings::{
    RunOrigin, SettingLimit, SettingsLayer, TIMEOUT_ACTION, TIMEOUT_FIRST_BYTE, TIMEOUT_HOOK,
    TIMEOUT_NAVIGATION,
};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::time::Instant;

use super::{Deadline, Enclosing};
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

/// The enclosing call of the nesting tests: a call of `hook.handle` under a
/// `timeout.hook` of 10 s, whose end `ends_at` holds.
fn enclosing(ends_at: watch::Receiver<Instant>) -> Enclosing {
    Enclosing::new(
        &Deadline::given(&TIMEOUT_HOOK, 10_000, SettingsLayer::UserConfiguration),
        EnclosingCall {
            operation: "hook.handle".to_owned(),
            call_id: RequestId::Number(7),
        },
        move || *ends_at.borrow(),
    )
}

/// An action started with 10 s left in its enclosing call gets 10 s, not
/// its own 30 s (ADR 0080): it ends at the enclosing call's end with
/// `deadline_exceeded`, whose `limit` is the enclosing timeout, `ended_by`
/// names that call and `observed` the last state seen.
/// Covers: story:11-41, story:11-43, error:deadline_exceeded
#[test]
fn an_inner_call_ends_at_its_enclosing_calls_end_which_it_names() {
    let ended = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let (_ends_at, ending) = watch::channel(clock.now() + Duration::from_secs(10));
        let enclosing = enclosing(ending);
        let action = Deadline::declared(&TIMEOUT_ACTION);
        enclosing
            .bound(
                &clock,
                action.within(
                    &clock,
                    WaitedFor::BROWSER_ANSWER,
                    std::future::pending::<()>(),
                ),
                || Some("a navigation under way on page `p-1`".to_owned()),
            )
            .await
    });

    let error = ended.unwrap_err();
    let (_, _, details) = error.wire_parts();
    assert_eq!(
        (error.code(), Value::Object(details)),
        (
            "deadline_exceeded",
            json!({
                "limit": { "key": "timeout.hook", "value": 10_000, "layer": "user_configuration" },
                "elapsed": 10_000,
                "waited_for": "answer",
                "ended_by": { "operation": "hook.handle", "call_id": 7 },
                "observed": "a navigation under way on page `p-1`",
            })
        )
    );
}

/// A child may end sooner than its parent (ADR 0080): an inner timeout
/// shorter than what is left of the enclosing one ends the call with its
/// own `deadline_exceeded`, which names no enclosing call.
/// Covers: story:11-41
#[test]
fn an_inner_call_whose_own_timeout_comes_first_ends_with_its_own() {
    let ended = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let (_ends_at, ending) = watch::channel(clock.now() + Duration::from_secs(10));
        let enclosing = enclosing(ending);
        let action = Deadline::given(&TIMEOUT_ACTION, 5_000, SettingsLayer::UserConfiguration);
        enclosing
            .bound(
                &clock,
                action.within(
                    &clock,
                    WaitedFor::BROWSER_ANSWER,
                    std::future::pending::<()>(),
                ),
                || None,
            )
            .await
    });

    let error = ended.unwrap_err();
    let (_, _, details) = error.wire_parts();
    assert_eq!(
        Value::Object(details),
        json!({
            "limit": { "key": "timeout.action", "value": 5_000, "layer": "user_configuration" },
            "elapsed": 5_000,
            "waited_for": "browser_answer",
        })
    );
}

/// A hold of the enclosing call's clock, such as a Human pause or a
/// handler's wait for a navigation-gate permit, moves its end later while
/// it lasts (ADRs 0080, 0092 and 0093): an inner call answered past the
/// end first read is not ended.
/// Covers: story:11-41
#[test]
fn an_inner_call_outlives_an_enclosing_end_a_hold_moved_later() {
    let answered = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let started = clock.now();
        let (ends_at, ending) = watch::channel(started + Duration::from_secs(10));
        let enclosing = enclosing(ending);
        let hold = async {
            clock.sleep_until(started + Duration::from_secs(8)).await;
            ends_at.send_replace(started + Duration::from_secs(20));
            std::future::pending::<()>().await;
        };
        let work = async {
            clock.sleep_until(started + Duration::from_secs(15)).await;
            Ok("clicked")
        };
        tokio::select! {
            answered = enclosing.bound(&clock, work, || None) => answered,
            () = hold => unreachable!("the hold never ends"),
        }
    });

    assert_eq!(answered.ok(), Some("clicked"));
}
