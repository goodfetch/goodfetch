//! The online copy (#168; ADRs 0025, 0032 and 0035): the Access family's
//! second rung, which carries real vendors' challenges on
//! `challenges.test` and verifies each on the server side, so the oracle
//! reads the vendor's own verdict, never the tool's report. Each vendor
//! comes from a section of `serve --online-config`, a TOML file, with its
//! site key, the environment variable holding its secret (the deployment
//! fills it from a protected GitHub environment) and, for a test's local
//! stand-in, its verification endpoint:
//!
//! ```toml
//! [turnstile]
//! site_key = "0x4AAAAAAA..."
//! secret_env = "PROVING_GROUND_TURNSTILE_SECRET"
//! # siteverify = "https://challenges.cloudflare.com/turnstile/v0/siteverify"
//!
//! [recaptcha]
//! site_key = "6Lc..."
//! secret_env = "PROVING_GROUND_RECAPTCHA_SECRET"
//! min_score = 0.5
//!
//! [hcaptcha]
//! site_key = "..."
//! secret_env = "PROVING_GROUND_HCAPTCHA_SECRET"
//!
//! [cap]
//! site_key = "..."
//! secret_env = "PROVING_GROUND_CAP_SECRET"
//! siteverify = "https://cap.goodfetch.dev/<site_key>/siteverify"
//! ```
//!
//! A vendor's page, `/<vendor>`, loads its widget, which posts its token to
//! `/<vendor>/verify`; the server asks the vendor's endpoint about it and
//! records the verdict. A refused token, or a reCAPTCHA score below
//! `min_score`, fails; what the vendor cannot say (unreachable, out of
//! quota, an error of its own or of the copy's keys) is inconclusive, never
//! a pass (ADR 0064).

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use hyper::{Method, StatusCode};
use serde::{Deserialize, Serialize};

use crate::exchange::{self, Reply};

/// The host name the second rung's challenges are served on.
pub(crate) const HOST: &str = "challenges.test";

/// The longest a vendor's endpoint has to answer.
const VERIFICATION_TIMEOUT: Duration = Duration::from_secs(10);

/// The largest answer of a vendor's endpoint read.
const ANSWER_LIMIT: usize = 64 * 1024;

/// reCAPTCHA's own threshold when the configuration sets none.
const RECAPTCHA_MIN_SCORE: f64 = 0.5;

/// The error codes that say the copy's own keys or the vendor are at fault,
/// never the token: the vendor cannot judge the tool.
const VENDOR_SIDE_ERRORS: [&str; 6] = [
    "missing-input-secret",
    "invalid-input-secret",
    "sitekey-secret-mismatch",
    "invalid-keys",
    "bad-request",
    "internal-error",
];

/// `serve --online-config`: one optional section per vendor.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    turnstile: Option<Section>,
    recaptcha: Option<Section>,
    hcaptcha: Option<Section>,
    cap: Option<Section>,
}

/// One vendor's section.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Section {
    site_key: String,
    /// The environment variable holding the vendor's secret.
    secret_env: String,
    /// The vendor's verification endpoint, its own unless set.
    siteverify: Option<String>,
    /// reCAPTCHA's lowest passing score.
    min_score: Option<f64>,
}

impl Config {
    /// Parses the configuration's TOML text.
    pub(crate) fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).context("parsing the online configuration")
    }

    fn sections(&self) -> impl Iterator<Item = (Vendor, &Section)> {
        [
            (Vendor::Turnstile, &self.turnstile),
            (Vendor::Recaptcha, &self.recaptcha),
            (Vendor::Hcaptcha, &self.hcaptcha),
            (Vendor::Cap, &self.cap),
        ]
        .into_iter()
        .filter_map(|(vendor, section)| section.as_ref().map(|section| (vendor, section)))
    }
}

/// A vendor whose verdict the online copy reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Vendor {
    Turnstile,
    Recaptcha,
    Hcaptcha,
    Cap,
}

impl Vendor {
    fn name(self) -> &'static str {
        match self {
            Self::Turnstile => "turnstile",
            Self::Recaptcha => "recaptcha",
            Self::Hcaptcha => "hcaptcha",
            Self::Cap => "cap",
        }
    }

    /// The form field the vendor's widget puts its token in.
    fn field(self) -> &'static str {
        match self {
            Self::Turnstile => "cf-turnstile-response",
            Self::Recaptcha => "g-recaptcha-response",
            Self::Hcaptcha => "h-captcha-response",
            Self::Cap => "cap-token",
        }
    }

    /// The vendor's own verification endpoint; Cap's is the instance the
    /// copy runs, which the configuration names.
    fn siteverify(self) -> Option<&'static str> {
        match self {
            Self::Turnstile => Some("https://challenges.cloudflare.com/turnstile/v0/siteverify"),
            Self::Recaptcha => Some("https://www.google.com/recaptcha/api/siteverify"),
            Self::Hcaptcha => Some("https://api.hcaptcha.com/siteverify"),
            Self::Cap => None,
        }
    }

    /// The widget's script and element, in the form that posts its token.
    fn widget(self, site_key: &str) -> String {
        let key = escape(site_key);
        match self {
            Self::Turnstile => format!(
                r#"<script src="https://challenges.cloudflare.com/turnstile/v0/api.js" async defer></script><div class="cf-turnstile" data-sitekey="{key}"></div>"#
            ),
            Self::Recaptcha => format!(
                r#"<script src="https://www.google.com/recaptcha/api.js" async defer></script><div class="g-recaptcha" data-sitekey="{key}"></div>"#
            ),
            Self::Hcaptcha => format!(
                r#"<script src="https://js.hcaptcha.com/1/api.js" async defer></script><div class="h-captcha" data-sitekey="{key}"></div>"#
            ),
            Self::Cap => format!(
                r#"<script src="https://cdn.jsdelivr.net/npm/@cap.js/widget@0.1.26" async></script><cap-widget data-cap-api-endpoint="/cap/{key}/"></cap-widget>"#
            ),
        }
    }
}

/// What the vendor said of a tool, as the oracle reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Passed,
    Failed,
    /// The vendor could not say: never a pass.
    Inconclusive,
}

/// One vendor's verdict on one challenge, read on the server side.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Verdict {
    pub(crate) vendor: Vendor,
    pub(crate) outcome: Outcome,
    /// The score the vendor gave, for a vendor that scores.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) score: Option<f64>,
    /// Why it is not a pass: the vendor's error codes, or the copy's own
    /// reason, such as `unreachable`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

impl Verdict {
    fn new(vendor: Vendor, outcome: Outcome, reason: &str) -> Self {
        Self {
            vendor,
            outcome,
            score: None,
            reason: Some(reason.to_owned()),
        }
    }
}

/// A configured vendor, its secret read.
struct Configured {
    vendor: Vendor,
    site_key: String,
    secret: String,
    siteverify: String,
    min_score: f64,
}

/// The online copy's vendors, and the client that asks their endpoints.
pub(crate) struct Online {
    vendors: Vec<Configured>,
    client: reqwest::Client,
}

/// What a vendor's endpoint answers.
#[derive(Deserialize)]
struct SiteverifyAnswer {
    success: bool,
    #[serde(default, rename = "error-codes")]
    error_codes: Vec<String>,
    score: Option<f64>,
}

impl Online {
    /// The copy of `config`'s vendors, each with the secret `secret` gives
    /// for its variable.
    ///
    /// # Errors
    ///
    /// When a vendor's secret is unset, Cap names no endpoint, a vendor
    /// other than reCAPTCHA sets `min_score`, or the client cannot be built.
    pub(crate) fn new(config: &Config, secret: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let mut vendors = Vec::new();
        for (vendor, section) in config.sections() {
            let Some(value) = secret(&section.secret_env).filter(|value| !value.is_empty()) else {
                bail!(
                    "the {} secret's variable {} is unset",
                    vendor.name(),
                    section.secret_env
                );
            };
            let Some(siteverify) = section
                .siteverify
                .clone()
                .or_else(|| vendor.siteverify().map(str::to_owned))
            else {
                bail!("the {} section names no siteverify endpoint", vendor.name());
            };
            if section.min_score.is_some() && vendor != Vendor::Recaptcha {
                bail!(
                    "only reCAPTCHA scores: the {} section sets min_score",
                    vendor.name()
                );
            }
            vendors.push(Configured {
                vendor,
                site_key: section.site_key.clone(),
                secret: value,
                siteverify,
                min_score: section.min_score.unwrap_or(RECAPTCHA_MIN_SCORE),
            });
        }
        Ok(Self {
            vendors,
            client: client()?,
        })
    }

    /// The reply to a request on [`HOST`] to `path`, with the verdict it
    /// read; `None` for a path of no configured vendor.
    pub(crate) async fn respond(
        &self,
        method: &Method,
        path: &str,
        body: &[u8],
    ) -> Option<(Reply, Option<Verdict>)> {
        let rest = path.strip_prefix('/')?;
        let (name, verify) = match rest.split_once('/') {
            Some((name, "verify")) => (name, true),
            Some(_) => return None,
            None => (rest, false),
        };
        let configured = self
            .vendors
            .iter()
            .find(|configured| configured.vendor.name() == name)?;
        match (method, verify) {
            (&Method::GET, false) => Some((page(configured), None)),
            (&Method::POST, true) => {
                let token = std::str::from_utf8(body)
                    .ok()
                    .and_then(|form| exchange::form_value(form, configured.vendor.field()))
                    .filter(|token| !token.is_empty());
                let verdict = match token {
                    Some(token) => self.verify(configured, &token).await,
                    None => Verdict::new(configured.vendor, Outcome::Failed, "missing_token"),
                };
                Some((verdict_page(&verdict), Some(verdict)))
            }
            _ => Some((
                Reply::error(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed"),
                None,
            )),
        }
    }

    /// Asks the vendor's endpoint about `token`.
    async fn verify(&self, configured: &Configured, token: &str) -> Verdict {
        let vendor = configured.vendor;
        let request = self.client.post(&configured.siteverify);
        let request = if vendor == Vendor::Cap {
            let body = serde_json::json!({ "secret": configured.secret, "response": token });
            request
                .header("content-type", "application/json")
                .body(body.to_string())
        } else {
            let mut form = form_urlencoded::Serializer::new(String::new());
            form.append_pair("secret", &configured.secret)
                .append_pair("response", token);
            if vendor == Vendor::Hcaptcha {
                form.append_pair("sitekey", &configured.site_key);
            }
            request
                .header("content-type", "application/x-www-form-urlencoded")
                .body(form.finish())
        };
        let Ok(mut response) = request.send().await else {
            return Verdict::new(vendor, Outcome::Inconclusive, "unreachable");
        };
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Verdict::new(vendor, Outcome::Inconclusive, "out_of_quota");
        }
        if !status.is_success() {
            let reason = format!("status_{}", status.as_u16());
            return Verdict::new(vendor, Outcome::Inconclusive, &reason);
        }
        let mut bytes = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) if bytes.len() + chunk.len() <= ANSWER_LIMIT => {
                    bytes.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Ok(Some(_)) => {
                    return Verdict::new(vendor, Outcome::Inconclusive, "answer_too_large");
                }
                Err(_) => return Verdict::new(vendor, Outcome::Inconclusive, "unreachable"),
            }
        }
        let Ok(answer) = serde_json::from_slice::<SiteverifyAnswer>(&bytes) else {
            return Verdict::new(vendor, Outcome::Inconclusive, "malformed_answer");
        };
        judge(configured, &answer)
    }
}

/// The verdict of a vendor's answer.
fn judge(configured: &Configured, answer: &SiteverifyAnswer) -> Verdict {
    let vendor = configured.vendor;
    if !answer.success {
        let outcome = if answer
            .error_codes
            .iter()
            .any(|code| VENDOR_SIDE_ERRORS.contains(&code.as_str()))
        {
            Outcome::Inconclusive
        } else {
            Outcome::Failed
        };
        let reason = if answer.error_codes.is_empty() {
            "refused".to_owned()
        } else {
            answer.error_codes.join(" ")
        };
        return Verdict {
            score: answer.score,
            ..Verdict::new(vendor, outcome, &reason)
        };
    }
    match answer.score {
        Some(score) if vendor == Vendor::Recaptcha && score < configured.min_score => Verdict {
            score: Some(score),
            ..Verdict::new(vendor, Outcome::Failed, "score_below_threshold")
        },
        score => Verdict {
            vendor,
            outcome: Outcome::Passed,
            score,
            reason: None,
        },
    }
}

/// The client that asks the vendors' endpoints: rustls on the ring
/// provider, trusting the Mozilla roots, over HTTP/1.1.
fn client() -> Result<reqwest::Client> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .context("choosing the TLS versions")?
    .with_root_certificates(roots)
    .with_no_client_auth();
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    #[expect(
        clippy::disallowed_methods,
        reason = "the online copy asks the vendors' endpoints itself: the Proving ground never judges with the HTTP fetch tier it judges (ADR 0032)"
    )]
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .timeout(VERIFICATION_TIMEOUT)
        .no_proxy()
        .build()
        .context("building the vendors' client")
}

/// A vendor's page: its widget in the form that posts its token.
fn page(configured: &Configured) -> Reply {
    let name = configured.vendor.name();
    let mut page = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{name}</title></head><body>"
    );
    let _ = write!(
        page,
        r#"<form method="post" action="/{name}/verify">{}<button type="submit">Continue</button></form></body></html>"#,
        configured.vendor.widget(&configured.site_key)
    );
    Reply::html(page)
}

/// The page that answers a verification with its verdict.
fn verdict_page(verdict: &Verdict) -> Reply {
    let outcome = match verdict.outcome {
        Outcome::Passed => "passed",
        Outcome::Failed => "failed",
        Outcome::Inconclusive => "inconclusive",
    };
    Reply::html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{outcome}</title></head><body><p id=\"verdict\">{outcome}</p></body></html>"
    ))
}

/// `text` escaped for an HTML attribute.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
