//! A call's deadline through its interface (spec #213's Seam 4), on the
//! paused `Clock`: a phase's timeout as its setting resolved, and the
//! `deadline_exceeded` a call past it ends with.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::{Duration, UNIX_EPOCH};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
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

/// Covers: story:3-52, error:deadline_exceeded, setting:timeout.first_byte
#[test]
fn a_wait_past_its_first_byte_timeout_ends_with_deadline_exceeded_naming_the_setting() {
    let settings = settings(json!({ "timeout": { "first_byte": 5_000 } }));
    let deadline = Deadline::of(&settings, &TIMEOUT_FIRST_BYTE).unwrap();

    let ended = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        deadline
            .within(&clock, "first_byte", std::future::pending::<()>())
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
            .within(&clock, "navigation", async {
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
