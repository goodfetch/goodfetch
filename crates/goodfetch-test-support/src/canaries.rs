//! Secret canaries (ADR 0067): recognisable fake secrets a test plants,
//! one of each kind of secret Goodfetch holds, and the search that finds
//! each one in an output raw and in Goodfetch's encodings (JSON-escaped,
//! URL-encoded, base64 and hex), naming the output and the byte offset of
//! every hit, so a leak is found exactly. A copy where the test planted the
//! value is told apart from a leak.
//!
//! - `Canaries::plant(kind)` draws a fresh value from the test's seed; the
//!   test hands it to the code under test, such as through a Secret
//!   provider's real interface, and records where that keeps it with
//!   `record_planted`.
//! - `Canaries::plant_through(&proving_ground, kind)` plants a typed
//!   password, a TOTP seed, a Session cookie or an endpoint token through
//!   the Proving ground's `canaries` login flows, and `sightings` reads every
//!   request that carried one, each marked planted where its flow sends it.
//! - `gitleaks_rule()` is the `gitleaks` configuration with one rule per
//!   kind, which `xtask codegen` commits to
//!   `test-support/fixtures/canaries/gitleaks.toml` for CI's output scan.
//!
//! Every value is made only of ASCII; a kind whose value may hold any
//! character ends with `"\+/=&`, which JSON escaping and URL encoding both
//! change, so every encoding is its own search. A canary's `Debug` form and
//! every hit name it by ID, never by value.
#![expect(
    clippy::disallowed_methods,
    reason = "test support reads outputs and calls the Proving ground outside any async worker (ADR 0062)"
)]

use std::fmt::{self, Write as _};
use std::io;
use std::path::{Path, PathBuf};

use chacha20::ChaCha20Rng;
use chacha20::rand_core::Rng as _;
use serde::{Deserialize, Serialize};

use crate::proving_ground::ProvingGround;
use crate::seeded;

/// Every value's mark, which the `gitleaks` rule's keyword is.
const MARK: &str = "gfcanary";

/// The end of a value that may hold any character: JSON escaping changes
/// `"` and `\`, URL encoding every one of them.
const HARD_TAIL: &str = "\"\\+/=&";

/// The TOTP seed's mark: a seed is base32, whose alphabet holds `GFCANARY`.
const TOTP_MARK: &str = "GFCANARYTOTP";

/// A canary with each searched form of its value.
type Searched<'a> = (&'a Canary, Vec<(Encoding, Vec<u8>)>);

/// RFC 4648's base32 alphabet.
const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// The length, in characters, under which a value is short (spec #564's
/// short-value rule, ADR 0067): the engine masks it only where it was typed
/// and in what it keeps, so a page read may hold it by chance.
pub const SHORT_VALUE: usize = 8;

/// The characters of a short canary's value, which hex and base64 text
/// rarely hold side by side: lowercase letters but `l`.
const SHORT_ALPHABET: &[u8; 25] = b"abcdefghijkmnopqrstuvwxyz";

/// The end of a short value that may hold any character.
const SHORT_HARD_TAIL: &str = "\"\\+";

/// The kinds of secret a canary stands for (ADR 0067).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    /// A password a tool types into a login form.
    TypedPassword,
    /// A TOTP seed, base32 as RFC 6238's authenticators take it.
    TotpSeed,
    /// A one-time code a code source reads, such as one emailed or texted.
    OneTimeCode,
    /// The password of an upstream proxy.
    ProxyPassword,
    /// The password `session.http_credentials` answers a site's HTTP
    /// authentication with.
    HttpAuthenticationPassword,
    /// A header or JSON value `page.request` or a route sends.
    HeaderValue,
    /// A value a `caller:` Secret reference's resolver returns.
    CallerAnswer,
    /// A Model provider's API key.
    ModelApiKey,
    /// A webhook's signing secret.
    WebhookSigningSecret,
    /// A Sink's credential.
    SinkCredential,
    /// A Session cookie a site sets.
    SessionCookie,
    /// A token an endpoint takes.
    EndpointToken,
    /// A run's data key, 32 bytes (ADR 0015).
    RunDataKey,
}

impl SecretKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 13] = [
        Self::TypedPassword,
        Self::TotpSeed,
        Self::OneTimeCode,
        Self::ProxyPassword,
        Self::HttpAuthenticationPassword,
        Self::HeaderValue,
        Self::CallerAnswer,
        Self::ModelApiKey,
        Self::WebhookSigningSecret,
        Self::SinkCredential,
        Self::SessionCookie,
        Self::EndpointToken,
        Self::RunDataKey,
    ];

    /// The kind's name in snake case, as the Proving ground and the shared
    /// fixtures write it.
    pub fn name(self) -> &'static str {
        match self {
            Self::TypedPassword => "typed_password",
            Self::TotpSeed => "totp_seed",
            Self::OneTimeCode => "one_time_code",
            Self::ProxyPassword => "proxy_password",
            Self::HttpAuthenticationPassword => "http_authentication_password",
            Self::HeaderValue => "header_value",
            Self::CallerAnswer => "caller_answer",
            Self::ModelApiKey => "model_api_key",
            Self::WebhookSigningSecret => "webhook_signing_secret",
            Self::SinkCredential => "sink_credential",
            Self::SessionCookie => "session_cookie",
            Self::EndpointToken => "endpoint_token",
            Self::RunDataKey => "run_data_key",
        }
    }

    /// The kind in words, as the glossary writes it.
    fn label(self) -> &'static str {
        match self {
            Self::TypedPassword => "typed password",
            Self::TotpSeed => "TOTP seed",
            Self::OneTimeCode => "one-time code",
            Self::ProxyPassword => "proxy password",
            Self::HttpAuthenticationPassword => "HTTP authentication password",
            Self::HeaderValue => "header or JSON value",
            Self::CallerAnswer => "`caller:` answer",
            Self::ModelApiKey => "model API key",
            Self::WebhookSigningSecret => "webhook signing secret",
            Self::SinkCredential => "Sink credential",
            Self::SessionCookie => "Session cookie",
            Self::EndpointToken => "endpoint token",
            Self::RunDataKey => "run data key",
        }
    }

    /// The regular expression every value of the kind matches, planted here
    /// or by the Proving ground, without the hard tail; JSON escaping and URL
    /// encoding leave it unchanged.
    pub fn pattern(self) -> String {
        if self == Self::TotpSeed {
            format!("{TOTP_MARK}[A-Z2-7]{{20}}")
        } else {
            let (tag, hex) = self.shape();
            format!("{MARK}-{tag}-[0-9a-f]{{{hex}}}")
        }
    }

    /// Whether the Proving ground's `canaries` family has a login flow that
    /// plants the kind.
    pub fn has_login_flow(self) -> bool {
        matches!(
            self,
            Self::TypedPassword | Self::TotpSeed | Self::SessionCookie | Self::EndpointToken
        )
    }

    /// The tag after the mark and the number of hex digits after it, for
    /// every kind but the TOTP seed.
    fn shape(self) -> (&'static str, usize) {
        match self {
            Self::TypedPassword => ("password", 16),
            Self::OneTimeCode => ("code", 16),
            Self::ProxyPassword => ("proxy-password", 16),
            Self::HttpAuthenticationPassword => ("http-password", 16),
            Self::HeaderValue => ("header-value", 16),
            Self::CallerAnswer => ("caller-answer", 16),
            Self::WebhookSigningSecret => ("webhook-secret", 16),
            Self::SinkCredential => ("sink-credential", 16),
            Self::ModelApiKey => ("model-key", 32),
            Self::SessionCookie => ("session", 32),
            Self::EndpointToken => ("token", 32),
            // 17 bytes of mark and tag and 15 digits: a 32-byte key.
            Self::RunDataKey => ("run-key", 15),
            Self::TotpSeed => ("totp", 0),
        }
    }

    /// Whether the kind's value may hold any character, so it ends with the
    /// hard tail.
    fn takes_any_character(self) -> bool {
        matches!(
            self,
            Self::TypedPassword
                | Self::ProxyPassword
                | Self::HttpAuthenticationPassword
                | Self::HeaderValue
                | Self::CallerAnswer
                | Self::WebhookSigningSecret
                | Self::SinkCredential
        )
    }
}

/// The encodings a value is searched in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    /// The value's own bytes.
    Raw,
    /// Inside a JSON string: `"`, `\` and control characters escaped.
    JsonEscaped,
    /// Percent-encoded, in uppercase hex: every byte but `A-Z a-z 0-9 - . _
    /// ~`; as a browser encodes a form, keeping `*` and writing a space `+`;
    /// or as JavaScript's `encodeURIComponent` does, keeping `! ~ * ' ( )`
    /// too.
    UrlEncoded,
    /// Base64, standard or URL-safe, at any alignment inside a longer
    /// encoded run (such as a password in a Basic `Authorization` header).
    Base64,
    /// Hex, lowercase or uppercase.
    Hex,
}

/// Whether a hit is a copy where the value was planted or a leak.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// Where the test planted it, or where its Proving ground flow sends it.
    Planted,
    /// Anywhere else.
    Leaked,
}

/// The account a Proving ground plant hands out, which logs in to its
/// task's flow.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct PlantAccount {
    /// The account's username.
    pub username: String,
    /// Its password, the canary itself for a typed password.
    pub password: String,
    /// Its TOTP seed in base32, the canary itself for a TOTP seed.
    #[serde(default)]
    pub totp_secret: Option<String>,
}

impl fmt::Debug for PlantAccount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlantAccount")
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// A plant through the Proving ground: its ID there, its task and account.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ProvingGroundPlant {
    id: String,
    task: String,
    account: Option<PlantAccount>,
}

/// One planted canary.
#[derive(Clone, PartialEq, Eq)]
pub struct Canary {
    id: String,
    kind: SecretKind,
    value: String,
    planted_in: Vec<String>,
    proving_ground: Option<ProvingGroundPlant>,
}

impl Canary {
    /// A canary whose value came from elsewhere, such as a shared fixture,
    /// for `Canaries::add`.
    pub fn known(id: &str, kind: SecretKind, value: &str) -> Self {
        Self {
            id: id.to_owned(),
            kind,
            value: value.to_owned(),
            planted_in: Vec::new(),
            proving_ground: None,
        }
    }

    /// Its ID, unique in its set, which hits name it by.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Its kind.
    pub fn kind(&self) -> SecretKind {
        self.kind
    }

    /// Its value, which the test hands to the code under test.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Whether its value has fewer than [`SHORT_VALUE`] characters, so a search
    /// leaves it out of a page read ([`Output::Read`]).
    pub fn is_short(&self) -> bool {
        self.value.chars().count() < SHORT_VALUE
    }

    /// The Proving ground task whose flow uses it, when planted through it.
    pub fn task(&self) -> Option<&str> {
        self.proving_ground
            .as_ref()
            .map(|plant| plant.task.as_str())
    }

    /// The account that logs in to its flow, when its Proving ground plant
    /// handed one out.
    pub fn account(&self) -> Option<&PlantAccount> {
        self.proving_ground
            .as_ref()
            .and_then(|plant| plant.account.as_ref())
    }

    /// Each searched form of the value, with its encoding; a form an earlier
    /// encoding already gives is left out.
    fn forms(&self) -> Vec<(Encoding, Vec<u8>)> {
        let value = self.value.as_bytes();
        let mut forms: Vec<(Encoding, Vec<u8>)> = Vec::new();
        let mut add = |encoding: Encoding, form: Vec<u8>| {
            if !form.is_empty() && forms.iter().all(|(_, earlier)| *earlier != form) {
                forms.push((encoding, form));
            }
        };
        add(Encoding::Raw, value.to_vec());
        add(
            Encoding::JsonEscaped,
            json_escaped(&self.value).into_bytes(),
        );
        add(
            Encoding::UrlEncoded,
            url_encoded(value, UNRESERVED).into_bytes(),
        );
        add(
            Encoding::UrlEncoded,
            url_encoded(value, FORM_KEPT)
                .replace("%20", "+")
                .into_bytes(),
        );
        add(
            Encoding::UrlEncoded,
            url_encoded(value, URI_COMPONENT_KEPT).into_bytes(),
        );
        for alphabet in [
            &data_encoding::BASE64_NOPAD,
            &data_encoding::BASE64URL_NOPAD,
        ] {
            for alignment in 0..3 {
                add(Encoding::Base64, base64_at(alphabet, value, alignment));
            }
        }
        add(
            Encoding::Hex,
            data_encoding::HEXLOWER.encode(value).into_bytes(),
        );
        add(
            Encoding::Hex,
            data_encoding::HEXUPPER.encode(value).into_bytes(),
        );
        forms
    }
}

impl fmt::Debug for Canary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Canary")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("planted_in", &self.planted_in)
            .finish_non_exhaustive()
    }
}

/// An output a search reads.
#[derive(Clone, Copy, Debug)]
pub enum Output<'a> {
    /// A file, or a folder whose every file is read, in path order; each
    /// hit names the file's path.
    Path(&'a Path),
    /// Bytes the test holds, such as a captured `repr`, named for the hits.
    Bytes {
        /// The name hits give it.
        name: &'a str,
        /// Its bytes.
        bytes: &'a [u8],
    },
    /// A page read the test holds, such as a call's result or an event's
    /// text: searched for every canary but a short one, which the engine
    /// masks in a read only where it was typed (spec #564).
    Read {
        /// The name hits give it.
        name: &'a str,
        /// Its bytes.
        bytes: &'a [u8],
    },
}

/// One place an output holds a canary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// The canary's ID.
    pub canary: String,
    /// Its kind.
    pub kind: SecretKind,
    /// The encoding it was found in.
    pub encoding: Encoding,
    /// The output: a file's path, or the name given to bytes.
    pub output: String,
    /// The byte offset in the output where the encoded value starts.
    pub offset: usize,
    /// Whether the output is where the canary was planted.
    pub placement: Placement,
}

/// One request the Proving ground saw carrying a canary planted through it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sighting {
    /// The canary's ID in its set.
    pub canary: String,
    /// Its kind.
    pub kind: SecretKind,
    /// The host name the request went to.
    pub host: String,
    /// Its target, at most 256 bytes.
    pub target: String,
    /// The part of the request that carried it: `target`, `body` or
    /// `header:<lowercase name>`.
    pub part: String,
    /// Planted when that is where the canary's flow sends it.
    pub placement: Placement,
}

/// Why a plant or a search failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CanaryError {
    /// An output could not be read.
    #[error("reading {path} for Secret canaries")]
    Read {
        /// The path.
        path: PathBuf,
        /// The failure.
        #[source]
        source: io::Error,
    },
    /// The Proving ground has no login flow for the kind.
    #[error("the Proving ground has no login flow that plants a {}", .0.name())]
    NoLoginFlow(SecretKind),
    /// The Proving ground did not answer as its `canaries` family does.
    #[error("the Proving ground's /canaries answered: {0}")]
    ProvingGround(String),
}

/// A test's Secret canaries: every one planted, with where.
pub struct Canaries {
    stream: ChaCha20Rng,
    planted: Vec<Canary>,
}

impl fmt::Debug for Canaries {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Canaries")
            .field("planted", &self.planted)
            .finish_non_exhaustive()
    }
}

impl Canaries {
    /// An empty set whose values are drawn from `seed`, the test's own
    /// (`TestEnv::seed`), so a seed always plants the same values.
    pub fn new(seed: u64) -> Self {
        Self {
            stream: seeded::stream(seed, b"canaries"),
            planted: Vec::new(),
        }
    }

    /// Plants a canary of `kind`: a value no earlier plant of the set
    /// returned.
    pub fn plant(&mut self, kind: SecretKind) -> &Canary {
        let value = loop {
            let value = self.draw(kind);
            if self.planted.iter().all(|canary| canary.value != value) {
                break value;
            }
        };
        let id = self.next_id();
        self.push(Canary {
            id,
            kind,
            value,
            planted_in: Vec::new(),
            proving_ground: None,
        })
    }

    /// Plants a short canary of `kind`: seven characters, under [`SHORT_VALUE`],
    /// lowercase letters, the last three `"\+` for a kind whose value may
    /// hold any character. It bears no mark, so `gitleaks` knows none, and
    /// a search leaves it out of a page read.
    pub fn plant_short(&mut self, kind: SecretKind) -> &Canary {
        let tail = if kind.takes_any_character() {
            SHORT_HARD_TAIL
        } else {
            ""
        };
        let value = loop {
            let random: String = (0..SHORT_VALUE - 1 - tail.len())
                .map(|_| {
                    char::from(
                        SHORT_ALPHABET[usize::try_from(self.stream.next_u32() % 25)
                            .expect("invariant: below 25 fits a usize")],
                    )
                })
                .collect();
            let value = format!("{random}{tail}");
            if self.planted.iter().all(|canary| canary.value != value) {
                break value;
            }
        };
        let id = self.next_id();
        self.push(Canary {
            id,
            kind,
            value,
            planted_in: Vec::new(),
            proving_ground: None,
        })
    }

    /// Plants a canary of `kind` through the Proving ground's login flow for
    /// it, which draws the value and records where it sends it.
    ///
    /// # Errors
    ///
    /// `NoLoginFlow` for a kind without one, `ProvingGround` when its answer
    /// is not a plant.
    pub fn plant_through(
        &mut self,
        proving_ground: &ProvingGround,
        kind: SecretKind,
    ) -> Result<&Canary, CanaryError> {
        #[derive(Deserialize)]
        struct Planted {
            id: String,
            task: String,
            value: String,
            #[serde(default)]
            account: Option<PlantAccount>,
        }

        if !kind.has_login_flow() {
            return Err(CanaryError::NoLoginFlow(kind));
        }
        let mut response = ureq::post(format!("http://{}/canaries", proving_ground.address()))
            .send_json(serde_json::json!({ "kind": kind.name() }))
            .map_err(|error| CanaryError::ProvingGround(error.to_string()))?;
        let planted: Planted = response
            .body_mut()
            .read_json()
            .map_err(|error| CanaryError::ProvingGround(error.to_string()))?;
        let id = self.next_id();
        Ok(self.push(Canary {
            id,
            kind,
            value: planted.value,
            planted_in: Vec::new(),
            proving_ground: Some(ProvingGroundPlant {
                id: planted.id,
                task: planted.task,
                account: planted.account,
            }),
        }))
    }

    /// Adds a canary whose value came from elsewhere.
    ///
    /// # Panics
    ///
    /// If the set already has its ID or its value.
    pub fn add(&mut self, canary: Canary) -> &Canary {
        assert!(
            self.planted
                .iter()
                .all(|planted| planted.id != canary.id && planted.value != canary.value),
            "{} is already in the set",
            canary.id
        );
        self.push(canary)
    }

    /// Records that the output named `output` (a file's path as a search
    /// names it, or the name given to bytes) holds canary `id` because the
    /// test planted it there, so its hits there are planted copies.
    ///
    /// # Panics
    ///
    /// If the set has no canary `id`.
    pub fn record_planted(&mut self, id: &str, output: &str) {
        let canary = self
            .planted
            .iter_mut()
            .find(|canary| canary.id == id)
            .unwrap_or_else(|| panic!("the set has no canary {id}"));
        canary.planted_in.push(output.to_owned());
    }

    /// The canary `id`.
    pub fn canary(&self, id: &str) -> Option<&Canary> {
        self.planted.iter().find(|canary| canary.id == id)
    }

    /// Every canary, in plant order.
    pub fn iter(&self) -> impl Iterator<Item = &Canary> {
        self.planted.iter()
    }

    /// Every place `outputs` hold a canary of the set, in any encoding: in
    /// output order, then by offset, then in plant and encoding order.
    ///
    /// # Errors
    ///
    /// `Read` when a path cannot be read.
    pub fn search(&self, outputs: &[Output<'_>]) -> Result<Vec<Hit>, CanaryError> {
        let forms: Vec<Searched<'_>> = self
            .planted
            .iter()
            .map(|canary| (canary, canary.forms()))
            .collect();
        let mut hits = Vec::new();
        for output in outputs {
            match output {
                Output::Bytes { name, bytes } => search_bytes(&forms, name, bytes, &mut hits),
                Output::Read { name, bytes } => {
                    let long: Vec<Searched<'_>> = forms
                        .iter()
                        .filter(|(canary, _)| !canary.is_short())
                        .cloned()
                        .collect();
                    search_bytes(&long, name, bytes, &mut hits);
                }
                Output::Path(path) => {
                    for file in files(path)? {
                        let bytes = std::fs::read(&file).map_err(|source| CanaryError::Read {
                            path: file.clone(),
                            source,
                        })?;
                        search_bytes(&forms, &file.display().to_string(), &bytes, &mut hits);
                    }
                }
            }
        }
        Ok(hits)
    }

    /// The hits of `search` that are leaks.
    ///
    /// # Errors
    ///
    /// As `search`.
    pub fn leaks(&self, outputs: &[Output<'_>]) -> Result<Vec<Hit>, CanaryError> {
        let mut hits = self.search(outputs)?;
        hits.retain(|hit| hit.placement == Placement::Leaked);
        Ok(hits)
    }

    /// Every request the Proving ground saw carrying a canary this set
    /// planted through it, in the order it saw them.
    ///
    /// # Errors
    ///
    /// `ProvingGround` when its `/canaries` document cannot be read.
    pub fn sightings(&self, proving_ground: &ProvingGround) -> Result<Vec<Sighting>, CanaryError> {
        #[derive(Deserialize)]
        struct Document {
            canaries: Vec<Listed>,
        }
        #[derive(Deserialize)]
        struct Listed {
            id: String,
            sightings: Vec<Seen>,
        }
        #[derive(Deserialize)]
        struct Seen {
            host: String,
            target: String,
            part: String,
            expected: bool,
        }

        let mut response = ureq::get(format!("http://{}/canaries", proving_ground.address()))
            .call()
            .map_err(|error| CanaryError::ProvingGround(error.to_string()))?;
        let document: Document = response
            .body_mut()
            .read_json()
            .map_err(|error| CanaryError::ProvingGround(error.to_string()))?;
        let mut sightings = Vec::new();
        for listed in document.canaries {
            let Some(canary) = self.planted.iter().find(|canary| {
                canary
                    .proving_ground
                    .as_ref()
                    .is_some_and(|plant| plant.id == listed.id)
            }) else {
                continue;
            };
            sightings.extend(listed.sightings.into_iter().map(|seen| Sighting {
                canary: canary.id.clone(),
                kind: canary.kind,
                host: seen.host,
                target: seen.target,
                part: seen.part,
                placement: if seen.expected {
                    Placement::Planted
                } else {
                    Placement::Leaked
                },
            }));
        }
        Ok(sightings)
    }

    fn next_id(&self) -> String {
        format!("canary-{}", self.planted.len() + 1)
    }

    fn push(&mut self, canary: Canary) -> &Canary {
        self.planted.push(canary);
        self.planted
            .last()
            .expect("invariant: a canary was just pushed")
    }

    /// A value of `kind` drawn from the set's stream.
    fn draw(&mut self, kind: SecretKind) -> String {
        if kind == SecretKind::TotpSeed {
            let random: String = (0..20)
                .map(|_| {
                    char::from(
                        BASE32[usize::try_from(self.stream.next_u32() % 32)
                            .expect("invariant: below 32 fits a usize")],
                    )
                })
                .collect();
            return format!("{TOTP_MARK}{random}");
        }
        let (tag, digits) = kind.shape();
        let random = format!(
            "{:016x}{:016x}",
            self.stream.next_u64(),
            self.stream.next_u64()
        );
        let tail = if kind.takes_any_character() {
            HARD_TAIL
        } else {
            ""
        };
        format!("{MARK}-{tag}-{}{tail}", &random[..digits])
    }
}

/// Records each place `bytes`, the output `name`, holds one of `forms`.
fn search_bytes(forms: &[Searched<'_>], name: &str, bytes: &[u8], hits: &mut Vec<Hit>) {
    let mut found = Vec::new();
    for (order, (canary, canary_forms)) in forms.iter().enumerate() {
        let placement = if canary.planted_in.iter().any(|planted| planted == name) {
            Placement::Planted
        } else {
            Placement::Leaked
        };
        for (encoding, form) in canary_forms {
            for offset in occurrences(bytes, form) {
                found.push((offset, order, *encoding, canary, placement));
            }
        }
    }
    found.sort_by_key(|(offset, order, encoding, _, _)| (*offset, *order, *encoding));
    found.dedup_by_key(|(offset, order, encoding, _, _)| (*offset, *order, *encoding));
    hits.extend(
        found
            .into_iter()
            .map(|(offset, _, encoding, canary, placement)| Hit {
                canary: canary.id.clone(),
                kind: canary.kind,
                encoding,
                output: name.to_owned(),
                offset,
                placement,
            }),
    );
}

/// Every offset where `bytes` holds `form`.
fn occurrences<'a>(bytes: &'a [u8], form: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    bytes
        .windows(form.len())
        .enumerate()
        .filter(move |(_, window)| *window == form)
        .map(|(offset, _)| offset)
}

/// `path` itself when it is a file, else every file under it, in path
/// order.
fn files(path: &Path) -> Result<Vec<PathBuf>, CanaryError> {
    let read = |path: &Path, source| CanaryError::Read {
        path: path.to_owned(),
        source,
    };
    let metadata = std::fs::metadata(path).map_err(|source| read(path, source))?;
    if !metadata.is_dir() {
        return Ok(vec![path.to_owned()]);
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
        .map_err(|source| read(path, source))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()
        .map_err(|source| read(path, source))?;
    entries.sort();
    let mut files = Vec::new();
    for entry in entries {
        files.extend(self::files(&entry)?);
    }
    Ok(files)
}

/// `value` as it stands inside a JSON string.
fn json_escaped(value: &str) -> String {
    let quoted = serde_json::to_string(value).expect("invariant: a string serializes to JSON");
    quoted[1..quoted.len() - 1].to_owned()
}

/// The marks RFC 3986 leaves unreserved.
const UNRESERVED: &[u8] = b"-._~";

/// The marks a browser keeps as it encodes a form
/// (`application/x-www-form-urlencoded`), which writes a space `+`.
const FORM_KEPT: &[u8] = b"*-._";

/// The marks JavaScript's `encodeURIComponent` keeps.
const URI_COMPONENT_KEPT: &[u8] = b"-_.!~*'()";

/// `value` percent-encoded, every byte but the ASCII letters, digits and
/// `kept`.
fn url_encoded(value: &[u8], kept: &[u8]) -> String {
    let mut encoded = String::new();
    for byte in value {
        if byte.is_ascii_alphanumeric() || kept.contains(byte) {
            encoded.push(char::from(*byte));
        } else {
            write!(encoded, "%{byte:02X}").expect("invariant: writing to a String succeeds");
        }
    }
    encoded
}

/// The base64 characters that encode `value` alone when it starts
/// `alignment` bytes into a group of three: the characters shared with the
/// bytes before or after it are cut.
fn base64_at(alphabet: &data_encoding::Encoding, value: &[u8], alignment: usize) -> Vec<u8> {
    let mut padded = vec![0; alignment];
    padded.extend_from_slice(value);
    let encoded = alphabet.encode(&padded).into_bytes();
    let start = [0, 2, 3][alignment];
    let end = padded.len() / 3 * 4 + padded.len() % 3;
    if start >= end {
        return Vec::new();
    }
    encoded[start..end].to_vec()
}

/// The `gitleaks` configuration that finds every kind's canaries, raw and
/// in every encoding: the rule matches the value's mark and random part,
/// which JSON escaping and URL encoding leave unchanged, and `gitleaks`
/// decodes base64 and hex itself. The shared fixture artifacts that hold
/// canaries on purpose are allowed, so a scan of commits passes over them.
pub fn gitleaks_rule() -> String {
    let mut rule = String::from(
        "# Generated by `xtask codegen` from goodfetch-test-support's Secret canary\n\
         # kinds (ADR 0067); never edited by hand. One rule per kind finds its\n\
         # canaries raw, JSON-escaped and URL-encoded, and gitleaks' decoding finds\n\
         # them in base64 and hex. CI's gitleaks step reads it.\n\
         title = \"Goodfetch Secret canaries\"\n",
    );
    for kind in SecretKind::ALL {
        write!(
            rule,
            "\n[[rules]]\nid = \"goodfetch-secret-canary-{}\"\n\
             description = \"Goodfetch Secret canary: {} (ADR 0067)\"\n\
             regex = '''{}'''\nkeywords = [\"{MARK}\"]\n",
            kind.name().replace('_', "-"),
            kind.label(),
            kind.pattern(),
        )
        .expect("invariant: writing to a String succeeds");
    }
    rule.push_str(
        "\n[[allowlists]]\n\
         description = \"The fixture artifacts that hold Secret canaries on purpose\"\n\
         paths = ['''^test-support/fixtures/canaries/artifacts/''']\n",
    );
    rule
}

#[cfg(test)]
mod tests;
