//! The content origin's signed addresses (ADR 0031; spec #592): scraped
//! HTML, screenshots, DOM snapshots and page bodies are shown only from a
//! second origin, which carries no cookie and takes no credential but the
//! address itself. The Console's origin turns a file's handle into an
//! address on that origin signed for that one file, as it is to be shown,
//! and valid for [`VALID_FOR`]; the content origin serves a file only to an
//! address whose signature holds, for that file, and has not expired.
//!
//! The signature is HMAC-SHA-256, under a key of the OS generator that
//! lives as long as the Daemon's process, over the file's Run, its ID, the
//! way it is shown and the address's expiry. Rules that depend on time take
//! the wall time as an argument (ADR 0059).

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_protocol::run::RunId;
use hmac::{Hmac, Mac as _};
use sha2::Sha256;

use crate::endpoint::same_token;

/// How long a signed address stays valid.
pub(crate) const VALID_FOR: Duration = Duration::from_mins(5);

/// The path under which both origins name a Run's bodies:
/// `/bodies/<run_id>/<body>`.
pub(crate) const BODIES: &str = "/bodies/";

/// What the signature covers before the file, so a MAC made for another
/// purpose under the same key is never a content address's.
const PURPOSE: &[u8] = b"goodfetch content address";

/// The key content addresses are signed under: 32 bytes of the OS
/// generator, kept only in memory.
pub(crate) struct ContentKey([u8; 32]);

impl fmt::Debug for ContentKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ContentKey(…)")
    }
}

/// How a file is shown, which its address carries and its answer's
/// `Content-Type` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shown {
    /// A page body or a DOM snapshot, as HTML.
    Html,
    /// Text.
    Text,
    /// A PNG image, such as a screenshot.
    Png,
    /// A JPEG image.
    Jpeg,
    /// A WebP image.
    Webp,
}

impl Shown {
    /// Its name in an address's `type` parameter.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::Text => "text",
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Webp => "webp",
        }
    }

    /// The way named `name`, if any.
    pub(crate) fn named(name: &str) -> Option<Self> {
        [Self::Html, Self::Text, Self::Png, Self::Jpeg, Self::Webp]
            .into_iter()
            .find(|shown| shown.name() == name)
    }

    /// The `Content-Type` its answer carries.
    pub(crate) fn media_type(self) -> &'static str {
        match self {
            Self::Html => "text/html; charset=utf-8",
            Self::Text => "text/plain; charset=utf-8",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }
}

/// A file of a Run, by its handle, and how it is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContentFile {
    /// The Run whose folder holds it.
    pub(crate) run_id: RunId,
    /// Its ID, the lowercase hex SHA-256 of its bytes.
    pub(crate) body: String,
    /// How it is shown.
    pub(crate) shown: Shown,
}

impl ContentFile {
    /// The file `run_id` and `body` name, shown as `shown`; `None` when
    /// they name no Run or no body.
    pub(crate) fn of(run_id: &str, body: &str, shown: Shown) -> Option<Self> {
        let run_id = RunId::parse(run_id).ok()?;
        is_body_id(body).then(|| Self {
            run_id,
            body: body.to_owned(),
            shown,
        })
    }

    /// Its path on either origin, `/bodies/<run_id>/<body>`.
    pub(crate) fn path(&self) -> String {
        format!("{BODIES}{}/{}", self.run_id, self.body)
    }
}

/// Why the content origin serves nothing to an address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unsigned {
    /// Its path names no file.
    NoFile,
    /// It carries no signature, expiry or way to show the file.
    Missing,
    /// Its signature is not this file's: altered, made for another file,
    /// or made under another key.
    Wrong,
    /// Its signature holds, but it has expired.
    Expired,
}

impl Unsigned {
    /// The endpoint table's reason, which the refusal is recorded with;
    /// `None` for a path that names no file, which is no refusal.
    pub(crate) fn reason(self) -> Option<&'static str> {
        match self {
            Self::NoFile => None,
            Self::Missing => Some("token_missing"),
            Self::Wrong => Some("token_wrong"),
            Self::Expired => Some("token_expired"),
        }
    }
}

impl ContentKey {
    /// A new key from the OS generator.
    ///
    /// # Errors
    ///
    /// When the OS generator gives nothing.
    pub(crate) fn generated() -> Result<Self, getrandom::Error> {
        let mut key = [0; 32];
        getrandom::fill(&mut key)?;
        Ok(Self(key))
    }

    /// The content origin's path and query of `file`, signed for it and
    /// valid for [`VALID_FOR`] from `now`:
    /// `/bodies/<run_id>/<body>?type=<shown>&expires=<seconds>&signature=<hex>`.
    pub(crate) fn sign(&self, file: &ContentFile, now: SystemTime) -> String {
        let expires = seconds(now).saturating_add(VALID_FOR.as_secs());
        format!(
            "{}?type={}&expires={expires}&signature={}",
            file.path(),
            file.shown.name(),
            self.mac(file, expires)
        )
    }

    /// The file `target`, a request's path and query, is signed for, if
    /// its signature holds at `now`.
    ///
    /// # Errors
    ///
    /// Why it serves no file.
    pub(crate) fn check(&self, target: &str, now: SystemTime) -> Result<ContentFile, Unsigned> {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let (run_id, body) = path
            .strip_prefix(BODIES)
            .and_then(|rest| rest.split_once('/'))
            .ok_or(Unsigned::NoFile)?;
        let parameter = |name: &str| {
            let mut found = query
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .filter(|(key, _)| *key == name)
                .map(|(_, value)| value);
            match (found.next(), found.next()) {
                (Some(value), None) => Ok(Some(value)),
                (None, _) => Ok(None),
                (Some(_), Some(_)) => Err(Unsigned::Wrong),
            }
        };
        let (Some(shown), Some(expires), Some(signature)) = (
            parameter("type")?,
            parameter("expires")?,
            parameter("signature")?,
        ) else {
            return Err(Unsigned::Missing);
        };
        let shown = Shown::named(shown).ok_or(Unsigned::Wrong)?;
        let expires = expires.parse::<u64>().map_err(|_| Unsigned::Wrong)?;
        let file = ContentFile::of(run_id, body, shown).ok_or(Unsigned::NoFile)?;
        if !same_token(signature, &self.mac(&file, expires)) {
            return Err(Unsigned::Wrong);
        }
        if seconds(now) >= expires {
            return Err(Unsigned::Expired);
        }
        Ok(file)
    }

    /// The signature of `file` expiring at `expires`, in lowercase hex.
    fn mac(&self, file: &ContentFile, expires: u64) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0)
            .expect("invariant: HMAC takes a key of any length");
        for part in [
            PURPOSE,
            file.run_id.as_str().as_bytes(),
            file.body.as_bytes(),
            file.shown.name().as_bytes(),
            expires.to_string().as_bytes(),
        ] {
            mac.update(part);
            mac.update(b"\0");
        }
        data_encoding::HEXLOWER.encode(&mac.finalize().into_bytes())
    }
}

/// Whether `body` is a body's ID: 64 lowercase hex digits.
fn is_body_id(body: &str) -> bool {
    body.len() == 64
        && body
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Whole seconds since the Unix epoch at `moment`; 0 before it.
fn seconds(moment: SystemTime) -> u64 {
    moment
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

#[cfg(test)]
mod tests;
