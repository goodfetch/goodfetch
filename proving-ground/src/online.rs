//! The online copy (#168; ADRs 0025, 0032 and 0035): the Access family's
//! second rung, which carries real vendors' challenges on
//! `challenges.test` and verifies each on the server side, so the oracle
//! reads the vendor's own verdict, never the tool's report. Each vendor
//! comes from a section of `serve --online-config`, a TOML file, which
//! names the environment variable holding each secret (the deployment
//! fills it from a protected GitHub environment):
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
//!
//! [altcha]
//! secret_env = "PROVING_GROUND_ALTCHA_KEY"
//! max_number = 100000
//!
//! [anubis]
//! secret_env = "PROVING_GROUND_ANUBIS_HS512_SECRET"
//!
//! [edge]
//! secret_env = "PROVING_GROUND_EDGE_SECRET"
//!
//! [cloudflare_bot_score]
//! min_score = 30
//!
//! [aws_waf]
//!
//! [botid]
//! ```
//!
//! Three kinds of vendor are judged:
//!
//! - The token vendors (Turnstile, reCAPTCHA, hCaptcha, Cap): `/<vendor>`
//!   loads the widget, which posts its token to `/<vendor>/verify`, and the
//!   copy asks the vendor's endpoint about it (`siteverify` points a test
//!   at a local stand-in).
//! - The proof-of-work challenges. ALTCHA's `/altcha/challenge` issues a
//!   challenge signed with the ALTCHA key, which its widget solves and
//!   posts to `/altcha/verify`; each is accepted once. Anubis is a proxy in
//!   front of the copy whose cookie, an HS512 JSON Web Token, `/anubis`
//!   verifies with the key Anubis signs with.
//! - The edge vendors (Cloudflare's bot score, AWS WAF Bot Control,
//!   Vercel's bot protection), which judge a request at the edge in front of the copy. The
//!   edge forwards its verdict in a header to `/<vendor>/verify`, trusted
//!   only beside `x-proving-ground-edge`, the `[edge]` secret.
//!
//! A refused token or solution, a forged or expired cookie, or a score
//! below `min_score` fails. What the vendor cannot say is inconclusive,
//! never a pass (ADR 0064): an unreachable vendor, one out of quota, an
//! error of its own or of the copy's keys, or a request that did not come
//! through it.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use data_encoding::{BASE64, BASE64URL_NOPAD, HEXLOWER};
use hmac::{Hmac, Mac as _};
use hyper::header::{COOKIE, HeaderMap, HeaderValue};
use hyper::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256, Sha512};

use crate::clock;
use crate::exchange::{self, Reply};
use crate::sync::Mutex;

/// The host name the second rung's challenges are served on.
pub(crate) const HOST: &str = "challenges.test";

/// The longest a vendor's endpoint has to answer.
const VERIFICATION_TIMEOUT: Duration = Duration::from_secs(10);

/// The largest answer of a vendor's endpoint read.
const ANSWER_LIMIT: usize = 64 * 1024;

/// reCAPTCHA's own threshold when the configuration sets none.
const RECAPTCHA_MIN_SCORE: f64 = 0.5;

/// The bot score under which Cloudflare calls a request likely automated.
const CLOUDFLARE_MIN_SCORE: f64 = 30.0;

/// ALTCHA's difficulty when the configuration sets none: its own default.
const ALTCHA_MAX_NUMBER: u64 = 1_000_000;

/// How long an issued ALTCHA challenge may be solved.
const ALTCHA_LIFETIME_SECONDS: u64 = 600;

/// The most ALTCHA challenges open at once; past it, none is issued.
const ALTCHA_OPEN_LIMIT: usize = 10_000;

/// The cookie Anubis sets once a client passed its challenge.
const ANUBIS_COOKIE: &str = "techaro.lol-anubis-auth";

/// The header that proves a request came through the edge.
const EDGE_HEADER: &str = "x-proving-ground-edge";

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

/// `serve --online-config`: one optional section per vendor, and the edge.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    turnstile: Option<TokenSection>,
    recaptcha: Option<TokenSection>,
    hcaptcha: Option<TokenSection>,
    cap: Option<TokenSection>,
    altcha: Option<AltchaSection>,
    anubis: Option<KeySection>,
    edge: Option<KeySection>,
    cloudflare_bot_score: Option<EdgeSection>,
    aws_waf: Option<EdgeSection>,
    botid: Option<EdgeSection>,
}

/// A token vendor's section.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenSection {
    site_key: String,
    /// The environment variable holding the vendor's secret.
    secret_env: String,
    /// The vendor's verification endpoint, its own unless set.
    siteverify: Option<String>,
    /// reCAPTCHA's lowest passing score.
    min_score: Option<f64>,
}

/// ALTCHA's section.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AltchaSection {
    /// The environment variable holding the key challenges are signed with.
    secret_env: String,
    /// The largest secret number of a challenge: its difficulty.
    max_number: Option<u64>,
}

/// A section that names only its secret's variable: Anubis's, the edge's.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeySection {
    secret_env: String,
}

/// An edge vendor's section.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EdgeSection {
    /// Cloudflare's lowest passing bot score.
    min_score: Option<f64>,
}

impl Config {
    /// Parses the configuration's TOML text.
    pub(crate) fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).context("parsing the online configuration")
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
    Altcha,
    Anubis,
    CloudflareBotScore,
    AwsWaf,
    Botid,
}

impl Vendor {
    /// Its name, in its section, its paths and its verdicts.
    fn name(self) -> &'static str {
        match self {
            Self::Turnstile => "turnstile",
            Self::Recaptcha => "recaptcha",
            Self::Hcaptcha => "hcaptcha",
            Self::Cap => "cap",
            Self::Altcha => "altcha",
            Self::Anubis => "anubis",
            Self::CloudflareBotScore => "cloudflare_bot_score",
            Self::AwsWaf => "aws_waf",
            Self::Botid => "botid",
        }
    }
}

/// A vendor whose widget hands the tool a token the copy verifies.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenVendor {
    Turnstile,
    Recaptcha,
    Hcaptcha,
    Cap,
}

impl TokenVendor {
    fn vendor(self) -> Vendor {
        match self {
            Self::Turnstile => Vendor::Turnstile,
            Self::Recaptcha => Vendor::Recaptcha,
            Self::Hcaptcha => Vendor::Hcaptcha,
            Self::Cap => Vendor::Cap,
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
    /// Why it is not a pass: the vendor's error codes or labels, or the
    /// copy's own reason, such as `unreachable`.
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

    fn passed(vendor: Vendor) -> Self {
        Self {
            vendor,
            outcome: Outcome::Passed,
            score: None,
            reason: None,
        }
    }
}

/// A configured token vendor, its secret read.
struct Siteverify {
    vendor: TokenVendor,
    site_key: String,
    secret: String,
    endpoint: String,
    min_score: f64,
}

/// ALTCHA's key, difficulty and the challenges it issued and has not
/// accepted yet.
struct Altcha {
    key: Vec<u8>,
    max_number: u64,
    issued: Mutex<Issued>,
}

struct Issued {
    /// How many challenges were issued: each one's salt and number derive
    /// from its place.
    count: u64,
    /// The salts of the challenges open.
    open: HashSet<String>,
}

/// A vendor at the edge, and how it is judged.
#[derive(Clone, Copy)]
enum EdgeVendor {
    CloudflareBotScore { min_score: f64 },
    AwsWaf,
    Botid,
}

impl EdgeVendor {
    fn vendor(self) -> Vendor {
        match self {
            Self::CloudflareBotScore { .. } => Vendor::CloudflareBotScore,
            Self::AwsWaf => Vendor::AwsWaf,
            Self::Botid => Vendor::Botid,
        }
    }

    /// The header the edge forwards its verdict in. AWS WAF prefixes the
    /// headers its rules insert with `x-amzn-waf-`.
    fn header(self) -> &'static str {
        match self {
            Self::CloudflareBotScore { .. } => "x-proving-ground-cf-bot-score",
            Self::AwsWaf => "x-amzn-waf-bot-control",
            Self::Botid => "x-proving-ground-botid",
        }
    }

    /// The verdict of the edge's header value.
    fn judge(self, value: &str) -> Verdict {
        let vendor = self.vendor();
        let malformed = || Verdict::new(vendor, Outcome::Inconclusive, "malformed_edge_verdict");
        match self {
            Self::CloudflareBotScore { min_score } => {
                let Some(score) = value
                    .parse::<u8>()
                    .ok()
                    .filter(|score| (1..=99).contains(score))
                else {
                    return malformed();
                };
                let score = f64::from(score);
                if score < min_score {
                    Verdict {
                        score: Some(score),
                        ..Verdict::new(vendor, Outcome::Failed, "score_below_threshold")
                    }
                } else {
                    Verdict {
                        score: Some(score),
                        ..Verdict::passed(vendor)
                    }
                }
            }
            Self::AwsWaf => match value {
                "none" => Verdict::passed(vendor),
                "" => malformed(),
                labels => Verdict::new(vendor, Outcome::Failed, labels),
            },
            Self::Botid => match value {
                "human" => Verdict::passed(vendor),
                "bot" => Verdict::new(vendor, Outcome::Failed, "bot"),
                "bypassed" => Verdict::new(vendor, Outcome::Inconclusive, "bypassed"),
                _ => malformed(),
            },
        }
    }
}

/// How one configured vendor is judged.
enum Check {
    Siteverify(Siteverify),
    Altcha(Altcha),
    Anubis { key: Vec<u8> },
    Edge(EdgeVendor),
}

impl Check {
    fn vendor(&self) -> Vendor {
        match self {
            Self::Siteverify(siteverify) => siteverify.vendor.vendor(),
            Self::Altcha(_) => Vendor::Altcha,
            Self::Anubis { .. } => Vendor::Anubis,
            Self::Edge(edge) => edge.vendor(),
        }
    }
}

/// The online copy's vendors, and the client that asks their endpoints.
pub(crate) struct Online {
    checks: Vec<Check>,
    /// The secret the edge sends in [`EDGE_HEADER`].
    edge_secret: Option<String>,
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

/// What ALTCHA's widget posts: the challenge it solved and its number.
#[derive(Deserialize)]
struct AltchaSolution {
    algorithm: String,
    challenge: String,
    number: u64,
    salt: String,
    signature: String,
}

/// The claims of Anubis's cookie the copy checks.
#[derive(Deserialize)]
struct AnubisClaims {
    exp: i64,
    nbf: Option<i64>,
}

impl Online {
    /// The copy of `config`'s vendors, each with the secret `secret` gives
    /// for its variable.
    ///
    /// # Errors
    ///
    /// When a secret is unset, Cap names no endpoint, a vendor other than
    /// reCAPTCHA or Cloudflare's bot score sets `min_score`, an edge vendor
    /// has no `[edge]`, or the client cannot be built.
    pub(crate) fn new(config: &Config, secret: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let read = |vendor: &str, variable: &str| {
            secret(variable)
                .filter(|value| !value.is_empty())
                .with_context(|| format!("the {vendor} secret's variable {variable} is unset"))
        };
        let mut checks = Vec::new();
        for (vendor, section) in [
            (TokenVendor::Turnstile, &config.turnstile),
            (TokenVendor::Recaptcha, &config.recaptcha),
            (TokenVendor::Hcaptcha, &config.hcaptcha),
            (TokenVendor::Cap, &config.cap),
        ] {
            let Some(section) = section else { continue };
            let name = vendor.vendor().name();
            let Some(endpoint) = section
                .siteverify
                .clone()
                .or_else(|| vendor.siteverify().map(str::to_owned))
            else {
                bail!("the {name} section names no siteverify endpoint");
            };
            if section.min_score.is_some() && vendor != TokenVendor::Recaptcha {
                bail!(
                    "the {name} section sets min_score, which only reCAPTCHA and Cloudflare's bot score take"
                );
            }
            checks.push(Check::Siteverify(Siteverify {
                vendor,
                site_key: section.site_key.clone(),
                secret: read(name, &section.secret_env)?,
                endpoint,
                min_score: section.min_score.unwrap_or(RECAPTCHA_MIN_SCORE),
            }));
        }
        if let Some(section) = &config.altcha {
            checks.push(Check::Altcha(Altcha {
                key: read("altcha", &section.secret_env)?.into_bytes(),
                max_number: section.max_number.unwrap_or(ALTCHA_MAX_NUMBER),
                issued: Mutex::new(Issued {
                    count: 0,
                    open: HashSet::new(),
                }),
            }));
        }
        if let Some(section) = &config.anubis {
            checks.push(Check::Anubis {
                key: read("anubis", &section.secret_env)?.into_bytes(),
            });
        }
        for (name, section) in [
            ("cloudflare_bot_score", &config.cloudflare_bot_score),
            ("aws_waf", &config.aws_waf),
            ("botid", &config.botid),
        ] {
            let Some(section) = section else { continue };
            if config.edge.is_none() {
                bail!("the {name} section needs an [edge] section, the secret its edge sends");
            }
            let edge = match (name, section.min_score) {
                ("cloudflare_bot_score", min_score) => EdgeVendor::CloudflareBotScore {
                    min_score: min_score.unwrap_or(CLOUDFLARE_MIN_SCORE),
                },
                (_, Some(_)) => bail!(
                    "the {name} section sets min_score, which only reCAPTCHA and Cloudflare's bot score take"
                ),
                ("aws_waf", None) => EdgeVendor::AwsWaf,
                _ => EdgeVendor::Botid,
            };
            checks.push(Check::Edge(edge));
        }
        let edge_secret = match &config.edge {
            Some(section) => Some(read("edge", &section.secret_env)?),
            None => None,
        };
        Ok(Self {
            checks,
            edge_secret,
            client: client()?,
        })
    }

    /// The reply to a request on [`HOST`] to `path`, with the verdict it
    /// read; `None` for a path of no configured vendor.
    pub(crate) async fn respond(
        &self,
        method: &Method,
        path: &str,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Option<(Reply, Option<Verdict>)> {
        let rest = path.strip_prefix('/')?;
        let (name, action) = match rest.split_once('/') {
            Some((name, action)) => (name, Some(action)),
            None => (rest, None),
        };
        let check = self
            .checks
            .iter()
            .find(|check| check.vendor().name() == name)?;
        let form = || std::str::from_utf8(body).unwrap_or_default();
        let verdict = match (check, method, action) {
            (Check::Anubis { key }, &Method::GET, None) => anubis(key, headers),
            (Check::Anubis { .. }, _, None) => return Some((method_not_allowed(), None)),
            (Check::Altcha(altcha), &Method::GET, Some("challenge")) => {
                return Some((altcha.challenge(), None));
            }
            (Check::Anubis { .. }, _, Some(_)) => return None,
            (_, &Method::GET, None) => return Some((page(check), None)),
            (_, &Method::POST, Some("verify")) => match check {
                Check::Siteverify(siteverify) => {
                    let field = siteverify.vendor.field();
                    match exchange::form_value(form(), field).filter(|token| !token.is_empty()) {
                        Some(token) => self.verify(siteverify, &token).await,
                        None => Verdict::new(check.vendor(), Outcome::Failed, "missing_token"),
                    }
                }
                Check::Altcha(altcha) => altcha.verify(exchange::form_value(form(), "altcha")),
                Check::Edge(edge) => self.edge(*edge, headers),
                Check::Anubis { .. } => return None,
            },
            (_, _, None | Some("verify")) => return Some((method_not_allowed(), None)),
            _ => return None,
        };
        Some((verdict_page(&verdict), Some(verdict)))
    }

    /// Asks the vendor's endpoint about `token`.
    async fn verify(&self, siteverify: &Siteverify, token: &str) -> Verdict {
        let vendor = siteverify.vendor.vendor();
        let request = self.client.post(&siteverify.endpoint);
        let request = if siteverify.vendor == TokenVendor::Cap {
            let body = serde_json::json!({ "secret": siteverify.secret, "response": token });
            request
                .header("content-type", "application/json")
                .body(body.to_string())
        } else {
            let mut form = form_urlencoded::Serializer::new(String::new());
            form.append_pair("secret", &siteverify.secret)
                .append_pair("response", token);
            if siteverify.vendor == TokenVendor::Hcaptcha {
                form.append_pair("sitekey", &siteverify.site_key);
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
        judge(siteverify, &answer)
    }

    /// The verdict the edge forwarded for `edge`, if the request came
    /// through the edge.
    fn edge(&self, edge: EdgeVendor, headers: &HeaderMap) -> Verdict {
        let vendor = edge.vendor();
        let sent = headers.get(EDGE_HEADER).map(HeaderValue::as_bytes);
        let through_edge = match (sent, &self.edge_secret) {
            (Some(sent), Some(secret)) => same_secret(sent, secret.as_bytes()),
            _ => false,
        };
        if !through_edge {
            return Verdict::new(vendor, Outcome::Inconclusive, "not_from_edge");
        }
        match headers.get(edge.header()).map(|value| value.to_str()) {
            None => Verdict::new(vendor, Outcome::Inconclusive, "no_edge_verdict"),
            Some(Ok(value)) => edge.judge(value.trim()),
            Some(Err(_)) => Verdict::new(vendor, Outcome::Inconclusive, "malformed_edge_verdict"),
        }
    }
}

/// The verdict of a token vendor's answer.
fn judge(siteverify: &Siteverify, answer: &SiteverifyAnswer) -> Verdict {
    let vendor = siteverify.vendor.vendor();
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
        Some(score)
            if siteverify.vendor == TokenVendor::Recaptcha && score < siteverify.min_score =>
        {
            Verdict {
                score: Some(score),
                ..Verdict::new(vendor, Outcome::Failed, "score_below_threshold")
            }
        }
        score => Verdict {
            score,
            ..Verdict::passed(vendor)
        },
    }
}

impl Altcha {
    /// A new challenge, as ALTCHA's widget fetches it: a number up to
    /// `max_number` hidden behind its SHA-256 hash with a salt that says
    /// when it expires, signed with the key.
    fn challenge(&self) -> Reply {
        let mut issued = self.issued.lock();
        if issued.open.len() >= ALTCHA_OPEN_LIMIT {
            return Reply::error(StatusCode::SERVICE_UNAVAILABLE, "too_many_open_challenges");
        }
        let place = issued.count;
        issued.count += 1;
        let derived = hmac_sha256(&self.key, format!("challenge {place}").as_bytes());
        let (salt_bytes, number_bytes) = derived.split_at(12);
        let expires = clock::unix_seconds() + ALTCHA_LIFETIME_SECONDS;
        let salt = format!("{}?expires={expires}", HEXLOWER.encode(salt_bytes));
        let mut number = [0; 8];
        number.copy_from_slice(&number_bytes[..8]);
        let number = u64::from_be_bytes(number) % (self.max_number.saturating_add(1));
        let challenge = HEXLOWER.encode(&Sha256::digest(format!("{salt}{number}").as_bytes()));
        let signature = HEXLOWER.encode(&hmac_sha256(&self.key, challenge.as_bytes()));
        issued.open.insert(salt.clone());
        Reply::json(
            StatusCode::OK,
            &serde_json::json!({
                "algorithm": "SHA-256",
                "challenge": challenge,
                "maxnumber": self.max_number,
                "salt": salt,
                "signature": signature,
            }),
        )
    }

    /// The verdict of the widget's `payload`: its solution of a challenge
    /// this copy issued, signed, unexpired and not accepted before.
    fn verify(&self, payload: Option<String>) -> Verdict {
        let failed = |reason| Verdict::new(Vendor::Altcha, Outcome::Failed, reason);
        let Some(payload) = payload.filter(|payload| !payload.is_empty()) else {
            return failed("missing_token");
        };
        let Some(solution) = BASE64
            .decode(payload.as_bytes())
            .ok()
            .and_then(|json| serde_json::from_slice::<AltchaSolution>(&json).ok())
        else {
            return failed("malformed_solution");
        };
        if solution.algorithm != "SHA-256" {
            return failed("malformed_solution");
        }
        let signature = HEXLOWER.encode(&hmac_sha256(&self.key, solution.challenge.as_bytes()));
        if !same_secret(signature.as_bytes(), solution.signature.as_bytes()) {
            return failed("bad_signature");
        }
        let hash = Sha256::digest(format!("{}{}", solution.salt, solution.number).as_bytes());
        if HEXLOWER.encode(&hash) != solution.challenge {
            return failed("wrong_solution");
        }
        let expires = solution
            .salt
            .split_once("?expires=")
            .and_then(|(_, expires)| expires.parse::<u64>().ok());
        if expires.is_none_or(|expires| expires <= clock::unix_seconds()) {
            return failed("expired");
        }
        if !self.issued.lock().open.remove(&solution.salt) {
            return failed("unknown_challenge");
        }
        Verdict::passed(Vendor::Altcha)
    }
}

/// The verdict of Anubis's cookie on a request: signed with `key` and
/// unexpired, or the request never came through Anubis.
fn anubis(key: &[u8], headers: &HeaderMap) -> Verdict {
    let failed = |reason| Verdict::new(Vendor::Anubis, Outcome::Failed, reason);
    let token = headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|cookies| exchange::cookie(cookies, ANUBIS_COOKIE));
    let Some(token) = token else {
        return Verdict::new(Vendor::Anubis, Outcome::Inconclusive, "not_through_anubis");
    };
    let Some(claims) = hs512_claims(key, token) else {
        return failed("invalid_cookie");
    };
    let now = i64::try_from(clock::unix_seconds()).unwrap_or(i64::MAX);
    if claims.exp <= now || claims.nbf.is_some_and(|nbf| nbf > now) {
        return failed("expired_cookie");
    }
    Verdict::passed(Vendor::Anubis)
}

/// The claims of `token`, a JSON Web Token, when its header names HS512
/// and its signature is `key`'s.
fn hs512_claims(key: &[u8], token: &str) -> Option<AnubisClaims> {
    #[derive(Deserialize)]
    struct Header {
        alg: String,
    }
    let mut parts = token.split('.');
    let (header, payload, signature) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let decode = |part: &str| BASE64URL_NOPAD.decode(part.as_bytes()).ok();
    let alg = serde_json::from_slice::<Header>(&decode(header)?).ok()?.alg;
    if alg != "HS512" {
        return None;
    }
    let mut mac =
        Hmac::<Sha512>::new_from_slice(key).expect("invariant: HMAC takes a key of any length");
    mac.update(format!("{header}.{payload}").as_bytes());
    mac.verify_slice(&decode(signature)?).ok()?;
    serde_json::from_slice(&decode(payload)?).ok()
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).expect("invariant: HMAC takes a key of any length");
    mac.update(message);
    mac.finalize().into_bytes().to_vec()
}

/// Whether `sent` is `secret`, compared in a time that does not tell how
/// much of it matched: their digests are.
fn same_secret(sent: &[u8], secret: &[u8]) -> bool {
    Sha256::digest(sent) == Sha256::digest(secret)
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

fn method_not_allowed() -> Reply {
    Reply::error(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed")
}

/// ALTCHA's widget, which fetches its challenge from the copy.
const ALTCHA_WIDGET: &str = r#"<script async defer src="https://cdn.jsdelivr.net/npm/altcha@1.0.0/dist/altcha.min.js" type="module"></script><altcha-widget challengeurl="/altcha/challenge"></altcha-widget>"#;

/// A vendor's page: its widget, if any, in the form that posts what it
/// found to `/<vendor>/verify`.
fn page(check: &Check) -> Reply {
    let name = check.vendor().name();
    let widget = match check {
        Check::Siteverify(siteverify) => siteverify.vendor.widget(&siteverify.site_key),
        Check::Altcha(_) => ALTCHA_WIDGET.to_owned(),
        Check::Anubis { .. } | Check::Edge(_) => String::new(),
    };
    let mut page = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{name}</title></head><body>"
    );
    let _ = write!(
        page,
        r#"<form method="post" action="/{name}/verify">{widget}<button type="submit">Continue</button></form></body></html>"#
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
