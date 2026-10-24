//! What a handler's `blocked` tells its domain's Politeness, read from the
//! details as the protocol's `BlockedDetails` writes them, so the detail the
//! crawl reads a site's `Retry-After` from cannot drift from the one the
//! engine's `blocked` writes it under again (fix-queue 44).
#![expect(
    clippy::disallowed_methods,
    reason = "a pure rule's test takes one monotonic instant to measure the hold from (ADR 0059)"
)]

use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::admission::Ended;
use goodfetch_protocol::page::{BLOCKED, BlockedDetails};
use serde_json::{Value, json};
use tokio::time::Instant;

use super::super::attempt::Answer;
use super::domain_ending;

/// Covers: story:3-159
#[test]
fn a_blocked_rate_limit_holds_its_domain_until_the_time_its_details_name() {
    // 2026-01-05T09:00:00Z, and the site's Retry-After a minute later.
    let wall = UNIX_EPOCH + Duration::from_hours(491_001);
    let details: BlockedDetails = serde_json::from_value(json!({
        "cause": "rate_limit",
        "url": "https://shop.test/1",
        "status": 429,
        "retry_after": "2026-01-05T09:01:00Z",
    }))
    .unwrap();
    let Value::Object(details) = serde_json::to_value(details).unwrap() else {
        panic!("details are a table");
    };
    let now = Instant::now();
    let answer = Answer::Failed {
        code: BLOCKED.name.to_owned(),
        retryable: true,
        message: String::new(),
        details,
    };

    assert_eq!(
        domain_ending(&answer, now, wall),
        Ended::TooManyRequests(Some(now + Duration::from_secs(60)))
    );
}
