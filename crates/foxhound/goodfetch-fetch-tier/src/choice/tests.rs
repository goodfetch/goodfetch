//! Covers: story:5-115, story:5-118

use std::collections::BTreeMap;

use goodfetch_protocol::foxhound::crawl::LabelDeclaration;
use goodfetch_protocol::foxhound::fetch::FetchTier;
use serde_json::{Value, json};

use super::{Choosing, Forced, LabelTiers, SetTier, TierChoice, Unserved, unserved};

/// The Labels named in `labels`, each declared with its nested settings.
fn declared(labels: &[(&str, Value)]) -> BTreeMap<String, LabelDeclaration> {
    labels
        .iter()
        .map(|(name, settings)| {
            (
                (*name).to_owned(),
                LabelDeclaration {
                    payload: None,
                    settings: settings.as_object().cloned().unwrap_or_default(),
                },
            )
        })
        .collect()
}

fn forcing(tier: FetchTier, label: Option<&str>) -> Choosing {
    Choosing::Forced(forced(tier, label))
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
/// an `auto` set for a Label or the Run leaves the tier to the engine's
/// choice, while the built-in default hands the handler its request alone.
#[rstest::rstest]
#[case::label_forces_its_own(
    "list",
    SetTier::Forced(FetchTier::Browser),
    forcing(FetchTier::Http, Some("list"))
)]
#[case::another_label_takes_the_runs(
    "page",
    SetTier::Forced(FetchTier::Browser),
    forcing(FetchTier::Browser, None)
)]
#[case::an_auto_the_run_set("page", SetTier::Auto, Choosing::Engine)]
#[case::an_auto_the_label_set_under_a_forced_run(
    "auto",
    SetTier::Forced(FetchTier::Browser),
    Choosing::Engine
)]
#[case::an_auto_the_label_set_under_the_default("auto", SetTier::Unset, Choosing::Engine)]
#[case::the_default_is_handed_alone("page", SetTier::Unset, Choosing::Alone)]
#[case::undeclared_under_the_default("other", SetTier::Unset, Choosing::Alone)]
#[case::undeclared_under_a_forced_run(
    "other",
    SetTier::Forced(FetchTier::Http),
    forcing(FetchTier::Http, None)
)]
#[case::undeclared_under_an_auto_the_run_set("other", SetTier::Auto, Choosing::Engine)]
fn a_labels_tier_forces_its_own_requests_over_the_runs(
    #[case] label: &str,
    #[case] run: SetTier,
    #[case] expected: Choosing,
) {
    let tiers = LabelTiers::of(&declared(&[
        ("list", json!({ "fetch": { "tier": "http" } })),
        ("page", json!({})),
        (
            "auto",
            json!({ "fetch": { "tier": "auto", "encoding": "shift_jis" } }),
        ),
    ]))
    .unwrap();

    assert_eq!(tiers.choose(label, run), expected);
}

/// A Label key other than `fetch.tier` and `fetch.encoding` is refused
/// with `setting_not_applicable`, `fetch.tier_race` among them since it is
/// set for the Run only; a `fetch.tier` naming no tier, and a
/// `fetch.encoding` that is no WHATWG Encoding label, with
/// `setting_invalid`, each naming the key.
#[rstest::rstest]
#[case::other_key(json!({ "timeout": { "handler": 5000 } }), "setting_not_applicable", "timeout.handler")]
#[case::unknown_tier(json!({ "fetch": { "tier": "fast" } }), "setting_invalid", "fetch.tier")]
#[case::not_text(json!({ "fetch": { "tier": 3 } }), "setting_invalid", "fetch.tier")]
#[case::a_race(json!({ "fetch": { "tier_race": true } }), "setting_not_applicable", "fetch.tier_race")]
#[case::unknown_encoding(json!({ "fetch": { "encoding": "klingon" } }), "setting_invalid", "fetch.encoding")]
#[case::encoding_not_text(json!({ "fetch": { "encoding": 8 } }), "setting_invalid", "fetch.encoding")]
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

/// A Label's `fetch.encoding` is kept by its Label, its label as given.
#[test]
fn a_labels_encoding_is_kept_by_its_label() {
    let tiers = LabelTiers::of(&declared(&[
        ("list", json!({ "fetch": { "encoding": "Shift_JIS" } })),
        ("page", json!({ "fetch": { "encoding": null } })),
    ]))
    .unwrap();

    assert_eq!(
        tiers.encodings().collect::<Vec<_>>(),
        vec![("list", "Shift_JIS")]
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
