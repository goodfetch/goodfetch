//! The tier each crawl request takes, and what a call its tier cannot
//! serve exactly leads to (spec #579's `tiers`; ADRs 0020, 0023 and 0093).
//!
//! `fetch.tier` forces a tier for the Run, or for one Label, whose own
//! value wins for its requests. Under `auto`, set for its Label or its
//! Run or left at the built-in default, the engine chooses: the crawl
//! learns the tier per Label on a domain (`goodfetch-crawl`'s
//! `tier_choice`), its HTTP leaving from where `fetch.http_from` says. A
//! Label also takes `fetch.http_from`, `fetch.encoding`,
//! `crawl.content_types` and `session.page_check`. A call the request's tier cannot serve exactly
//! escalates the request to the browser under `auto`, and fails with
//! `browser_required` under a forced tier, which nothing escalates.

use std::collections::BTreeMap;

use encoding_rs::Encoding;
use goodfetch_engine::CallError;
use goodfetch_extractor::BuiltIn;
use goodfetch_protocol::foxhound::crawl::{ATTEMPT_ENDED, CRAWL_CONTENT_TYPES, LabelDeclaration};
use goodfetch_protocol::foxhound::fetch::{
    BROWSER_REQUIRED, BrowserRequiredDetails, FETCH_ENCODING, FETCH_HTTP_FROM,
    FETCH_KEPT_RESPONSES_URLS, FETCH_TIER, FETCH_TIER_RACE, FETCH_TIERS, FetchTier,
    HTTP_FROM_VALUES,
};
use goodfetch_protocol::foxhound::frontier::CRAWL_KEEP_URL_FRAGMENT;
use goodfetch_protocol::page::{CONTENT_EXTRACTOR, CONTENT_EXTRACTORS};
use goodfetch_protocol::settings::page_check::PageCheck;
use goodfetch_protocol::settings::{
    SESSION_PAGE_CHECK, SESSION_SOURCE, SETTING_INVALID, SETTING_NOT_APPLICABLE,
    SettingInvalidDetails, SettingNotApplicableDetails, SettingsLayer,
};
use serde_json::{Map, Value, json};

use crate::content_types::ContentTypes;

/// What the keys inside a Label's `session.source` start with, once its
/// shape is spelled out key by key.
const SESSION_SOURCE_KEYS: &str = "session.source.";

/// What every `proxy.*` key starts with: the Run's own, never a Label's.
const PROXY_KEYS: &str = "proxy.";

/// The Labels a crawl declares, each with what it sets of its own: its
/// `fetch.tier`, the `fetch.http_from`, the `fetch.encoding`, `crawl.content_types` (ADR 0119) and
/// `session.page_check` it sets, if any (ADR 0093), whether it keeps its URLs' fragments
/// (`crawl.keep_url_fragment`, ADR 0120) and whether it keeps responses
/// for its handler (`fetch.kept_responses.urls`, ADR 0110), when it says,
/// and the main-content step its pages' `formats` read with
/// (`content.extractor`, spec #580). Its `session.source` is the Session
/// pool's to read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabelSettings {
    tiers: BTreeMap<String, SetTier>,
    http_from: BTreeMap<String, HttpFrom>,
    encodings: BTreeMap<String, String>,
    content_types: BTreeMap<String, ContentTypes>,
    page_checks: BTreeMap<String, PageCheck>,
    fragments: BTreeMap<String, bool>,
    kept_responses: BTreeMap<String, bool>,
    extractors: BTreeMap<String, BuiltIn>,
}

/// How a request's tier is chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choosing {
    /// Under `auto`, set or the built-in default: the engine chooses.
    Engine,
    /// `fetch.tier` forces this tier.
    Forced(TierChoice),
}

/// Where a request's HTTP leaves from where the engine chooses its tier:
/// `fetch.http_from`, the Run's or a declared Label's own (ADR 0023).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HttpFrom {
    /// From inside the Session's browser, as `browser_fetch`: the default.
    #[default]
    Browser,
    /// Learned per Label; as [`HttpFrom::Transport`] until the engine
    /// learns which client a site accepts.
    Learned,
    /// From a Session's own HTTP transport, as `http`.
    Transport,
}

impl HttpFrom {
    /// The value `name` names, if it names one.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "browser" => Some(Self::Browser),
            "learned" => Some(Self::Learned),
            "transport" => Some(Self::Transport),
            _ => None,
        }
    }

    /// The Run's own `fetch.http_from` under `settings`.
    pub fn of_run(settings: &goodfetch_engine::settings::Resolved) -> Self {
        settings
            .value(&FETCH_HTTP_FROM)
            .as_str()
            .and_then(Self::named)
            .unwrap_or_default()
    }

    /// Whether the HTTP it chooses leaves from a Session's own transport
    /// rather than from inside its browser.
    pub const fn from_transport(self) -> bool {
        match self {
            Self::Browser => false,
            Self::Learned | Self::Transport => true,
        }
    }
}

/// The Run's `fetch.tier`, or a declared Label's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetTier {
    /// None is set, for a Label: the Run's applies.
    Unset,
    /// `auto`: set, or for the Run the built-in default.
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
    /// key but `fetch.tier`, `fetch.http_from`, `fetch.encoding`,
    /// `crawl.content_types`,
    /// `session.page_check`, `crawl.keep_url_fragment`,
    /// `fetch.kept_responses.urls`, `content.extractor` and
    /// `session.source`, which the Session pool reads; `fetch.tier_race` and every `proxy.*` key, the pools'
    /// identity policy, are the Run's only), and
    /// `setting_invalid` for a `fetch.tier` that names no tier, a
    /// `fetch.http_from` that names no sender, a `fetch.encoding` that is
    /// no WHATWG Encoding label, a
    /// `crawl.content_types` that is no list of media ranges, a
    /// `session.page_check` that is no Page check, a
    /// `crawl.keep_url_fragment` that is no Boolean, a
    /// `fetch.kept_responses.urls` that is no list of patterns or a
    /// `content.extractor` that names no built-in.
    pub fn of(labels: &BTreeMap<String, LabelDeclaration>) -> Result<Self, CallError> {
        let mut tiers = BTreeMap::new();
        let mut http_from = BTreeMap::new();
        let mut encodings = BTreeMap::new();
        let mut content_types = BTreeMap::new();
        let mut page_checks = BTreeMap::new();
        let mut fragments = BTreeMap::new();
        let mut kept_responses = BTreeMap::new();
        let mut extractors = BTreeMap::new();
        for (name, declaration) in labels {
            let mut keys = Vec::new();
            flatten("", &declaration.settings, &mut keys);
            let mut tier = SetTier::Unset;
            for (key, value) in keys {
                if key == FETCH_HTTP_FROM.name {
                    http_from.insert(name.clone(), http_from_of(name, &key, &value)?);
                    continue;
                }
                if key == FETCH_ENCODING.name {
                    if let Some(encoding) = encoding(name, &value)? {
                        encodings.insert(name.clone(), encoding);
                    }
                    continue;
                }
                if key == CRAWL_CONTENT_TYPES.name {
                    let types = ContentTypes::of(&value)
                        .map_err(|reason| invalid(name, &key, &value, &reason))?;
                    content_types.insert(name.clone(), types);
                    continue;
                }
                if key == SESSION_PAGE_CHECK.name {
                    if let Some(check) = page_check_of(name, &value)? {
                        page_checks.insert(name.clone(), check);
                    }
                    continue;
                }
                if key == CRAWL_KEEP_URL_FRAGMENT.name {
                    let Value::Bool(keeps) = value else {
                        return Err(invalid(name, &key, &value, "is no Boolean"));
                    };
                    fragments.insert(name.clone(), keeps);
                    continue;
                }
                if key == FETCH_KEPT_RESPONSES_URLS.name {
                    let Some(patterns) = value
                        .as_array()
                        .filter(|patterns| patterns.iter().all(Value::is_string))
                    else {
                        return Err(invalid(name, &key, &value, "is no list of URL patterns"));
                    };
                    kept_responses.insert(name.clone(), !patterns.is_empty());
                    continue;
                }
                if key == CONTENT_EXTRACTOR.name {
                    let Some(extractor) = value.as_str().and_then(BuiltIn::named) else {
                        return Err(invalid(
                            name,
                            &key,
                            &value,
                            &format!("is none of {}", CONTENT_EXTRACTORS.join(", ")),
                        ));
                    };
                    extractors.insert(name.clone(), extractor);
                    continue;
                }
                // The Session pool reads a Label's own Session source.
                if key == SESSION_SOURCE.name || key.starts_with(SESSION_SOURCE_KEYS) {
                    continue;
                }
                if key != FETCH_TIER.name {
                    return Err(not_taken(name, key));
                }
                tier = tier_of(name, key, &value)?;
            }
            tiers.insert(name.clone(), tier);
        }
        Ok(Self {
            tiers,
            http_from,
            encodings,
            content_types,
            page_checks,
            fragments,
            kept_responses,
            extractors,
        })
    }

    /// The main-content step the Label `label` chooses with
    /// `content.extractor`: `None` when it leaves it to the Run.
    pub fn extractor(&self, label: &str) -> Option<BuiltIn> {
        self.extractors.get(label).copied()
    }

    /// Each declared Label, with the `fetch.tier` it sets.
    pub fn tiers(&self) -> impl Iterator<Item = (&str, SetTier)> {
        self.tiers.iter().map(|(name, tier)| (name.as_str(), *tier))
    }

    /// Each declared Label that sets `crawl.keep_url_fragment`, with its
    /// value.
    pub fn fragments(&self) -> impl Iterator<Item = (&str, bool)> {
        self.fragments
            .iter()
            .map(|(name, keeps)| (name.as_str(), *keeps))
    }

    /// The `crawl.keep_url_fragment` the Label `label` sets: `None` when
    /// it leaves it to the Run.
    pub fn keeps_fragment(&self, label: &str) -> Option<bool> {
        self.fragments.get(label).copied()
    }

    /// Whether the Label `label` sets `fetch.kept_responses.urls`, and to
    /// a list that keeps any: `None` when it leaves it to the Run.
    pub fn keeps_responses(&self, label: &str) -> Option<bool> {
        self.kept_responses.get(label).copied()
    }

    /// Each declared Label that sets `fetch.http_from`, with its value.
    pub fn http_froms(&self) -> impl Iterator<Item = (&str, HttpFrom)> {
        self.http_from
            .iter()
            .map(|(name, http_from)| (name.as_str(), *http_from))
    }

    /// Where the HTTP of a request of `label` leaves from where the engine
    /// chooses its tier: its Label's `fetch.http_from`, else `run`, the
    /// Run's.
    pub fn http_from(&self, label: &str, run: HttpFrom) -> HttpFrom {
        self.http_from.get(label).copied().unwrap_or(run)
    }

    /// The encoding `fetch.encoding` forces on the own document of a
    /// request of `label` (ADR 0119): its Label's, else `run`, the Run's
    /// label.
    pub fn encoding(&self, label: &str, run: Option<&str>) -> Option<&'static Encoding> {
        self.encodings
            .get(label)
            .map(String::as_str)
            .or(run)
            .and_then(|label| Encoding::for_label(label.as_bytes()))
    }

    /// The types `crawl.content_types` lets the own document of a request
    /// of `label` be (ADR 0119): its Label's, else `run`, the Run's.
    pub fn content_types(&self, label: &str, run: &ContentTypes) -> ContentTypes {
        self.content_types.get(label).unwrap_or(run).clone()
    }

    /// The Page check the Label `label` declares of its own, if it does.
    pub fn page_check(&self, label: &str) -> Option<&PageCheck> {
        self.page_checks.get(label)
    }

    /// Each declared Label that declares a Page check of its own, with it.
    pub fn page_checks(&self) -> impl Iterator<Item = (&str, &PageCheck)> {
        self.page_checks
            .iter()
            .map(|(name, check)| (name.as_str(), check))
    }

    /// Each declared Label that forces a tier, with it.
    pub fn forced(&self) -> impl Iterator<Item = (&str, FetchTier)> {
        self.tiers.iter().filter_map(|(name, tier)| match tier {
            SetTier::Forced(tier) => Some((name.as_str(), *tier)),
            SetTier::Unset | SetTier::Auto => None,
        })
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
            (SetTier::Auto, _) | (SetTier::Unset, SetTier::Auto | SetTier::Unset) => {
                Choosing::Engine
            }
            (SetTier::Unset, SetTier::Forced(tier)) => Choosing::Forced(TierChoice {
                tier,
                forced: Some(Forced { label: None }),
            }),
        }
    }
}

/// `setting_not_applicable` for the Label `name`'s `key`, which a Label
/// does not take: a `proxy.*` key never, since a Run's proxy settings apply
/// to every Session of its pools, so one crawl presents one identity policy
/// (ADRs 0093 and 0117); `fetch.tier_race` is the Run's only; the others
/// not in this engine.
fn not_taken(name: &str, key: String) -> CallError {
    let why = if key.starts_with(PROXY_KEYS) {
        "which a Label never takes: a Run's proxy settings apply to every Session of its Session pools, so one crawl presents one identity policy".to_owned()
    } else {
        let only = if key == FETCH_TIER_RACE.name {
            " is set for the Run only"
        } else {
            ""
        };
        format!(
            "which a Label does not take in this engine{only}: only `fetch.tier`, `fetch.http_from`, `fetch.encoding`, `fetch.kept_responses.urls`, `crawl.content_types`, `crawl.keep_url_fragment`, `content.extractor`, `session.page_check` and `session.source` are set per Label"
        )
    };
    CallError::new(
        &SETTING_NOT_APPLICABLE,
        format!("The Label `{name}` sets `{key}`, {why}; set `{key}` for the Run instead."),
        &SettingNotApplicableDetails {
            key,
            layer: SettingsLayer::Run,
            fix: "the Run's settings".to_owned(),
        },
    )
}

/// The tier the Label `label` sets its `fetch.tier` to with `value`.
///
/// # Errors
///
/// `SETTING_INVALID` when `value` names neither auto nor a tier.
fn tier_of(label: &str, key: String, value: &Value) -> Result<SetTier, CallError> {
    let text = value.as_str();
    match (text, text.and_then(FetchTier::named)) {
        (Some("auto"), _) => Ok(SetTier::Auto),
        (_, Some(forced)) => Ok(SetTier::Forced(forced)),
        _ => Err(CallError::new(
            &SETTING_INVALID,
            format!(
                "The Label `{label}` sets `fetch.tier` to {value}, which is none of {}.",
                FETCH_TIERS.join(", ")
            ),
            &SettingInvalidDetails {
                key,
                layer: Some(SettingsLayer::Run),
                reason: format!("{value} is none of {}", FETCH_TIERS.join(", ")),
            },
        )),
    }
}

/// Where the Label `label` sends its HTTP from, as its `fetch.http_from`
/// sets it to `value`.
///
/// # Errors
///
/// `SETTING_INVALID` when `value` names no sender.
fn http_from_of(label: &str, key: &str, value: &Value) -> Result<HttpFrom, CallError> {
    value.as_str().and_then(HttpFrom::named).ok_or_else(|| {
        invalid(
            label,
            key,
            value,
            &format!("is none of {}", HTTP_FROM_VALUES.join(", ")),
        )
    })
}

/// `setting_invalid` for the Label `name`'s `key`, set to `value`, which
/// `reason`.
fn invalid(name: &str, key: &str, value: &Value, reason: &str) -> CallError {
    CallError::new(
        &SETTING_INVALID,
        format!("The Label `{name}` sets `{key}` to {value}, which {reason}."),
        &SettingInvalidDetails {
            key: key.to_owned(),
            layer: Some(SettingsLayer::Run),
            reason: format!("{value} {reason}"),
        },
    )
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

/// What Provenance and `browser_required` name, in place of a call, as
/// the cause of a request's own document that only a guess could decode
/// (ADR 0119): the setting that reads it on a non-browser tier.
pub(crate) const ENCODING_CALL: &str = "fetch.encoding";

/// `browser_required` for the crawl request's own document at `url`,
/// which declares no encoding and is not UTF-8, on `tier`, which `forced`
/// forced: its fix names `fetch.encoding` and `fetch.tier` (G6).
pub(crate) fn undeclared(tier: FetchTier, forced: &Forced, url: &str) -> CallError {
    let name = tier.as_str();
    let set = forced.label.as_ref().map_or_else(
        || "the Run".to_owned(),
        |label| format!("the Label `{label}`"),
    );
    CallError::new(
        &BROWSER_REQUIRED,
        format!(
            "The document at {url} declares no encoding and is not UTF-8, so only the browser's guess could read it, and the `{name}` tier, which {set} forces with `fetch.tier`, never guesses: set `fetch.encoding` to the document's encoding, or `fetch.tier` to `auto` or `browser`."
        ),
        &BrowserRequiredDetails {
            key: FETCH_TIER.name.to_owned(),
            tier,
            label: forced.label.clone(),
            call: ENCODING_CALL.to_owned(),
            fix: "`fetch.encoding` set to the document's encoding, or `fetch.tier: auto` or `fetch.tier: browser`".to_owned(),
        },
    )
}

#[cfg(test)]
mod tests;
