//! The canonical body `replay` keys a request by (ADR 0046), as a pure
//! function of its `Content-Type` and its bytes: a JSON body with its
//! object keys sorted (RFC 8785), a form with its fields sorted by name and
//! each name's values in their order, any other body as it is. So a
//! program that sends the same JSON or form in another order is served
//! what the first one was.
//!
//! A crawl request's body goes with no `Content-Type` of its own, so a body
//! without one is read by its bytes: JSON when it is an object or an array
//! that parses, a form when it is `name=value` pairs of URL-safe ASCII. A
//! form's fields are compared as sent, never percent-decoded, so two
//! different bodies never share a key.

use std::borrow::Cow;

use sha2::{Digest as _, Sha256};

/// The key of a request with no body: the SHA-256 of nothing, which no
/// tagged body below hashes to.
const NO_BODY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// What a body is read as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Json,
    Form,
    Raw,
}

/// The SHA-256 of `body`'s canonical form, as lower-case hex, its media
/// type read from `content_type`, or from its bytes without one: a JSON
/// one (`application/json` or a `+json` suffix) with its keys sorted, a
/// form (`application/x-www-form-urlencoded`) with its fields sorted by
/// name, their values kept in order, and any other, or JSON that does not
/// parse, as it is. Each form is tagged with its kind, so one never stands
/// for another; an empty body is [`NO_BODY`].
pub(crate) fn body_sha256(content_type: Option<&str>, body: &[u8]) -> String {
    if body.is_empty() {
        return NO_BODY.to_owned();
    }
    let json = || {
        serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|value| serde_json_canonicalizer::to_vec(&value).ok())
    };
    let (tag, canonical): (&[u8], Cow<'_, [u8]>) = match kind(content_type, body) {
        Kind::Json => match json() {
            Some(json) => (b"json\n", Cow::Owned(json)),
            None => (b"raw\n", Cow::Borrowed(body)),
        },
        Kind::Form => {
            let mut fields: Vec<&[u8]> = body
                .split(|&byte| byte == b'&')
                .filter(|field| !field.is_empty())
                .collect();
            // Stable: a name's values keep their order.
            fields.sort_by_key(|field| field.split(|&byte| byte == b'=').next());
            (b"form\n", Cow::Owned(fields.join(&b'&')))
        }
        Kind::Raw => (b"raw\n", Cow::Borrowed(body)),
    };
    let mut hash = Sha256::new();
    hash.update(tag);
    hash.update(&canonical);
    data_encoding::HEXLOWER.encode(&hash.finalize())
}

/// What `body`, sent with `content_type`, is read as.
fn kind(content_type: Option<&str>, body: &[u8]) -> Kind {
    let Some(content_type) = content_type else {
        return sniffed(body);
    };
    let media = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if media == "application/json" || media.ends_with("+json") {
        Kind::Json
    } else if media == "application/x-www-form-urlencoded" {
        Kind::Form
    } else {
        Kind::Raw
    }
}

/// What `body`, sent with no `Content-Type`, is read as: JSON when it
/// opens an object or an array, a form when it is `name=value` pairs of
/// URL-safe ASCII, otherwise as it is.
fn sniffed(body: &[u8]) -> Kind {
    match body.trim_ascii_start().first() {
        Some(b'{' | b'[') => Kind::Json,
        _ if body.contains(&b'=')
            && body.iter().all(|&byte| {
                byte.is_ascii_alphanumeric() || b"-._~%+=&*!'(),:/;@?".contains(&byte)
            }) =>
        {
            Kind::Form
        }
        _ => Kind::Raw,
    }
}

#[cfg(test)]
mod tests;
