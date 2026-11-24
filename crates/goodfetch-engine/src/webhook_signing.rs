//! Standard Webhooks signatures (ADR 0017; #666): what a delivery to a
//! Notification webhook or a Sink webhook carries so that its receiver can
//! tell it is genuine, and the reference verifier that the SDKs' helpers
//! follow.
//!
//! A delivery carries three headers: `webhook-id`, its delivery ID
//! (ADR 0008); `webhook-timestamp`, the Unix time it was signed, in
//! seconds; and `webhook-signature`, one signature for each signing key,
//! separated by spaces, so that while a key rotates a receiver that trusts
//! either the current or the previous key verifies it. A signature is
//! `v1,<base64>`, HMAC-SHA256 under a `whsec_` secret over
//! `<id>.<timestamp>.<body>`, the default; or `v2a,<base64>`, ed25519 under
//! a `whsk_` key over the SHA-256 of `<endpoint>.<id>.<timestamp>.<body>`,
//! whose receiver holds only the `whpk_` public key. The specification
//! deprecated ADR 0017's `v1a` for `v2a` (ADR 0017's note of 2026-11-24).
//! The endpoint ID names the receiving endpoint and is never sent.
//!
//! A Goodfetch delivery's body is [`body`]'s envelope,
//! `{"run_epoch":<epoch>,"data":<data>}`, so its run epoch is signed with
//! it: the specification signs only the ID, the timestamp and the body.
//! [`Verifier`] refuses a timestamp more than five minutes from its own
//! time, either way, and compares each signature in constant time.
//!
//! A signing key is resolved from its Secret reference, so the engine's
//! masks hold its value (ADR 0015). [`SigningKey`] and [`VerifyingKey`]
//! zero it when dropped, and neither their `Debug` form nor an error quotes
//! it. No Notification or Sink webhook calls this module yet.

use std::fmt;
use std::ops::RangeInclusive;

use data_encoding::BASE64;
use ed25519_dalek::{Signature, Signer as _};
use hmac::{Hmac, Mac as _};
use secrecy::zeroize::Zeroizing;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::hostile_input::{HeaderValueInvalid, check_header_value, check_headers};

#[cfg(test)]
mod tests;

/// The header that carries a delivery's ID.
pub const ID_HEADER: &str = "webhook-id";

/// The header that carries the Unix time a delivery was signed, in seconds.
pub const TIMESTAMP_HEADER: &str = "webhook-timestamp";

/// The header that carries a delivery's signatures.
pub const SIGNATURE_HEADER: &str = "webhook-signature";

/// How far, in seconds, a delivery's timestamp may be from the verifier's
/// time, in the past or the future.
pub const TOLERANCE_SECONDS: i64 = 5 * 60;

/// The prefix of a `v1` secret.
const SECRET_PREFIX: &str = "whsec_";

/// The prefix of a `v2a` signing key.
const SECRET_KEY_PREFIX: &str = "whsk_";

/// The prefix of a `v2a` public key.
const PUBLIC_KEY_PREFIX: &str = "whpk_";

/// The bytes a `v1` secret holds, as the specification bounds them.
const SECRET_BYTES: RangeInclusive<usize> = 24..=64;

/// The bytes of an ed25519 seed or public key.
const ED25519_KEY_BYTES: usize = 32;

/// HMAC-SHA256, `v1`'s algorithm.
type HmacSha256 = Hmac<Sha256>;

/// A signature scheme of the specification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    /// HMAC-SHA256 under a shared secret, the default.
    V1,
    /// ed25519 over the SHA-256 of the endpoint ID and the message.
    V2a,
}

impl Scheme {
    /// The version a signature of this scheme starts with.
    #[must_use]
    pub fn version(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2a => "v2a",
        }
    }
}

/// Why a key, or the keys a signer or verifier is given, cannot be used;
/// it never quotes a key.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// The key has no known prefix.
    #[error(
        "a webhook signing key starts with `whsec_` (HMAC-SHA256, `v1`), `whsk_` (an ed25519 signing key, `v2a`) or, to verify, `whpk_` (an ed25519 public key)"
    )]
    Prefix,
    /// What follows the prefix is not Base64.
    #[error("a webhook signing key is Base64 after its prefix")]
    Encoding,
    /// The key holds too few or too many bytes.
    #[error(
        "a `whsec_` secret holds 24 to 64 bytes, and a `whsk_` or `whpk_` key 32; this one holds {0}"
    )]
    Length(usize),
    /// A `whpk_` key is no ed25519 public key.
    #[error("a `whpk_` key is no ed25519 public key")]
    PublicKey,
    /// A `whpk_` key was given to sign.
    #[error("a `whpk_` key only verifies: signing takes its `whsk_` key")]
    PublicKeyCannotSign,
    /// A signer or verifier was given no key.
    #[error("a webhook signer or verifier takes at least one key")]
    NoKey,
    /// A `v2a` key was given without the endpoint ID it signs for.
    #[error("a `v2a` key signs for an endpoint: give its endpoint ID")]
    NoEndpoint,
}

/// The ID of the endpoint a `v2a` signature is made for: chosen by the
/// sender, never sent, holding no `.`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointId(String);

impl EndpointId {
    /// Takes `id` as an endpoint ID.
    ///
    /// # Errors
    ///
    /// [`EndpointIdInvalid`] for an empty ID or one holding a `.`.
    pub fn new(id: impl Into<String>) -> Result<Self, EndpointIdInvalid> {
        let id = id.into();
        if id.is_empty() || id.contains('.') {
            return Err(EndpointIdInvalid);
        }
        Ok(Self(id))
    }

    /// The ID.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An endpoint ID that is empty or holds a `.`.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("an endpoint ID is not empty and holds no `.`")]
pub struct EndpointIdInvalid;

/// A key that signs deliveries, zeroed when dropped.
pub struct SigningKey(Signing);

/// A signing key of each scheme.
enum Signing {
    /// A `whsec_` secret's bytes.
    V1(Zeroizing<Vec<u8>>),
    /// A `whsk_` key.
    V2a(ed25519_dalek::SigningKey),
}

impl SigningKey {
    /// Reads a resolved `whsec_` secret or `whsk_` key.
    ///
    /// # Errors
    ///
    /// [`KeyError`] for a key of no known prefix, not Base64 after it, of
    /// the wrong length, or a `whpk_` public key.
    pub fn from_secret(secret: &SecretString) -> Result<Self, KeyError> {
        match parse(secret.expose_secret())? {
            Parsed::Secret(bytes) => Ok(Self(Signing::V1(bytes))),
            Parsed::SecretKey(seed) => Ok(Self(Signing::V2a(
                ed25519_dalek::SigningKey::from_bytes(&seed),
            ))),
            Parsed::PublicKey(_) => Err(KeyError::PublicKeyCannotSign),
        }
    }

    /// The key's scheme.
    #[must_use]
    pub fn scheme(&self) -> Scheme {
        match self.0 {
            Signing::V1(_) => Scheme::V1,
            Signing::V2a(_) => Scheme::V2a,
        }
    }

    /// The key a receiver verifies this key's signatures with: the same
    /// secret for `v1`, the public key for `v2a`.
    #[must_use]
    pub fn verifying_key(&self) -> VerifyingKey {
        match &self.0 {
            Signing::V1(secret) => VerifyingKey(Verifying::V1(secret.clone())),
            Signing::V2a(key) => VerifyingKey(Verifying::V2a(key.verifying_key())),
        }
    }

    /// This key's signature of `message`, `<version>,<base64>`.
    fn sign(&self, endpoint: Option<&EndpointId>, message: &Message<'_>) -> String {
        let signature = match &self.0 {
            Signing::V1(secret) => keyed(secret, message).finalize().into_bytes().to_vec(),
            Signing::V2a(key) => key.sign(&digest(endpoint, message)).to_bytes().to_vec(),
        };
        format!("{},{}", self.scheme().version(), BASE64.encode(&signature))
    }
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("SigningKey")
            .field(&self.scheme())
            .finish_non_exhaustive()
    }
}

/// A key that verifies deliveries, zeroed when dropped when it is a
/// secret.
#[derive(Clone)]
pub struct VerifyingKey(Verifying);

/// A verifying key of each scheme.
#[derive(Clone)]
enum Verifying {
    /// A `whsec_` secret's bytes.
    V1(Zeroizing<Vec<u8>>),
    /// A `whpk_` key.
    V2a(ed25519_dalek::VerifyingKey),
}

impl VerifyingKey {
    /// Reads a resolved `whsec_` secret, a `whpk_` public key, or a `whsk_`
    /// key, whose public key it keeps.
    ///
    /// # Errors
    ///
    /// [`KeyError`] for a key of no known prefix, not Base64 after it, of
    /// the wrong length, or a `whpk_` value that is no ed25519 public key.
    pub fn from_secret(secret: &SecretString) -> Result<Self, KeyError> {
        match parse(secret.expose_secret())? {
            Parsed::Secret(bytes) => Ok(Self(Verifying::V1(bytes))),
            Parsed::SecretKey(seed) => Ok(Self(Verifying::V2a(
                ed25519_dalek::SigningKey::from_bytes(&seed).verifying_key(),
            ))),
            Parsed::PublicKey(bytes) => ed25519_dalek::VerifyingKey::from_bytes(&bytes)
                .map(|key| Self(Verifying::V2a(key)))
                .map_err(|_| KeyError::PublicKey),
        }
    }

    /// The key's scheme.
    #[must_use]
    pub fn scheme(&self) -> Scheme {
        match self.0 {
            Verifying::V1(_) => Scheme::V1,
            Verifying::V2a(_) => Scheme::V2a,
        }
    }

    /// Whether `signature`, decoded, is this key's signature of `message`,
    /// compared in constant time.
    fn verifies(
        &self,
        endpoint: Option<&EndpointId>,
        message: &Message<'_>,
        signature: &[u8],
    ) -> bool {
        match &self.0 {
            Verifying::V1(secret) => keyed(secret, message).verify_slice(signature).is_ok(),
            Verifying::V2a(key) => Signature::from_slice(signature).is_ok_and(|signature| {
                key.verify_strict(&digest(endpoint, message), &signature)
                    .is_ok()
            }),
        }
    }
}

impl fmt::Debug for VerifyingKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("VerifyingKey")
            .field(&self.scheme())
            .finish_non_exhaustive()
    }
}

/// A key read from its text, before it becomes a signing or verifying key.
enum Parsed {
    /// A `whsec_` secret's bytes.
    Secret(Zeroizing<Vec<u8>>),
    /// A `whsk_` key's seed.
    SecretKey(Zeroizing<[u8; ED25519_KEY_BYTES]>),
    /// A `whpk_` key's bytes.
    PublicKey([u8; ED25519_KEY_BYTES]),
}

/// Reads `key` by its prefix.
fn parse(key: &str) -> Result<Parsed, KeyError> {
    if let Some(encoded) = key.strip_prefix(SECRET_PREFIX) {
        let bytes = decode(encoded)?;
        if !SECRET_BYTES.contains(&bytes.len()) {
            return Err(KeyError::Length(bytes.len()));
        }
        return Ok(Parsed::Secret(bytes));
    }
    if let Some(encoded) = key.strip_prefix(SECRET_KEY_PREFIX) {
        let mut seed = Zeroizing::new([0_u8; ED25519_KEY_BYTES]);
        seed.copy_from_slice(&exact(&decode(encoded)?)?);
        return Ok(Parsed::SecretKey(seed));
    }
    if let Some(encoded) = key.strip_prefix(PUBLIC_KEY_PREFIX) {
        return exact(&decode(encoded)?).map(Parsed::PublicKey);
    }
    Err(KeyError::Prefix)
}

/// Decodes a key's Base64, zeroed when dropped.
fn decode(encoded: &str) -> Result<Zeroizing<Vec<u8>>, KeyError> {
    BASE64
        .decode(encoded.as_bytes())
        .map(Zeroizing::new)
        .map_err(|_| KeyError::Encoding)
}

/// The 32 bytes of an ed25519 key.
fn exact(bytes: &[u8]) -> Result<[u8; ED25519_KEY_BYTES], KeyError> {
    bytes.try_into().map_err(|_| KeyError::Length(bytes.len()))
}

/// What is signed: a delivery's ID, timestamp and body.
#[derive(Clone, Copy, Debug)]
pub struct Message<'a> {
    /// The delivery ID, `webhook-id`.
    pub id: &'a str,
    /// The Unix time it is signed, in seconds, `webhook-timestamp`.
    pub timestamp: i64,
    /// The body, as sent.
    pub body: &'a [u8],
}

/// The MAC of `message` under a `v1` secret, ready for its bytes.
fn keyed(secret: &[u8], message: &Message<'_>) -> HmacSha256 {
    let mut mac =
        HmacSha256::new_from_slice(secret).expect("invariant: HMAC takes a key of any length");
    mac.update(format!("{}.{}.", message.id, message.timestamp).as_bytes());
    mac.update(message.body);
    mac
}

/// The SHA-256 a `v2a` key signs: of the endpoint ID, the delivery ID, the
/// timestamp and the body, separated by `.`.
fn digest(endpoint: Option<&EndpointId>, message: &Message<'_>) -> [u8; 32] {
    let endpoint = endpoint.map_or("", EndpointId::as_str);
    let mut hash = Sha256::new();
    hash.update(format!("{endpoint}.{}.{}.", message.id, message.timestamp).as_bytes());
    hash.update(message.body);
    hash.finalize().into()
}

/// Checks that every `v2a` key among `schemes` has an endpoint, and that
/// there is at least one.
fn check_keys(
    mut schemes: impl Iterator<Item = Scheme>,
    endpoint: Option<&EndpointId>,
) -> Result<(), KeyError> {
    let mut any = false;
    schemes.try_for_each(|scheme| {
        any = true;
        if scheme == Scheme::V2a && endpoint.is_none() {
            return Err(KeyError::NoEndpoint);
        }
        Ok(())
    })?;
    if any { Ok(()) } else { Err(KeyError::NoKey) }
}

/// Signs deliveries with every key it holds: the current key first, then
/// those a rotation still keeps.
#[derive(Debug)]
pub struct Signer {
    /// The keys, each adding its own signature.
    keys: Vec<SigningKey>,
    /// The endpoint a `v2a` key signs for.
    endpoint: Option<EndpointId>,
}

impl Signer {
    /// A signer with `keys`, for `endpoint` when one of them is `v2a`.
    ///
    /// # Errors
    ///
    /// [`KeyError::NoKey`] for no key, [`KeyError::NoEndpoint`] for a `v2a`
    /// key with no endpoint.
    pub fn new(keys: Vec<SigningKey>, endpoint: Option<EndpointId>) -> Result<Self, KeyError> {
        check_keys(keys.iter().map(SigningKey::scheme), endpoint.as_ref())?;
        Ok(Self { keys, endpoint })
    }

    /// The `webhook-signature` value of `message`: each key's signature, in
    /// order, separated by a space.
    #[must_use]
    pub fn signature(&self, message: &Message<'_>) -> String {
        self.keys
            .iter()
            .map(|key| key.sign(self.endpoint.as_ref(), message))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The headers a delivery of `message` is sent with: the webhook's
    /// `configured` headers, then `webhook-id`, `webhook-timestamp` and
    /// `webhook-signature`.
    ///
    /// # Errors
    ///
    /// [`DeliveryRefused`] when a configured header, or the delivery ID,
    /// holds a CR, an LF or a NUL (ADR 0067), or the ID is empty or holds a
    /// `.`.
    pub fn headers(
        &self,
        message: &Message<'_>,
        configured: &[(&str, &str)],
    ) -> Result<Vec<(String, String)>, DeliveryRefused> {
        check_headers(configured.iter().copied())?;
        check_header_value(ID_HEADER, message.id)?;
        if message.id.is_empty() || message.id.contains('.') {
            return Err(DeliveryRefused::Id);
        }
        Ok(configured
            .iter()
            .map(|&(name, value)| (name.to_owned(), value.to_owned()))
            .chain([
                (ID_HEADER.to_owned(), message.id.to_owned()),
                (TIMESTAMP_HEADER.to_owned(), message.timestamp.to_string()),
                (SIGNATURE_HEADER.to_owned(), self.signature(message)),
            ])
            .collect())
    }
}

/// Why a delivery is not sent.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DeliveryRefused {
    /// A header value holds a CR, an LF or a NUL: `header_value_invalid`.
    #[error(
        "the value of the header `{}` holds a CR, an LF or a NUL, which no header value may hold",
        .0.name
    )]
    Header(HeaderValueInvalid),
    /// The delivery ID is empty or holds a `.`, which would make its
    /// signed text ambiguous.
    #[error("a delivery ID is not empty and holds no `.`")]
    Id,
}

impl From<HeaderValueInvalid> for DeliveryRefused {
    fn from(invalid: HeaderValueInvalid) -> Self {
        Self::Header(invalid)
    }
}

/// A Goodfetch delivery's body: the run epoch beside the data, so that
/// both are signed.
///
/// # Errors
///
/// `data`'s own serialization error.
pub fn body<T: Serialize + ?Sized>(run_epoch: u64, data: &T) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&Envelope { run_epoch, data })
}

/// A Goodfetch delivery's body, as sent.
#[derive(Serialize)]
struct Envelope<'a, T: ?Sized> {
    /// The run epoch of the engine that sent it.
    run_epoch: u64,
    /// What it delivers.
    data: &'a T,
}

/// A delivery as received: its headers, each absent when missing, and its
/// body.
#[derive(Clone, Copy, Debug)]
pub struct Received<'a> {
    /// `webhook-id`.
    pub id: Option<&'a str>,
    /// `webhook-timestamp`.
    pub timestamp: Option<&'a str>,
    /// `webhook-signature`.
    pub signature: Option<&'a str>,
    /// The body, as received.
    pub body: &'a [u8],
}

/// A delivery that verified.
#[derive(Clone, Debug, PartialEq)]
pub struct Verified {
    /// The run epoch of the engine that sent it.
    pub run_epoch: u64,
    /// What it delivers.
    pub data: serde_json::Value,
}

/// A Goodfetch delivery's body, as read back.
#[derive(Deserialize)]
struct ReadBack {
    /// The run epoch of the engine that sent it.
    run_epoch: u64,
    /// What it delivers.
    data: serde_json::Value,
}

/// Why a delivery does not verify.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum Unverified {
    /// A header is missing.
    #[error("the delivery has no `{0}` header")]
    MissingHeader(&'static str),
    /// `webhook-timestamp` is no Unix time in seconds.
    #[error("`webhook-timestamp` is no Unix time in seconds")]
    TimestampInvalid,
    /// The timestamp is more than five minutes in the past.
    #[error("the delivery was signed more than five minutes ago")]
    TimestampTooOld,
    /// The timestamp is more than five minutes in the future.
    #[error("the delivery is dated more than five minutes ahead")]
    TimestampTooNew,
    /// No signature is a trusted key's signature of the delivery.
    #[error("no signature of the delivery is a trusted key's")]
    NoMatchingSignature,
    /// The body is no Goodfetch delivery: no run epoch beside its data.
    #[error("the delivery's body holds no `run_epoch` and `data`")]
    NotADelivery,
}

/// Verifies deliveries against the keys it trusts.
#[derive(Debug)]
pub struct Verifier {
    /// The trusted keys.
    keys: Vec<VerifyingKey>,
    /// The endpoint a `v2a` signature is made for.
    endpoint: Option<EndpointId>,
}

impl Verifier {
    /// A verifier trusting `keys`, for `endpoint` when one of them is
    /// `v2a`.
    ///
    /// # Errors
    ///
    /// [`KeyError::NoKey`] for no key, [`KeyError::NoEndpoint`] for a `v2a`
    /// key with no endpoint.
    pub fn new(keys: Vec<VerifyingKey>, endpoint: Option<EndpointId>) -> Result<Self, KeyError> {
        check_keys(keys.iter().map(VerifyingKey::scheme), endpoint.as_ref())?;
        Ok(Self { keys, endpoint })
    }

    /// Verifies `received`'s signature at `now`, the verifier's Unix time
    /// in seconds, whatever its body.
    ///
    /// # Errors
    ///
    /// [`Unverified`] for a missing header, a timestamp that is not a Unix
    /// time or is more than five minutes from `now`, or no signature a
    /// trusted key made; an entry of an unknown version, or that is not
    /// Base64, matches nothing.
    pub fn verify_signature(&self, received: &Received<'_>, now: i64) -> Result<(), Unverified> {
        let id = received.id.ok_or(Unverified::MissingHeader(ID_HEADER))?;
        let timestamp = received
            .timestamp
            .ok_or(Unverified::MissingHeader(TIMESTAMP_HEADER))?;
        let signature = received
            .signature
            .ok_or(Unverified::MissingHeader(SIGNATURE_HEADER))?;
        let timestamp = unix_seconds(timestamp)?;
        if now.saturating_sub(timestamp) > TOLERANCE_SECONDS {
            return Err(Unverified::TimestampTooOld);
        }
        if timestamp.saturating_sub(now) > TOLERANCE_SECONDS {
            return Err(Unverified::TimestampTooNew);
        }
        let message = Message {
            id,
            timestamp,
            body: received.body,
        };
        let matched = signature.split(' ').any(|entry| {
            let Some((version, encoded)) = entry.split_once(',') else {
                return false;
            };
            let Ok(decoded) = BASE64.decode(encoded.as_bytes()) else {
                return false;
            };
            self.keys.iter().any(|key| {
                key.scheme().version() == version
                    && key.verifies(self.endpoint.as_ref(), &message, &decoded)
            })
        });
        if matched {
            Ok(())
        } else {
            Err(Unverified::NoMatchingSignature)
        }
    }

    /// Verifies `received` as [`Verifier::verify_signature`] does, then
    /// reads its body's run epoch and data.
    ///
    /// # Errors
    ///
    /// [`Verifier::verify_signature`]'s, then
    /// [`Unverified::NotADelivery`] for a body that is not [`body`]'s
    /// envelope.
    pub fn verify(&self, received: &Received<'_>, now: i64) -> Result<Verified, Unverified> {
        self.verify_signature(received, now)?;
        let read: ReadBack =
            serde_json::from_slice(received.body).map_err(|_| Unverified::NotADelivery)?;
        Ok(Verified {
            run_epoch: read.run_epoch,
            data: read.data,
        })
    }
}

/// Reads a `webhook-timestamp`: ASCII digits only.
fn unix_seconds(timestamp: &str) -> Result<i64, Unverified> {
    if timestamp.is_empty() || !timestamp.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Unverified::TimestampInvalid);
    }
    timestamp.parse().map_err(|_| Unverified::TimestampInvalid)
}
