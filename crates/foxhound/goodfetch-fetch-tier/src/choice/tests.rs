//! Covers: story:5-115, story:5-118

use std::collections::BTreeMap;

use goodfetch_protocol::foxhound::crawl::LabelDeclaration;
use goodfetch_protocol::foxhound::fetch::FetchTier;
use serde_json::{Value, json};

use super::{Forced, LabelTiers, TierChoice, Unserved, unserved};

/// The Labels named in `labels`, each declared with its nested settings.
fn declared(labels: &[(&str, Value)]) -> BTreeMap<String, LabelDeclaration> {
    labels
        .iter()
        .map(|(name, settings)| {
            (
                (*name).to_owned(),
                LabelDeclaration {
                    settings: settings.as_object().cloned().unwrap_or_default(),
                },
            )
        })
        .collect()
}

fn forced(tier: FetchTier, label: Option<&str>) -> TierChoice {
    TierChoice {
        tier,
        forced: Some(Forced {
            label: label.map(str::to_owned),
        }),
    }
}

/// A Label's own `fetch.tier` forces its requests only, over the Run's;
/// under `auto` a declared Label starts on `http` and an undeclared one is
/// handed its request alone.
#[rstest::rstest]
#[case::label_forces_its_own(
    "list",
    Some(FetchTier::Browser),
    Some(forced(FetchTier::Http, Some("list")))
)]
#[case::another_label_takes_the_runs(
    "page",
    Some(FetchTier::Browser),
    Some(forced(FetchTier::Browser, None))
)]
#[case::run_forces_the_rest("page", None, Some(TierChoice { tier: FetchTier::Http, forced: None }))]
#[case::auto_label_under_a_forced_run("auto", Some(FetchTier::Browser), Some(TierChoice { tier: FetchTier::Http, forced: None }))]
#[case::undeclared_is_handed_alone("other", None, None)]
#[case::undeclared_under_a_forced_run(
    "other",
    Some(FetchTier::Http),
    Some(forced(FetchTier::Http, None))
)]
fn a_labels_tier_forces_its_own_requests_over_the_runs(
    #[case] label: &str,
    #[case] run_forced: Option<FetchTier>,
    #[case] expected: Option<TierChoice>,
) {
    let tiers = LabelTiers::of(&declared(&[
        ("list", json!({ "fetch": { "tier": "http" } })),
        ("page", json!({})),
        ("auto", json!({ "fetch": { "tier": "auto" } })),
    ]))
    .unwrap();

    assert_eq!(tiers.choose(label, run_forced), expected);
}

/// A Label key other than `fetch.tier` is refused with
/// `setting_not_applicable`, and a `fetch.tier` naming no tier with
/// `setting_invalid`, each naming the key.
#[rstest::rstest]
#[case::other_key(json!({ "timeout": { "handler": 5000 } }), "setting_not_applicable", "timeout.handler")]
#[case::unknown_tier(json!({ "fetch": { "tier": "fast" } }), "setting_invalid", "fetch.tier")]
#[case::not_text(json!({ "fetch": { "tier": 3 } }), "setting_invalid", "fetch.tier")]
fn a_label_setting_it_cannot_take_is_refused(
    #[case] settings: Value,
    #[case] code: &str,
    #[case] key: &str,
) {
    let error = LabelTiers::of(&declared(&[("list", settings)])).unwrap_err();

    assert_eq!(
        (
            error.code(),
            error.details().and_then(|details| details["key"].as_str())
        ),
        (code, Some(key))
    );
}

/// Under `auto` a call the tier cannot serve ends the attempt with
/// `attempt_ended` naming the escalation; under a forced tier it fails
/// with `browser_required` naming `fetch.tier`, the tier, the Label and
/// the call, and for a wait also the content the page may lack.
#[test]
fn an_unserved_call_escalates_under_auto_and_is_refused_under_a_forced_tier() {
    let auto = TierChoice {
        tier: FetchTier::Http,
        forced: None,
    };
    let by_label = forced(FetchTier::Http, Some("list"));

    let Unserved::Escalated(escalated) = unserved(&auto, "locator.click", false) else {
        panic!("an unserved call under `auto` escalates");
    };
    let Unserved::Refused(refused) = unserved(&by_label, "locator.wait_for", true) else {
        panic!("an unserved call under a forced tier is refused");
    };

    assert_eq!(
        (
            escalated.code(),
            escalated.details().cloned().map(Value::Object),
            refused.code(),
            refused.details().cloned().map(Value::Object),
        ),
        (
            "attempt_ended",
            Some(json!({ "reason": "escalated", "call": "locator.click", "tier": "http" })),
            "browser_required",
            Some(json!({
                "key": "fetch.tier",
                "tier": "http",
                "label": "list",
                "call": "locator.wait_for",
                "fix": "`fetch.tier: auto` or `fetch.tier: browser`, or the page may simply lack what it waits for",
            })),
        )
    );
}
