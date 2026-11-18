//! Web Bot Auth (ADRs 0021 and 0045; #1551): an `identified` Session signs
//! each request it sends with its Ed25519 key, by Cloudflare's
//! `web-bot-auth`, as RFC 9421's HTTP Message Signatures over the request's
//! `@authority` and its `Signature-Agent`, which names where a site reads
//! the key directory. The key is resolved once, when the Session opens,
//! from its Secret reference; it is never written anywhere, and dropping
//! the [`RequestSigner`] zeroes it.
//!
//! A key is made by [`new_key`] from the OS generator, and the key
//! directory a site fetches is exported by [`directory`]: each key's public
//! JSON Web Key named by its thumbprint, each proven by its own signature
//! of the directory's authority, tagged `http-message-signatures-directory`
//! (`signing-key create` and `signing-key directory`, ADR 0045).

use std::fmt;
use std::sync::PoisonError;
use std::time::Duration;

use data_encoding::{BASE64, BASE64URL_NOPAD};
use ed25519_dalek::{Signer as _, SigningKey};
use indexmap::IndexMap;
use secrecy::zeroize::{Zeroize as _, Zeroizing};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use url::Url;
use web_bot_auth::ImplementationError;
use web_bot_auth::components::{
    CoveredComponent, DerivedComponent, HTTPField, HTTPFieldParametersSet,
};
use web_bot_auth::keyring::{Algorithm, Thumbprintable};
use web_bot_auth::message_signatures::{GenerateSignature, MessageSigner, UnsignedMessage};

use crate::entropy::Entropy;
use crate::sync::Mutex;

#[cfg(test)]
mod tests;

/// The tag a Web Bot Auth signature carries.
const TAG: &str = "web-bot-auth";

/// The tag of a key directory's proof that it holds each key.
const DIRECTORY_TAG: &str = "http-message-signatures-directory";

/// The label of the one signature a request carries.
const LABEL: &str = "sig1";

/// How long after it is made a signature stays valid: long enough for the
/// request to reach the site, short enough that a replay soon fails.
const VALID_FOR: Duration = Duration::from_secs(60);

/// The random bytes of each signature's nonce, as Cloudflare's examples
/// draw them.
const NONCE_BYTES: usize = 64;

/// The headers a signed request carries, in the order it sends them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedHeaders {
    /// `Signature-Agent`: where a site reads the key directory, as a
    /// structured-field string.
    pub signature_agent: String,
    /// `Signature-Input`: what the signature covers, and its parameters.
    pub signature_input: String,
    /// `Signature`: the signature itself.
    pub signature: String,
}

impl SignedHeaders {
    /// The names of the headers, which a signed request carries once each:
    /// any of them the request had is replaced.
    pub const NAMES: [&str; 3] = ["Signature-Agent", "Signature-Input", "Signature"];

    /// Each header's name and value, in [`Self::NAMES`]' order.
    #[must_use]
    pub fn into_entries(self) -> [(&'static str, String); 3] {
        let [agent, input, signature] = Self::NAMES;
        [
            (agent, self.signature_agent),
            (input, self.signature_input),
            (signature, self.signature),
        ]
    }
}

/// Why a key or a key directory's location cannot sign; it never quotes the
/// key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// The Secret's value is not a JSON Web Key.
    #[error("the signing key is not a JSON Web Key with `kty`, `crv`, `d` and `x`")]
    NotJwk,
    /// The key is not an Ed25519 one.
    #[error(
        "the signing key is not an Ed25519 key: its `kty` must be `OKP` and its `crv` `Ed25519`"
    )]
    NotEd25519,
    /// `d` is not 32 bytes in unpadded Base64url.
    #[error("the signing key's `d` is not 32 bytes in unpadded Base64url")]
    BadPrivateKey,
    /// `x` is not the public key of `d`.
    #[error("the signing key's `x` is not the public key of its `d`")]
    MismatchedPublicKey,
    /// The key directory's location cannot be sent as a header string.
    #[error("the signature agent must be printable ASCII without `\"` or `\\`")]
    BadAgent,
}

/// The private JSON Web Key a Secret holds, borrowed from its value so
/// that `d` is never copied out of the zeroized Secret.
#[derive(Deserialize)]
struct PrivateJwk<'a> {
    kty: &'a str,
    crv: &'a str,
    d: &'a str,
    x: &'a str,
}

/// An Ed25519 key, which the `web-bot-auth` signer calls.
struct Key(SigningKey);

impl GenerateSignature for Key {
    fn generate_signature(
        &self,
        algorithm: Algorithm,
        message: &[u8],
    ) -> Result<Vec<u8>, ImplementationError> {
        match algorithm {
            Algorithm::Ed25519 => Ok(self.0.sign(message).to_vec()),
            other => Err(ImplementationError::UnsupportedAlgorithm(other)),
        }
    }
}

/// What one signature covers, its label, and the members of the
/// `Signature-Input` and `Signature` fields it is given.
struct Covered {
    components: IndexMap<CoveredComponent, String>,
    label: String,
    input: String,
    signature: String,
}

impl Covered {
    /// A signature labelled `label` of `components`, not yet made.
    fn new(label: String, components: IndexMap<CoveredComponent, String>) -> Self {
        Self {
            components,
            label,
            input: String::new(),
            signature: String::new(),
        }
    }
}

impl UnsignedMessage for Covered {
    fn fetch_components_to_cover(&self) -> IndexMap<CoveredComponent, String> {
        self.components.clone()
    }

    fn register_header_contents(&mut self, signature_input: String, signature_header: String) {
        self.input = format!("{}={signature_input}", self.label);
        self.signature = format!("{}={signature_header}", self.label);
    }
}

/// Signs `covered` with `key`, named `key_id`, now, valid for `valid_for`,
/// tagged `tag`, with `nonce`.
fn sign(
    covered: &mut Covered,
    key: &Key,
    key_id: &str,
    nonce: String,
    tag: &str,
    valid_for: Duration,
) {
    #[expect(
        clippy::disallowed_methods,
        reason = "`web-bot-auth` dates a signature by the wall clock itself, which the site that checks it judges it by (ADR 0045)"
    )]
    MessageSigner {
        keyid: key_id.to_owned(),
        nonce,
        tag: tag.to_owned(),
    }
    .generate_signature_headers_content(covered, valid_for, Algorithm::Ed25519, key)
    .expect("invariant: what a signature covers is printable ASCII: the agent and the authority were checked, the key ID and the nonce are Base64, and the tag is a constant");
}

/// The Ed25519 key a private JSON Web Key holds, and its public key in
/// unpadded Base64url, checked against its `x`.
fn parsed(key: &SecretString) -> Result<(Key, String), KeyError> {
    let jwk: PrivateJwk<'_> =
        serde_json::from_str(key.expose_secret()).map_err(|_| KeyError::NotJwk)?;
    if jwk.kty != "OKP" || jwk.crv != "Ed25519" {
        return Err(KeyError::NotEd25519);
    }
    let mut private = [0; 32];
    let decoded = BASE64URL_NOPAD
        .decode_len(jwk.d.len())
        .ok()
        .filter(|length| *length == private.len())
        .and_then(|_| {
            BASE64URL_NOPAD
                .decode_mut(jwk.d.as_bytes(), &mut private)
                .ok()
        });
    let signing = decoded.map(|_| SigningKey::from_bytes(&private));
    private.zeroize();
    let signing = signing.ok_or(KeyError::BadPrivateKey)?;
    let public = BASE64URL_NOPAD.encode(signing.verifying_key().as_bytes());
    if public != jwk.x {
        return Err(KeyError::MismatchedPublicKey);
    }
    Ok((Key(signing), public))
}

/// The ID of the Ed25519 public key `x`, its RFC 7638 thumbprint.
fn key_id_of(x: &str) -> String {
    Thumbprintable::OKP {
        crv: "Ed25519".to_owned(),
        x: x.to_owned(),
    }
    .b64_thumbprint()
}

/// A key's public half as a key directory publishes it: its public JSON
/// Web Key, named by its thumbprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicKey {
    /// The key type, `OKP`.
    pub kty: &'static str,
    /// The curve, `Ed25519`.
    pub crv: &'static str,
    /// The public key, in unpadded Base64url.
    pub x: String,
    /// Its RFC 7638 thumbprint.
    pub kid: String,
}

/// The public half of the private JSON Web Key `key`, checked as a signer
/// checks it.
///
/// # Errors
///
/// As [`RequestSigner::new`] refuses a key.
pub fn public_key(key: &SecretString) -> Result<PublicKey, KeyError> {
    let (_, x) = parsed(key)?;
    Ok(PublicKey {
        kty: "OKP",
        crv: "Ed25519",
        kid: key_id_of(&x),
        x,
    })
}

/// The OS generator gave no random bytes.
#[derive(Debug, thiserror::Error)]
#[error("the OS generator gave no random bytes: {0}")]
pub struct NoRandomness(getrandom::Error);

/// A new Ed25519 key from the OS generator, as the private JSON Web Key a
/// Secret holds.
///
/// # Errors
///
/// When the OS generator gives no random bytes.
pub fn new_key() -> Result<SecretString, NoRandomness> {
    let mut private = Zeroizing::new([0_u8; 32]);
    getrandom::fill(private.as_mut_slice()).map_err(NoRandomness)?;
    let signing = SigningKey::from_bytes(&private);
    let d = Zeroizing::new(BASE64URL_NOPAD.encode(private.as_slice()));
    let x = BASE64URL_NOPAD.encode(signing.verifying_key().as_bytes());
    Ok(SecretString::from(format!(
        r#"{{"kty":"OKP","crv":"Ed25519","d":"{}","x":"{x}"}}"#,
        d.as_str()
    )))
}

/// Why a key directory cannot be exported; it never quotes a key.
#[derive(Debug, thiserror::Error)]
pub enum DirectoryError {
    /// The directory publishes no key.
    #[error("a key directory publishes at least one key")]
    NoKeys,
    /// The key at `index` cannot sign.
    #[error("key {index}: {error}")]
    Key {
        /// Its place among the keys, from 0.
        index: usize,
        /// Why it cannot sign.
        error: KeyError,
    },
    /// The authority is not a host and an optional port.
    #[error(
        "`{0}` is no authority: a host, and its port when it is not the scheme's own, such as `bot.example`"
    )]
    BadAuthority(String),
    /// No nonce could be drawn.
    #[error(transparent)]
    NoRandomness(#[from] NoRandomness),
}

/// A key directory as its origin serves it: the body, and the
/// `Signature-Input` and `Signature` fields of its response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directory {
    /// The JSON key set.
    pub body: String,
    /// One member per key, `sig1` first.
    pub signature_input: String,
    /// One member per key, in the same order.
    pub signature: String,
    /// The keys' IDs, in the order given.
    pub key_ids: Vec<String>,
    /// When the first of its signatures expires, in seconds since the Unix
    /// epoch.
    pub expires: i64,
}

/// The key directory of `keys`, private JSON Web Keys, served at
/// `authority`, each key's proof valid for `valid_for`.
///
/// # Errors
///
/// As [`DirectoryError`] says.
pub fn directory(
    keys: &[SecretString],
    authority: &str,
    valid_for: Duration,
) -> Result<Directory, DirectoryError> {
    if keys.is_empty() {
        return Err(DirectoryError::NoKeys);
    }
    let authority = checked_authority(authority)?;
    let mut published = Vec::with_capacity(keys.len());
    let (mut inputs, mut signatures, mut expires) = (Vec::new(), Vec::new(), i64::MAX);
    for (index, key) in keys.iter().enumerate() {
        let (key, x) = parsed(key).map_err(|error| DirectoryError::Key { index, error })?;
        let kid = key_id_of(&x);
        let mut covered = Covered::new(
            format!("sig{}", index + 1),
            IndexMap::from([(
                CoveredComponent::Derived(DerivedComponent::Authority { req: false }),
                authority.clone(),
            )]),
        );
        sign(
            &mut covered,
            &key,
            &kid,
            os_nonce()?,
            DIRECTORY_TAG,
            valid_for,
        );
        expires = expires.min(expiry(&covered.input));
        inputs.push(covered.input);
        signatures.push(covered.signature);
        published.push(PublicKey {
            kty: "OKP",
            crv: "Ed25519",
            x,
            kid,
        });
    }
    let key_ids = published.iter().map(|key| key.kid.clone()).collect();
    Ok(Directory {
        body: serde_json::json!({ "keys": published }).to_string(),
        signature_input: inputs.join(", "),
        signature: signatures.join(", "),
        key_ids,
        expires,
    })
}

/// `authority` as `@authority` covers it: in lower case, a host and an
/// optional port.
fn checked_authority(authority: &str) -> Result<String, DirectoryError> {
    let refused = || DirectoryError::BadAuthority(authority.to_owned());
    if authority.is_empty() || authority.contains(['/', '?', '#', '@', '\\']) {
        return Err(refused());
    }
    let url = Url::parse(&format!("http://{authority}/")).map_err(|_| refused())?;
    let host = url.host_str().ok_or_else(refused)?;
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}

/// The `expires` parameter of the `Signature-Input` member `input`, which
/// the signer always writes.
fn expiry(input: &str) -> i64 {
    input
        .split(';')
        .find_map(|parameter| parameter.strip_prefix("expires="))
        .and_then(|seconds| seconds.parse().ok())
        .expect("invariant: the signer writes `expires` as an integer")
}

/// A fresh nonce, [`NONCE_BYTES`] random bytes from the OS generator in
/// Base64.
fn os_nonce() -> Result<String, NoRandomness> {
    let mut bytes = [0; NONCE_BYTES];
    getrandom::fill(&mut bytes).map_err(NoRandomness)?;
    Ok(BASE64.encode(&bytes))
}

/// A Session's signer: its key, the key's ID and where a site reads the key
/// directory, with the Entropy each nonce is drawn from.
pub struct RequestSigner {
    key: Key,
    key_id: String,
    agent: String,
    entropy: Mutex<Entropy>,
}

impl fmt::Debug for RequestSigner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestSigner")
            .field("key_id", &self.key_id)
            .field("agent", &self.agent)
            .finish_non_exhaustive()
    }
}

impl RequestSigner {
    /// A signer with `key`, the Secret's value, a private Ed25519 JSON Web
    /// Key, naming the key directory at `agent`, its nonces drawn from
    /// `entropy`.
    pub fn new(key: &SecretString, agent: &str, entropy: Entropy) -> Result<Self, KeyError> {
        let (key, public) = parsed(key)?;
        if !agent
            .chars()
            .all(|character| matches!(character, ' '..='~') && !matches!(character, '"' | '\\'))
        {
            return Err(KeyError::BadAgent);
        }
        Ok(Self {
            key,
            key_id: key_id_of(&public),
            agent: format!("\"{agent}\""),
            entropy: Mutex::new(entropy),
        })
    }

    /// The key's ID, its RFC 7638 thumbprint, which the key directory names
    /// it by.
    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// The headers of a request to `url`, signed now; none for a URL that
    /// is not `http` or `https`, which carries no headers.
    #[must_use]
    pub fn sign(&self, url: &Url) -> Option<SignedHeaders> {
        let authority = authority(url)?;
        let mut covered = Covered::new(
            LABEL.to_owned(),
            IndexMap::from([
                (
                    CoveredComponent::Derived(DerivedComponent::Authority { req: false }),
                    authority,
                ),
                (
                    CoveredComponent::HTTP(HTTPField {
                        name: "signature-agent".to_owned(),
                        parameters: HTTPFieldParametersSet(vec![]),
                    }),
                    self.agent.clone(),
                ),
            ]),
        );
        sign(
            &mut covered,
            &self.key,
            &self.key_id,
            self.nonce(),
            TAG,
            VALID_FOR,
        );
        Some(SignedHeaders {
            signature_agent: self.agent.clone(),
            signature_input: covered.input,
            signature: covered.signature,
        })
    }

    /// A fresh nonce, [`NONCE_BYTES`] random bytes in Base64.
    fn nonce(&self) -> String {
        let mut bytes = [0; NONCE_BYTES];
        {
            let mut entropy = self.entropy.lock().unwrap_or_else(PoisonError::into_inner);
            for chunk in bytes.as_chunks_mut::<8>().0 {
                *chunk = entropy.next_u64().to_le_bytes();
            }
        }
        BASE64.encode(&bytes)
    }
}

/// The `@authority` of a request to `url` (RFC 9421, section 2.2.3): its
/// host, in lower case as a parsed URL holds it, and its port when it is
/// not the scheme's own; none for a URL that is not `http` or `https`.
fn authority(url: &Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}
