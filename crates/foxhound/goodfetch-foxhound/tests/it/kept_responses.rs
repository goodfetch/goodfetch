//! Kept responses conformance cases, the refusal at the Run's start (ADR
//! 0110), on Foxhound's executable against the Proving ground: a Label
//! that keeps responses for its handler (`fetch.kept_responses.urls`)
//! while its requests are forced onto `http` or `browser_fetch`, whose
//! pages run no script, fails the crawl's start with
//! `setting_not_applicable` naming the key, before the Run begins, so no
//! handler runs and the site receives nothing. The tier rule and
//! `discovered_api`'s answers are #936's.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use rstest::rstest;
use serde_json::{Value, json};

use crate::url_fragments::refused_start;

/// The responses a Label keeps: the page's JSON calls.
fn keeping(tier: Option<&str>) -> Value {
    let mut fetch = json!({ "kept_responses": { "urls": ["**/api/**"] } });
    if let Some(tier) = tier {
        fetch["tier"] = json!(tier);
    }
    json!({ "fetch": fetch })
}

/// Covers: story:6-262, setting:fetch.kept_responses.urls, error:setting_not_applicable
#[rstest]
#[case::on_its_own_forced_http(json!({}), keeping(Some("http")))]
#[case::on_its_own_forced_browser_fetch(json!({}), keeping(Some("browser_fetch")))]
#[case::on_the_runs_forced_http(json!({ "fetch": { "tier": "http" } }), keeping(None))]
fn a_label_keeping_responses_on_a_tier_that_runs_no_script_fails_the_start(
    #[case] run: Value,
    #[case] label: Value,
) {
    let refusal = refused_start(&run, &label);

    assert_eq!(
        refusal,
        (
            json!(["setting_not_applicable", "fetch.kept_responses.urls"]),
            Vec::<String>::new()
        )
    );
}
