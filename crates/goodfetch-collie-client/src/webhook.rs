//! The verification helper of a `webhook` channel's delivery (ADRs 0017,
//! 0027 and 0030; issue 1391): pure, with no engine and no Connection, so a
//! receiver calls it on whatever server or edge runtime takes the call. It
//! checks the Standard Webhooks headers, `webhook-id`, `webhook-timestamp`
//! and `webhook-signature`, against the secrets given, the current one and
//! the previous one while a secret rotates, then reads the body as the
//! [`NotificationPayload`] the engine sent. A call it refuses fails with
//! `webhook_invalid`, whose details name the [`WebhookInvalidReason`].
//!
//! A `whsec_` secret verifies a `v1` signature, HMAC-SHA256 over
//! `<id>.<timestamp>.<body>`; a `whpk_` public key verifies a `v2a` one,
//! ed25519 over the SHA-256 of `<endpoint>.<id>.<timestamp>.<body>`, for
//! the endpoint ID [`VerifyWebhookOptions::endpoint`] names. Each signature
//! is compared in constant time, and an entry of a version no secret has,
//! or that is no Base64, matches nothing, as the engine's reference
//! verifier does.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use data_encoding::BASE64;
use ed25519_dalek::{Signature, VerifyingKey};
use goodfetch_protocol::notifications::{
    NOTIFICATION_PAYLOAD_VERSION, NotificationPayload, WEBHOOK_INVALID, WebhookInvalidDetails,
    WebhookInvalidReason,
};
use hmac::{Hmac, Mac as _};
use sha2::{Digest as _, Sha256};

use crate::Error;

/// How far a delivery's timestamp may be from the receiver's time, either
/// way, when [`VerifyWebhookOptions::tolerance`] is not set.
const TOLERANCE: Duration = Duration::from_mins(5);

/// The headers a delivery carries, as Standard Webhooks names them.
const ID: &str = "webhook-id";
const TIMESTAMP: &str = "webhook-timestamp";
const SIGNATURE: &str = "webhook-signature";

/// What [`verify_webhook_with`] takes beside the call: the tolerance, the
/// endpoint a `v2a` signature is made for, and the time to verify at.
#[derive(Clone, Debug, Default)]
pub struct VerifyWebhookOptions {
    tolerance: Option<Duration>,
    endpoint: Option<String>,
    at: Option<SystemTime>,
}

impl VerifyWebhookOptions {
    /// How far the delivery's timestamp may be from now, either way:
    /// 5 minutes when not set.
    #[must_use]
    pub fn tolerance(mut self, tolerance: Duration) -> Self {
        self.tolerance = Some(tolerance);
        self
    }

    /// The endpoint ID the channel's `whsk_` key signs for, which a `whpk_`
    /// secret needs; never sent with the call.
    #[must_use]
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// The time to verify at in place of the system's, such as a test's.
    #[must_use]
    pub fn at(mut self, now: SystemTime) -> Self {
        self.at = Some(now);
        self
    }
}

/// Verifies that a call is a `webhook` channel's delivery signed with one
/// of `secrets` less than 5 minutes ago, and reads its body: `body` as
/// received, before any parsing, and `headers` as name and value pairs,
/// whose names are matched in any case.
///
/// # Errors
///
/// `webhook_invalid` for a call that is no such delivery, its details
/// naming why; `invalid_params` for no secret, a secret that is neither a
/// `whsec_` secret nor a `whpk_` public key, or a `whpk_` key with no
/// endpoint.
///
/// # Examples
///
/// ```
/// use goodfetch_collie_client::{ErrorCode, verify_webhook};
///
/// let secret = "whsec_AAAAAAAAAAAAAAAAAAAAAAAA";
/// // A call without the Standard Webhooks headers is no delivery.
/// let refused = verify_webhook(b"{}", [("content-type", "application/json")], &[secret]);
/// assert_eq!(refused.unwrap_err().code(), ErrorCode::WEBHOOK_INVALID);
/// ```
pub fn verify_webhook<'a>(
    body: &[u8],
    headers: impl IntoIterator<Item = (&'a str, &'a str)>,
    secrets: &[impl AsRef<str>],
) -> Result<NotificationPayload, Error> {
    verify_webhook_with(body, headers, secrets, &VerifyWebhookOptions::default())
}

/// [`verify_webhook`] with `options`.
///
/// # Errors
///
/// As [`verify_webhook`].
///
/// # Examples
///
/// ```
/// use std::time::{Duration, UNIX_EPOCH};
///
/// use goodfetch_collie_client::{ErrorCode, VerifyWebhookOptions, verify_webhook_with};
///
/// let secret = "whsec_AAAAAAAAAAAAAAAAAAAAAAAA";
/// let headers = [
///     ("webhook-id", "evt_01JDQ8X2Y5"),
///     ("webhook-timestamp", "1767603600"),
///     ("webhook-signature", "v1,c2lnbmF0dXJl"),
/// ];
/// // An hour after the call was signed, past a 10-minute tolerance.
/// let options = VerifyWebhookOptions::default()
///     .tolerance(Duration::from_mins(10))
///     .at(UNIX_EPOCH + Duration::from_secs(1_767_603_600 + 3_600));
/// let refused = verify_webhook_with(b"{}", headers, &[secret], &options);
/// assert_eq!(refused.unwrap_err().code(), ErrorCode::WEBHOOK_INVALID);
/// ```
pub fn verify_webhook_with<'a>(
    body: &[u8],
    headers: impl IntoIterator<Item = (&'a str, &'a str)>,
    secrets: &[impl AsRef<str>],
    options: &VerifyWebhookOptions,
) -> Result<NotificationPayload, Error> {
    let trusted = Trusted::read(secrets, options.endpoint.as_deref())?;
    let call = Call::read(headers)?;
    #[expect(
        clippy::disallowed_methods,
        reason = "ADR 0059: the receiving program has no engine and no Clock; `VerifyWebhookOptions::at` sets the time a test reads"
    )]
    let now = options
        .at
        .unwrap_or_else(SystemTime::now)
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let tolerance = options.tolerance.unwrap_or(TOLERANCE).as_secs();
    let timestamp = seconds(call.timestamp)?;
    if now.abs_diff(timestamp) > tolerance {
        return Err(invalid(
            WebhookInvalidReason::Timestamp,
            &format!(
                "the call was signed more than {tolerance} seconds from now: a delayed or replayed call"
            ),
        ));
    }
    let signed = format!("{}.{}.", call.id, call.timestamp);
    if !trusted.signed(&signed, body, call.signature) {
        return Err(invalid(
            WebhookInvalidReason::Signature,
            "no signature of `webhook-signature` matches a secret given",
        ));
    }
    let payload: NotificationPayload = serde_json::from_slice(body).map_err(|_| {
        invalid(
            WebhookInvalidReason::Malformed,
            "the body is no Notification payload",
        )
    })?;
    if payload.format.major != NOTIFICATION_PAYLOAD_VERSION.major {
        return Err(invalid(
            WebhookInvalidReason::Malformed,
            &format!(
                "the body's `format` is {}, a major this SDK does not read",
                payload.format
            ),
        ));
    }
    Ok(payload)
}

/// The three Standard Webhooks headers of a call.
struct Call<'a> {
    id: &'a str,
    timestamp: &'a str,
    signature: &'a str,
}

impl<'a> Call<'a> {
    /// The first value of each header, whatever its name's case.
    fn read(headers: impl IntoIterator<Item = (&'a str, &'a str)>) -> Result<Self, Error> {
        let (mut id, mut timestamp, mut signature) = (None, None, None);
        for (name, value) in headers {
            let slot = if name.eq_ignore_ascii_case(ID) {
                &mut id
            } else if name.eq_ignore_ascii_case(TIMESTAMP) {
                &mut timestamp
            } else if name.eq_ignore_ascii_case(SIGNATURE) {
                &mut signature
            } else {
                continue;
            };
            slot.get_or_insert(value);
        }
        Ok(Self {
            id: id.ok_or_else(|| missing(ID))?,
            timestamp: timestamp.ok_or_else(|| missing(TIMESTAMP))?,
            signature: signature.ok_or_else(|| missing(SIGNATURE))?,
        })
    }
}

/// The secrets a call may be signed with, and the endpoint their `v2a`
/// signatures name.
struct Trusted<'a> {
    keys: Vec<Key>,
    endpoint: &'a str,
}

/// A secret, read.
enum Key {
    /// A `whsec_` secret's bytes, which verify a `v1` signature.
    V1(Vec<u8>),
    /// A `whpk_` public key, which verifies a `v2a` signature.
    V2a(VerifyingKey),
}

impl<'a> Trusted<'a> {
    /// Reads `secrets`, refusing one that is no secret and a `whpk_` key
    /// without `endpoint`.
    fn read(secrets: &[impl AsRef<str>], endpoint: Option<&'a str>) -> Result<Self, Error> {
        if secrets.is_empty() {
            return Err(Error::refused(
                "verify_webhook",
                "no secret was given",
                "Pass the channel's `webhook.secret`, and the previous one too while it rotates.",
            ));
        }
        let keys = secrets
            .iter()
            .map(|secret| Key::read(secret.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        let endpoint = match endpoint {
            Some(endpoint) if !endpoint.is_empty() && !endpoint.contains('.') => endpoint,
            Some(_) => {
                return Err(Error::refused(
                    "verify_webhook",
                    "the endpoint ID is empty or holds a `.`",
                    "Pass the endpoint ID the channel's `whsk_` key signs for.",
                ));
            }
            None if keys.iter().any(|key| matches!(key, Key::V2a(_))) => {
                return Err(Error::refused(
                    "verify_webhook",
                    "a `whpk_` key verifies a `v2a` signature, which names an endpoint, and none was given",
                    "Pass the endpoint ID the channel's `whsk_` key signs for.",
                ));
            }
            None => "",
        };
        Ok(Self { keys, endpoint })
    }

    /// Whether an entry of `signatures` is a key's signature of `signed`,
    /// the ID and timestamp, followed by `body`.
    fn signed(&self, signed: &str, body: &[u8], signatures: &str) -> bool {
        signatures.split(' ').any(|entry| {
            let Some((version, encoded)) = entry.split_once(',') else {
                return false;
            };
            let Ok(signature) = BASE64.decode(encoded.as_bytes()) else {
                return false;
            };
            self.keys
                .iter()
                .any(|key| key.verifies(version, self.endpoint, signed, body, &signature))
        })
    }
}

impl Key {
    /// Reads a secret by its prefix.
    fn read(secret: &str) -> Result<Self, Error> {
        let unreadable = || {
            Error::refused(
                "verify_webhook",
                "a secret is neither a `whsec_` secret nor a `whpk_` public key in Base64",
                "Pass the channel's `webhook.secret` as the engine has it, or the `whpk_` public key of its `whsk_` key.",
            )
        };
        // An empty `whsec_` secret is refused, as WebCrypto refuses an
        // empty HMAC key, so each SDK accepts the same secrets.
        if let Some(encoded) = secret.strip_prefix("whsec_") {
            return BASE64
                .decode(encoded.as_bytes())
                .ok()
                .filter(|bytes| !bytes.is_empty())
                .map(Self::V1)
                .ok_or_else(unreadable);
        }
        let bytes = secret
            .strip_prefix("whpk_")
            .and_then(|encoded| BASE64.decode(encoded.as_bytes()).ok())
            .ok_or_else(unreadable)?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| unreadable())?;
        VerifyingKey::from_bytes(&bytes)
            .map(Self::V2a)
            .map_err(|_| unreadable())
    }

    /// Whether `signature`, of `version`, is this key's signature of
    /// `signed` followed by `body`, compared in constant time.
    fn verifies(
        &self,
        version: &str,
        endpoint: &str,
        signed: &str,
        body: &[u8],
        signature: &[u8],
    ) -> bool {
        match self {
            Self::V1(secret) if version == "v1" => {
                let mut mac = Hmac::<Sha256>::new_from_slice(secret)
                    .expect("invariant: HMAC takes a key of any length");
                mac.update(signed.as_bytes());
                mac.update(body);
                mac.verify_slice(signature).is_ok()
            }
            Self::V2a(key) if version == "v2a" => {
                let mut hash = Sha256::new();
                hash.update(endpoint.as_bytes());
                hash.update(b".");
                hash.update(signed.as_bytes());
                hash.update(body);
                Signature::from_slice(signature)
                    .is_ok_and(|signature| key.verify_strict(&hash.finalize(), &signature).is_ok())
            }
            Self::V1(_) | Self::V2a(_) => false,
        }
    }
}

/// Reads a `webhook-timestamp`: ASCII digits only, in seconds.
fn seconds(timestamp: &str) -> Result<u64, Error> {
    timestamp
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| timestamp.parse().ok())
        .flatten()
        .ok_or_else(|| {
            invalid(
                WebhookInvalidReason::Timestamp,
                "`webhook-timestamp` is no Unix time in seconds",
            )
        })
}

/// A call with no `name` header.
fn missing(name: &str) -> Error {
    invalid(
        WebhookInvalidReason::MissingHeader,
        &format!("the call has no `{name}` header"),
    )
}

/// `webhook_invalid` for `reason`, saying what failed.
fn invalid(reason: WebhookInvalidReason, what_failed: &str) -> Error {
    Error::of_sdk(
        &WEBHOOK_INVALID,
        &format!("{what_failed}, so it is no verified `webhook` delivery"),
        match reason {
            WebhookInvalidReason::Signature => {
                "Answer the call with an error; when every delivery fails, pass the channel's `webhook.secret`, and the previous one too while it rotates."
            }
            WebhookInvalidReason::Timestamp => {
                "Answer the call with an error; when every delivery fails, check the receiver's clock."
            }
            WebhookInvalidReason::Malformed => {
                "Answer the call with an error; when every delivery fails, update the SDK to one that reads the engine's payload."
            }
            WebhookInvalidReason::MissingHeader => {
                "Pass the request's headers as received; a call without them is no `webhook` delivery."
            }
        },
        &WebhookInvalidDetails { reason },
    )
}

#[cfg(test)]
mod tests;
