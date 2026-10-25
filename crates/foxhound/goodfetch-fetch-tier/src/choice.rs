//! The tier each crawl request takes, and what a call its tier cannot
//! serve exactly leads to (spec #579's `tiers`; ADRs 0020, 0023 and 0093).
//!
//! `fetch.tier` forces a tier for the Run, or for one Label, whose own
//! value wins for its requests. Under an `auto` set for its Label or its
//! Run the engine chooses: the crawl learns the tier per Label on a domain
//! (`goodfetch-crawl`'s `tier_choice`), while a request left at the
//! built-in default is handed to its handler alone, as before any tier was
//! built, until #1602. A Label also takes `fetch.encoding` and
//! `session.page_check`. A call the request's tier cannot serve exactly
//! escalates the request to the browser under `auto`, and fails with
//! `browser_required` under a forced tier, which nothing escalates.

use std::collections::BTreeMap;

use encoding_rs::Encoding;
use goodfetch_engine::CallError;
use goodfetch_protocol::foxhound::crawl::{ATTEMPT_ENDED, LabelDeclaration};
use goodfetch_protocol::foxhound::fetch::{
    BROWSER_REQUIRED, BrowserRequiredDetails, FETCH_ENCODING, FETCH_TIER, FETCH_TIER_RACE,
    FETCH_TIERS, FetchTier,
};
use goodfetch_protocol::settings::page_check::PageCheck;
use goodfetch_protocol::settings::{
    SESSION_PAGE_CHECK, SETTING_INVALID, SETTING_NOT_APPLICABLE, SettingInvalidDetails,
    SettingNotApplicableDetails, SettingsLayer,
};
use serde_json::{Map, Value, json};

/// The Labels a crawl declares, each with what it sets of its own: its
/// `fetch.tier`, the `fetch.encoding` and the `session.page_check` it sets,
/// if any (ADR 0093).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabelSettings {
    tiers: BTreeMap<String, SetTier>,
    encodings: BTreeMap<String, String>,
    page_checks: BTreeMap<String, PageCheck>,
}

/// How a request's tier is chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choosing {
    /// It takes no tier: its handler is given its request alone.
    Alone,
    /// `auto` is set: the engine chooses.
    Engine,
    /// `fetch.tier` forces this tier.
    Forced(TierChoice),
}

/// The Run's `fetch.tier`, or a declared Label's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetTier {
    /// None is set: for a Label, the Run's applies; for the Run, the
    /// built-in `auto`, which hands each handler its request alone until
    /// the engine's tier choice is built.
    Unset,
    /// `auto`, set.
    Auto,
    /// A tier it forces.
    Forced(FetchTier),
}

impl LabelSettings {
    /// The Labels `labels` declares, read from their settings.
    ///
    /// # Errors
    ///
    /// `setting_not_applicable` for a key a Label does not take here (every
    /// key but `fetch.tier`, `fetch.encoding` and `session.page_check`;
    /// `fetch.tier_race` is the Run's only), and `setting_invalid` for a
    /// `fetch.tier` that names no tier, a `fetch.encoding` that is no WHATWG
    /// Encoding label or a `session.page_check` that is no Page check.
    pub fn of(labels: &BTreeMap<String, LabelDeclaration>) -> Result<Self, CallError> {
        let mut tiers = BTreeMap::new();
        let mut encodings = BTreeMap::new();
        let mut page_checks = BTreeMap::new();
        for (name, declaration) in labels {
            let mut keys = Vec::new();
            flatten("", &declaration.settings, &mut keys);
            let mut tier = SetTier::Unset;
            for (key, value) in keys {
                if key == FETCH_ENCODING.name {
                    if let Some(encoding) = encoding(name, &value)? {
                        encodings.insert(name.clone(), encoding);
                    }
                    continue;
                }
                if key == SESSION_PAGE_CHECK.name {
                    if let Some(check) = page_check_of(name, &value)? {
                        page_checks.insert(name.clone(), check);
                    }
                    continue;
                }
                if key != FETCH_TIER.name {
                    let only = if key == FETCH_TIER_RACE.name {
                        " is set for the Run only"
                    } else {
                        ""
                    };
                    return Err(CallError::new(
                        &SETTING_NOT_APPLICABLE,
                        format!(
                            "The Label `{name}` sets `{key}`, which a Label does not take in this engine{only}: only `fetch.tier`, `fetch.encoding` and `session.page_check` are set per Label; set `{key}` for the Run instead."
                        ),
                        &SettingNotApplicableDetails {
                            key,
                            layer: SettingsLayer::Run,
                            fix: "the Run's settings".to_owned(),
                        },
                    ));
                }
                let text = value.as_str();
                tier = match (text, text.and_then(FetchTier::named)) {
                    (Some("auto"), _) => SetTier::Auto,
                    (_, Some(forced)) => SetTier::Forced(forced),
                    _ => {
                        return Err(CallError::new(
                            &SETTING_INVALID,
                            format!(
                                "The Label `{name}` sets `fetch.tier` to {value}, which is none of {}.",
                                FETCH_TIERS.join(", ")
                            ),
                            &SettingInvalidDetails {
                                key,
                                layer: Some(SettingsLayer::Run),
                                reason: format!("{value} is none of {}", FETCH_TIERS.join(", ")),
                            },
                        ));
                    }
                };
            }
            tiers.insert(name.clone(), tier);
        }
        Ok(Self {
            tiers,
            encodings,
            page_checks,
        })
    }

    /// Each declared Label that sets `fetch.encoding`, with its label.
    pub fn encodings(&self) -> impl Iterator<Item = (&str, &str)> {
        self.encodings
            .iter()
            .map(|(name, encoding)| (name.as_str(), encoding.as_str()))
    }

    /// The Page check the Label `label` declares of its own, if it does.
    pub fn page_check(&self, label: &str) -> Option<&PageCheck> {
        self.page_checks.get(label)
    }

    /// Each declared Label that forces a tier, with it.
    pub fn forced(&self) -> impl Iterator<Item = (&str, FetchTier)> {
        self.tiers.iter().filter_map(|(name, tier)| match tier {
            SetTier::Forced(tier) => Some((name.as_str(), *tier)),
            SetTier::Unset | SetTier::Auto => None,
        })
    }

    /// Whether a declared Label sets `fetch.tier`.
    pub fn any(&self) -> bool {
        self.tiers.values().any(|tier| *tier != SetTier::Unset)
    }

    /// How the tier of a request of `label` is chosen, the Run's own
    /// `fetch.tier` being `run`.
    pub fn choose(&self, label: &str, run: SetTier) -> Choosing {
        match (
            self.tiers.get(label).copied().unwrap_or(SetTier::Unset),
            run,
        ) {
            (SetTier::Forced(tier), _) => Choosing::Forced(TierChoice {
                tier,
                forced: Some(Forced {
                    label: Some(label.to_owned()),
                }),
            }),
            (SetTier::Auto, _) | (SetTier::Unset, SetTier::Auto) => Choosing::Engine,
            (SetTier::Unset, SetTier::Forced(tier)) => Choosing::Forced(TierChoice {
                tier,
                forced: Some(Forced { label: None }),
            }),
            (SetTier::Unset, SetTier::Unset) => Choosing::Alone,
        }
    }
}

/// The WHATWG Encoding label the Label `name` sets `fetch.encoding` to, as
/// given; `None` for `null`.
///
/// # Errors
///
/// `setting_invalid` for a value that is no Encoding label.
pub(crate) fn encoding(name: &str, value: &Value) -> Result<Option<String>, CallError> {
    match value {
        Value::Null => Ok(None),
        Value::String(label) if Encoding::for_label(label.as_bytes()).is_some() => {
            Ok(Some(label.clone()))
        }
        other => Err(CallError::new(
            &SETTING_INVALID,
            format!(
                "The Label `{name}` sets `fetch.encoding` to {other}, which is no WHATWG Encoding label, such as `utf-8` or `shift_jis`."
            ),
            &SettingInvalidDetails {
                key: FETCH_ENCODING.name.to_owned(),
                layer: Some(SettingsLayer::Run),
                reason: format!("{other} is no WHATWG Encoding label"),
            },
        )),
    }
}

/// Every key of the nested settings `settings` under `prefix`, with its
/// value.
fn flatten(prefix: &str, settings: &Map<String, Value>, keys: &mut Vec<(String, Value)>) {
    for (name, value) in settings {
        let key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        match value {
            // A Page check is one setting's value, read whole.
            Value::Object(nested) if key != SESSION_PAGE_CHECK.name => {
                flatten(&key, nested, keys);
            }
            leaf => keys.push((key, leaf.clone())),
        }
    }
}

/// The Page check `value` the Label `name` sets, `None` for `null`.
fn page_check_of(name: &str, value: &Value) -> Result<Option<PageCheck>, CallError> {
    serde_json::from_value::<Option<PageCheck>>(value.clone()).map_err(|error| {
        CallError::new(
            &SETTING_INVALID,
            format!(
                "The Label `{name}` sets `{}` to {value}, which is no Page check: {error}.",
                SESSION_PAGE_CHECK.name
            ),
            &SettingInvalidDetails {
                key: SESSION_PAGE_CHECK.name.to_owned(),
                layer: Some(SettingsLayer::Run),
                reason: format!(
                    "expected a Page check, `login_page` and `expected_page` each a page match of one `kind`: {error}"
                ),
            },
        )
    })
}

/// The tier a crawl request takes, and who forced it, if anyone did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TierChoice {
    /// The tier.
    pub tier: FetchTier,
    /// Who forced it with `fetch.tier`; `None` under `auto`.
    pub forced: Option<Forced>,
}

/// Who forced a tier with `fetch.tier`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forced {
    /// The Label that set it, or `None` for the Run.
    pub label: Option<String>,
}

impl TierChoice {
    /// How Provenance names the way it was chosen: `forced`, or
    /// `compared` under `auto`, the crawl's sample comparison having chosen
    /// it (ADR 0023).
    pub fn chosen(&self) -> &'static str {
        if self.forced.is_some() {
            "forced"
        } else {
            "compared"
        }
    }
}

/// What a call its page's tier cannot serve exactly leads to.
#[derive(Debug)]
pub(crate) enum Unserved {
    /// Under `auto`: the attempt ends and its request runs again in the
    /// browser; the call fails with this `attempt_ended`.
    Escalated(CallError),
    /// Under a forced tier: the call fails with this `browser_required`.
    Refused(CallError),
}

/// What `call`, an operation's name, that `choice`'s tier cannot serve
/// exactly leads to; `waits` when the call waits for content the
/// document lacks.
pub(crate) fn unserved(choice: &TierChoice, call: &str, waits: bool) -> Unserved {
    let tier = choice.tier.as_str();
    match &choice.forced {
        None => Unserved::Escalated(CallError::new(
            &ATTEMPT_ENDED,
            format!(
                "`{call}` cannot be served exactly on the `{tier}` tier, so the attempt ended and its request runs again in the browser."
            ),
            &json!({ "reason": "escalated", "call": call, "tier": tier }),
        )),
        Some(forced) => {
            let set = forced.label.as_ref().map_or_else(
                || "the Run".to_owned(),
                |label| format!("the Label `{label}`"),
            );
            let lacks = if waits {
                ", or the page may simply lack what it waits for"
            } else {
                ""
            };
            Unserved::Refused(CallError::new(
                &BROWSER_REQUIRED,
                format!(
                    "`{call}` cannot be served exactly on the `{tier}` tier, which {set} forces with `fetch.tier`, so it does not escalate to the browser: set `fetch.tier` to `auto` or `browser`{lacks}."
                ),
                &BrowserRequiredDetails {
                    key: FETCH_TIER.name.to_owned(),
                    tier: choice.tier,
                    label: forced.label.clone(),
                    call: call.to_owned(),
                    fix: format!("`fetch.tier: auto` or `fetch.tier: browser`{lacks}"),
                },
            ))
        }
    }
}

#[cfg(test)]
mod tests;
