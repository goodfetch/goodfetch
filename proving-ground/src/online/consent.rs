//! The consent vendors of the online copy (#168): each one's `/<vendor>`
//! page embeds the vendor's real Consent prompt, configured on the vendor's
//! side, and a script of the copy's own that asks `/<vendor>/answer` once
//! the vendor's consent cookies change. That path reads the answer from
//! the cookies the vendor set on the copy's own domain, never from the
//! tool: a refusal of every optional category passes; an acceptance of
//! some or all of them fails, as does a prompt left unanswered; a cookie
//! the copy cannot read, or a prompt the vendor did not need to show, is
//! inconclusive.
//!
//! - OneTrust: answered once `OptanonAlertBoxClosed` is set; `OptanonConsent`,
//!   a form-encoded text, lists each group's state in `groups`
//!   (`C0001:1,C0002:0,…`), `C0001` the strictly necessary one.
//! - Cookiebot: `CookieConsent`, percent-encoded, holds `preferences`,
//!   `statistics` and `marketing` as `true` or `false`; `-1` when the
//!   visitor's region needs no consent.
//! - `CookieYes`: `cookieyes-consent` (`action:yes,functional:no,…`), answered
//!   once `action` is `yes`, each other category `yes` or `no`.
//! - Klaro, served from its package, which needs no account:
//!   `klaro` holds a JSON object of each service's state, the page's
//!   services being `analytics` and `marketing`.

use hyper::header::{COOKIE, HeaderMap};
use serde::Deserialize;

use super::{Config, Outcome, Vendor, Verdict, escape};
use crate::exchange::{self, Reply};

/// OneTrust's section: its domain script's ID.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OneTrustSection {
    domain_script: String,
}

/// Cookiebot's section: its domain group's ID.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CookiebotSection {
    cbid: String,
}

/// `CookieYes`'s section: the site's client ID.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CookieYesSection {
    client_id: String,
}

/// Klaro's section, empty: the page carries its configuration.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KlaroSection {}

/// The page's Klaro services, all optional.
const KLARO_SERVICES: [&str; 2] = ["analytics", "marketing"];

/// Klaro's configuration and package.
const KLARO_EMBED: &str = r#"<script>window.klaroConfig = { mustConsent: false, acceptAll: true, services: [{ name: "analytics", title: "Analytics", purposes: ["analytics"] }, { name: "marketing", title: "Marketing", purposes: ["marketing"] }] };</script><script defer src="https://cdn.jsdelivr.net/npm/klaro@0.7.21/dist/klaro.js"></script>"#;

/// The copy's script: asks `/<vendor>/answer` whenever the cookies its
/// `data-cookies` names change.
const WATCH: &str = r#"(() => {
  const names = document.currentScript.dataset.cookies.split(" ");
  const answer = location.pathname.replace(/\/$/, "") + "/answer";
  let seen = "";
  setInterval(() => {
    const now = document.cookie.split("; ").filter((pair) => names.includes(pair.split("=")[0])).sort().join("; ");
    if (now !== seen) {
      seen = now;
      fetch(answer, { credentials: "same-origin" });
    }
  }, 250);
})();"#;

/// A configured consent vendor's prompt.
pub(super) enum Prompt {
    OneTrust { domain_script: String },
    Cookiebot { cbid: String },
    CookieYes { client_id: String },
    Klaro,
}

/// What a prompt's cookies say of the visitor's answer.
enum Reading {
    /// Answered: each optional category's state, on or off.
    Answered(Vec<bool>),
    Unanswered,
    Malformed,
    /// The vendor needed no answer of this visitor.
    NotRequired,
}

/// The prompts `config` names, in the order of their vendors.
pub(super) fn prompts(config: &Config) -> impl Iterator<Item = Prompt> {
    [
        config.onetrust.as_ref().map(|section| Prompt::OneTrust {
            domain_script: section.domain_script.clone(),
        }),
        config.cookiebot.as_ref().map(|section| Prompt::Cookiebot {
            cbid: section.cbid.clone(),
        }),
        config.cookieyes.as_ref().map(|section| Prompt::CookieYes {
            client_id: section.client_id.clone(),
        }),
        config.klaro.as_ref().map(|_| Prompt::Klaro),
    ]
    .into_iter()
    .flatten()
}

impl Prompt {
    pub(super) fn vendor(&self) -> Vendor {
        match self {
            Self::OneTrust { .. } => Vendor::OneTrust,
            Self::Cookiebot { .. } => Vendor::Cookiebot,
            Self::CookieYes { .. } => Vendor::CookieYes,
            Self::Klaro => Vendor::Klaro,
        }
    }

    /// The cookies the vendor records its answer in.
    fn cookies(&self) -> &'static str {
        match self {
            Self::OneTrust { .. } => "OptanonConsent OptanonAlertBoxClosed",
            Self::Cookiebot { .. } => "CookieConsent",
            Self::CookieYes { .. } => "cookieyes-consent",
            Self::Klaro => "klaro",
        }
    }

    /// The page: the vendor's prompt over an article, and the copy's
    /// script that reports each change of its cookies.
    pub(super) fn page(&self) -> Reply {
        let embed = match self {
            Self::OneTrust { domain_script } => format!(
                r#"<script src="https://cdn.cookielaw.org/scripttemplates/otSDKStub.js" data-domain-script="{}" charset="UTF-8"></script><script>function OptanonWrapper() {{}}</script>"#,
                escape(domain_script)
            ),
            Self::Cookiebot { cbid } => format!(
                r#"<script id="Cookiebot" src="https://consent.cookiebot.com/uc.js" data-cbid="{}"></script>"#,
                escape(cbid)
            ),
            Self::CookieYes { client_id } => format!(
                r#"<script id="cookieyes" src="https://cdn-cookieyes.com/client_data/{}/script.js"></script>"#,
                escape(client_id)
            ),
            Self::Klaro => KLARO_EMBED.to_owned(),
        };
        let name = self.vendor().name();
        Reply::html(format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>{name}</title>{embed}</head><body><main><h1>An article</h1><p>The article this site's Consent prompt covers.</p></main><script data-cookies=\"{}\">{WATCH}</script></body></html>",
            self.cookies()
        ))
    }

    /// The verdict of the answer the request's cookies record.
    pub(super) fn answer(&self, headers: &HeaderMap) -> Verdict {
        let reading = match self {
            Self::OneTrust { .. } => onetrust(headers),
            Self::Cookiebot { .. } => cookiebot(headers),
            Self::CookieYes { .. } => cookieyes(headers),
            Self::Klaro => klaro(headers),
        };
        let vendor = self.vendor();
        match reading {
            Reading::Answered(states) if states.is_empty() => {
                Verdict::new(vendor, Outcome::Inconclusive, "malformed_cookie")
            }
            Reading::Answered(states) if states.iter().all(|on| !on) => Verdict::passed(vendor),
            Reading::Answered(states) if states.iter().all(|on| *on) => {
                Verdict::new(vendor, Outcome::Failed, "accepted")
            }
            Reading::Answered(_) => Verdict::new(vendor, Outcome::Failed, "partially_accepted"),
            Reading::Unanswered => Verdict::new(vendor, Outcome::Failed, "unanswered"),
            Reading::Malformed => Verdict::new(vendor, Outcome::Inconclusive, "malformed_cookie"),
            Reading::NotRequired => {
                Verdict::new(vendor, Outcome::Inconclusive, "consent_not_required")
            }
        }
    }
}

/// The value of the request's cookie `name`, as sent.
fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|cookies| exchange::cookie(cookies, name))
}

/// `text`, which reads `on` or `off`, as `true` or `false`.
fn state(text: &str, on: &str, off: &str) -> Option<bool> {
    (text == on || text == off).then_some(text == on)
}

fn onetrust(headers: &HeaderMap) -> Reading {
    if cookie(headers, "OptanonAlertBoxClosed").is_none() {
        return Reading::Unanswered;
    }
    let Some(groups) =
        cookie(headers, "OptanonConsent").and_then(|value| exchange::form_value(value, "groups"))
    else {
        return Reading::Malformed;
    };
    groups
        .split(',')
        .filter(|group| !group.is_empty())
        .filter_map(|group| match group.split_once(':') {
            Some(("C0001", _)) => None,
            Some((_, value)) => Some(state(value, "1", "0")),
            None => Some(None),
        })
        .collect::<Option<Vec<bool>>>()
        .map_or(Reading::Malformed, Reading::Answered)
}

fn cookiebot(headers: &HeaderMap) -> Reading {
    let Some(value) = cookie(headers, "CookieConsent") else {
        return Reading::Unanswered;
    };
    if value == "-1" {
        return Reading::NotRequired;
    }
    let Some(text) = exchange::decode(value) else {
        return Reading::Malformed;
    };
    let Some(fields) = text
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return Reading::Malformed;
    };
    let field = |name: &str| {
        fields
            .split(',')
            .filter_map(|pair| pair.split_once(':'))
            .find_map(|(key, value)| (key == name).then_some(value))
            .and_then(|value| state(value, "true", "false"))
    };
    ["preferences", "statistics", "marketing"]
        .into_iter()
        .map(field)
        .collect::<Option<Vec<bool>>>()
        .map_or(Reading::Malformed, Reading::Answered)
}

fn cookieyes(headers: &HeaderMap) -> Reading {
    let Some(text) = cookie(headers, "cookieyes-consent").map(exchange::decode) else {
        return Reading::Unanswered;
    };
    let Some(text) = text else {
        return Reading::Malformed;
    };
    let pairs: Vec<(&str, &str)> = text
        .split(',')
        .filter_map(|pair| pair.split_once(':'))
        .collect();
    match pairs.iter().find(|(key, _)| *key == "action") {
        Some((_, "yes")) => {}
        Some((_, "no")) => return Reading::Unanswered,
        _ => return Reading::Malformed,
    }
    Reading::Answered(
        pairs
            .iter()
            .filter(|(key, _)| !["consentid", "consent", "action", "necessary"].contains(key))
            .filter_map(|(_, value)| state(value, "yes", "no"))
            .collect(),
    )
}

fn klaro(headers: &HeaderMap) -> Reading {
    let Some(text) = cookie(headers, "klaro").map(exchange::decode) else {
        return Reading::Unanswered;
    };
    let Some(consents) =
        text.and_then(|text| serde_json::from_str::<serde_json::Map<_, _>>(&text).ok())
    else {
        return Reading::Malformed;
    };
    KLARO_SERVICES
        .into_iter()
        .map(|service| consents.get(service).and_then(serde_json::Value::as_bool))
        .collect::<Option<Vec<bool>>>()
        .map_or(Reading::Malformed, Reading::Answered)
}
