//! `serve --online-config`'s shape: one optional section per vendor,
//! consent vendors included, the edge's secret, and the public origins a
//! deployed copy's tasks start from.

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

/// `serve --online-config`: the copy's public origin, one optional section
/// per vendor, and the edge.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    /// The origin a deployed copy's tasks start from, such as
    /// `https://challenges.goodfetch.dev`; unless set, `challenges.test` on
    /// the bound port.
    pub(super) origin: Option<String>,
    pub(super) turnstile: Option<TokenSection>,
    pub(super) recaptcha: Option<TokenSection>,
    pub(super) hcaptcha: Option<TokenSection>,
    pub(super) cap: Option<TokenSection>,
    pub(super) altcha: Option<AltchaSection>,
    pub(super) anubis: Option<KeySection>,
    pub(super) edge: Option<KeySection>,
    pub(super) cloudflare_bot_score: Option<EdgeSection>,
    pub(super) aws_waf: Option<EdgeSection>,
    pub(super) botid: Option<EdgeSection>,
    pub(super) onetrust: Option<OneTrustSection>,
    pub(super) cookiebot: Option<CookiebotSection>,
    pub(super) cookieyes: Option<CookieYesSection>,
    pub(super) klaro: Option<KlaroSection>,
}

/// A token vendor's section.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TokenSection {
    pub(super) site_key: String,
    /// The environment variable holding the vendor's secret.
    pub(super) secret_env: String,
    /// The vendor's verification endpoint, its own unless set.
    pub(super) siteverify: Option<String>,
    /// reCAPTCHA's lowest passing score.
    pub(super) min_score: Option<f64>,
}

/// ALTCHA's section.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AltchaSection {
    /// The environment variable holding the key challenges are signed with.
    pub(super) secret_env: String,
    /// The largest secret number of a challenge: its difficulty.
    pub(super) max_number: Option<u64>,
}

/// A section that names only its secret's variable: Anubis's, the edge's.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KeySection {
    pub(super) secret_env: String,
}

/// An edge vendor's section.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EdgeSection {
    /// Cloudflare's lowest passing bot score.
    pub(super) min_score: Option<f64>,
    /// The origin the vendor's task starts from when its edge serves
    /// another host name than the copy's, such as Vercel's for `BotID`.
    pub(super) origin: Option<String>,
}

/// OneTrust's section: its domain script's ID.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OneTrustSection {
    pub(super) domain_script: String,
}

/// Cookiebot's section: its domain group's ID.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CookiebotSection {
    pub(super) cbid: String,
}

/// `CookieYes`'s section: the site's client ID.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CookieYesSection {
    pub(super) client_id: String,
}

/// Klaro's section, empty: the page carries its configuration.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KlaroSection {}

impl Config {
    /// Parses the configuration's TOML text.
    ///
    /// # Errors
    ///
    /// When it is no such configuration, or an origin it names is not a
    /// bare origin.
    pub(crate) fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text).context("parsing the online configuration")?;
        let edge_origins = [&config.cloudflare_bot_score, &config.aws_waf, &config.botid]
            .into_iter()
            .flatten()
            .filter_map(|section| section.origin.as_deref());
        for origin in config.origin.as_deref().into_iter().chain(edge_origins) {
            bare_origin(origin)?;
        }
        Ok(config)
    }
}

/// Refuses an origin that is not `http(s)://<host>[:<port>]`, as a task's
/// start URL appends the vendor's path to it.
fn bare_origin(origin: &str) -> Result<()> {
    let host = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"));
    match host {
        Some(host) if !host.is_empty() && !host.contains(['/', '?', '#']) => Ok(()),
        _ => bail!("the origin `{origin}` is not a bare origin, such as `https://<host>`"),
    }
}
