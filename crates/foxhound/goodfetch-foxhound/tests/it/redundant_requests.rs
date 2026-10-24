//! A crawl's redundant requests on Foxhound's executable against the
//! Proving ground (spec #213's Seam 1; ADRs 0040, 0086 and 0094: hedged
//! requests are Foxhound's, with the HTTP fetch tiers): a crawl that
//! switches hedges on while a domain admits one request at a time is
//! refused, since the second copy could never start.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::TestEnv;
use serde_json::json;

use crate::crawl::Attached;

/// A crawl that switches hedges on under a `politeness.domain.concurrency`
/// of 1, ADR 0086's default, is refused before its Run begins,
/// naming the key and the layer that switched them on (ADR 0086).
///
/// Covers: story:3-63, setting:fetch.hedge.enabled, error:setting_not_applicable
#[test]
fn a_crawl_that_hedges_while_a_domain_admits_one_request_at_a_time_is_refused() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);

    let refused = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "hedged",
            "seeds": [{ "url": "http://pacing.test/", "label": "page" }],
            "settings": {
                "fetch": { "tier": "http", "hedge": { "enabled": true } },
                "politeness": { "domain": { "concurrency": 1 } },
            },
        }),
        &mut |_| None,
    );
    assert!(engine.finish());

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            &refused["error"]["data"]["key"],
            &refused["error"]["data"]["layer"],
        ),
        (
            &json!("setting_not_applicable"),
            &json!("fetch.hedge.enabled"),
            &json!("run"),
        ),
        "{refused}"
    );
}
