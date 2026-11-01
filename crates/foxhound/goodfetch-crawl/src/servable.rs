//! The refusal at a crawl's start of a setting that can never serve (ADRs
//! 0110 and 0120; spec #582): a scope, the Run or one of its Labels, that
//! keeps responses for its handler (`fetch.kept_responses.urls`) while its
//! requests are forced onto `http` or `browser_fetch`, whose pages run no
//! script, or that keeps its URLs' fragments (`crawl.keep_url_fragment`)
//! while they are forced onto a document tier, where every fragment would
//! fetch the same document again, fails the start with
//! `setting_not_applicable`, before the Run begins and ahead of the tier's
//! own refusal. Responses kept where a tier could serve them are refused
//! with `capability_unavailable` until the engine keeps them at its
//! interception (area 5).

use goodfetch_engine::CallError;
use goodfetch_engine::settings::Resolved;
use goodfetch_fetch_tier::{FetchTiers, LabelSettings, SetTier};
use goodfetch_protocol::foxhound::fetch::{FETCH_KEPT_RESPONSES_URLS, FETCH_TIER, FetchTier};
use goodfetch_protocol::foxhound::frontier::CRAWL_KEEP_URL_FRAGMENT;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SETTING_NOT_APPLICABLE,
    SettingNotApplicableDetails, SettingsLayer,
};
use serde_json::Value;

/// What one scope's requests are given: the tier forced on them, if any,
/// and whether they keep fragments and responses.
struct Served<'a> {
    /// The Label, or `None` for the Run's own requests.
    label: Option<&'a str>,
    /// The layer that set what is refused.
    layer: SettingsLayer,
    tier: SetTier,
    keeps_fragment: bool,
    keeps_responses: bool,
}

/// Refuses the crawl whose Run resolved `settings` and whose Labels set
/// `labels` when a scope's settings can never be served.
///
/// # Errors
///
/// `setting_not_applicable` naming `fetch.kept_responses.urls` or
/// `crawl.keep_url_fragment`, then `capability_unavailable` naming
/// `fetch.kept_responses.urls`.
pub(crate) fn check(settings: &Resolved, labels: &LabelSettings) -> Result<(), CallError> {
    let run_tier = FetchTiers::run_tier(settings);
    let set = |key: &str, keeps: fn(&Value) -> bool| {
        settings
            .get(key)
            .filter(|effective| keeps(&effective.value))
            .map(|effective| effective.layer)
    };
    let run_fragment = set(CRAWL_KEEP_URL_FRAGMENT.name, |value| {
        *value == Value::Bool(true)
    });
    let run_responses = set(FETCH_KEPT_RESPONSES_URLS.name, |value| {
        value
            .as_array()
            .is_some_and(|patterns| !patterns.is_empty())
    });
    let run = Served {
        label: None,
        layer: run_fragment.or(run_responses).unwrap_or(SettingsLayer::Run),
        tier: run_tier,
        keeps_fragment: run_fragment.is_some(),
        keeps_responses: run_responses.is_some(),
    };
    let (run_keeps_fragment, run_keeps_responses) = (run.keeps_fragment, run.keeps_responses);
    let by_label = labels.tiers().map(move |(label, tier)| Served {
        label: Some(label),
        layer: SettingsLayer::Run,
        tier: match tier {
            SetTier::Unset => run_tier,
            own => own,
        },
        keeps_fragment: labels.keeps_fragment(label).unwrap_or(run_keeps_fragment),
        keeps_responses: labels.keeps_responses(label).unwrap_or(run_keeps_responses),
    });
    let scopes: Vec<Served<'_>> = std::iter::once(run).chain(by_label).collect();
    for scope in &scopes {
        let SetTier::Forced(tier) = scope.tier else {
            continue;
        };
        if scope.keeps_responses && matches!(tier, FetchTier::Http | FetchTier::BrowserFetch) {
            return Err(never_served(
                scope,
                tier,
                FETCH_KEPT_RESPONSES_URLS.name,
                "keeps responses for its handler, which only a tier that runs the page's scripts, or `discovered_api`, can keep",
            ));
        }
        if scope.keeps_fragment
            && matches!(
                tier,
                FetchTier::Http | FetchTier::BrowserFetch | FetchTier::DiscoveredApi
            )
        {
            return Err(never_served(
                scope,
                tier,
                CRAWL_KEEP_URL_FRAGMENT.name,
                "keeps its URLs' fragments, whose views only a tier that runs the page's scripts serves: on a document tier every fragment would fetch the same document again",
            ));
        }
    }
    if let Some(scope) = scopes.iter().find(|scope| scope.keeps_responses) {
        return Err(CallError::new(
            &CAPABILITY_UNAVAILABLE,
            format!(
                "This engine does not keep a crawl request's responses for its handler yet, which `{}` asks for in {}: leave it empty.",
                FETCH_KEPT_RESPONSES_URLS.name,
                named(scope)
            ),
            &CapabilityUnavailableDetails {
                key: FETCH_KEPT_RESPONSES_URLS.name.to_owned(),
                layer: scope.layer,
                kind: "kept_responses".to_owned(),
                fix: format!("`{}: []`", FETCH_KEPT_RESPONSES_URLS.name),
            },
        ));
    }
    Ok(())
}

/// `setting_not_applicable` for `key`, which `scope` sets and which `why`,
/// on the `tier` forced on its requests.
fn never_served(scope: &Served<'_>, tier: FetchTier, key: &str, why: &str) -> CallError {
    let tier = tier.as_str();
    CallError::new(
        &SETTING_NOT_APPLICABLE,
        format!(
            "{} sets `{key}`, but its requests are forced onto the `{tier}` tier with `{}`, so it could never be served: it {why}. Set `{}` to `auto` or `browser` for it, or leave `{key}` unset.",
            capitalized(&named(scope)),
            FETCH_TIER.name,
            FETCH_TIER.name,
        ),
        &SettingNotApplicableDetails {
            key: key.to_owned(),
            layer: scope.layer,
            fix: format!(
                "`{}: auto` or `{}: browser`",
                FETCH_TIER.name, FETCH_TIER.name
            ),
        },
    )
}

/// How a message names `scope`.
fn named(scope: &Served<'_>) -> String {
    scope.label.map_or_else(
        || "the Run".to_owned(),
        |label| format!("the Label `{label}`"),
    )
}

/// `text` with its first letter capitalized.
fn capitalized(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}
