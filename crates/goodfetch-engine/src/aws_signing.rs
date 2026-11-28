//! AWS Signature Version 4 (the "Signature Version 4 signing process"
//! pages of AWS's General Reference): a POST to an AWS API signed by an
//! access key, for each engine caller of one, Secrets Manager's Secret
//! provider and the Amazon SES email channel. The secret key and session
//! token never leave this module but as the signature and the
//! `x-amz-security-token` header, both marked sensitive.

use std::time::SystemTime;

use hmac::{Hmac, Mac as _};
use hyper::Request;
use hyper::body::Bytes;
use hyper::header::HeaderValue;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use url::Url;

#[cfg(test)]
mod tests;

/// An access key: its ID, its secret and, for temporary credentials, the
/// session token.
#[derive(Clone)]
pub struct Credentials {
    /// The access key ID.
    pub access_key_id: String,
    /// The secret access key.
    pub secret_access_key: SecretString,
    /// The session token of temporary credentials.
    pub session_token: Option<SecretString>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("access_key_id", &self.access_key_id)
            .finish_non_exhaustive()
    }
}

/// Where a signature is valid: an AWS Region and the service's signing
/// name, such as `secretsmanager` or `ses`.
#[derive(Clone, Copy, Debug)]
pub struct Scope<'a> {
    /// The Region, such as `eu-west-3`.
    pub region: &'a str,
    /// The service's signing name.
    pub service: &'a str,
}

/// Why a request cannot be signed: a part of the access key no HTTP
/// header may carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Unsignable {
    /// The session token.
    #[error("the access key's session token holds characters no HTTP header may carry")]
    SessionToken,
    /// The access key ID.
    #[error("the access key ID holds characters no HTTP header may carry")]
    AccessKeyId,
}

/// A POST of `body` to `url` with `headers`, each named in lower case,
/// signed for `scope` by `credentials` at `now`: the `host` and
/// `x-amz-date` headers, and the session token when it has one, are added
/// and signed with them.
///
/// # Errors
///
/// [`Unsignable`] when the session token or the access key ID holds
/// characters no HTTP header may carry.
///
/// # Panics
///
/// When a header of `headers` is no valid header, which its caller's
/// constants always are.
pub fn signed_post(
    url: &Url,
    headers: &[(&'static str, &str)],
    body: Bytes,
    scope: Scope<'_>,
    credentials: &Credentials,
    now: SystemTime,
) -> Result<Request<Bytes>, Unsignable> {
    signed_request(
        hyper::Method::POST,
        url,
        headers,
        body,
        scope,
        credentials,
        now,
    )
}

/// An HTTP request of `method` and `body` to `url` with `headers`, each named
/// in lower case, signed for `scope` by `credentials` at `now`: `host`,
/// `x-amz-date`, `x-amz-content-sha256` (for `s3`), and `x-amz-security-token`
/// (when present) are added and signed, with canonicalized query parameters.
///
/// # Errors
///
/// [`Unsignable`] when the session token or the access key ID holds
/// characters no HTTP header may carry.
///
/// # Panics
///
/// When a header of `headers` is no valid header, which its caller's
/// constants always are.
pub fn signed_request(
    method: hyper::Method,
    url: &Url,
    headers: &[(&str, &str)],
    body: Bytes,
    scope: Scope<'_>,
    credentials: &Credentials,
    now: SystemTime,
) -> Result<Request<Bytes>, Unsignable> {
    let amz_date = jiff::Timestamp::try_from(now)
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
        .strftime("%Y%m%dT%H%M%SZ")
        .to_string();
    let host = match url.port() {
        Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
        None => url.host_str().unwrap_or_default().to_owned(),
    };
    let payload_hash = data_encoding::HEXLOWER.encode(&Sha256::digest(&body));
    let mut signed: Vec<(&str, String)> = headers
        .iter()
        .map(|(name, value)| (*name, (*value).to_owned()))
        .chain([("host", host), ("x-amz-date", amz_date.clone())])
        .collect();
    if scope.service == "s3"
        && !signed
            .iter()
            .any(|(name, _)| *name == "x-amz-content-sha256")
    {
        signed.push(("x-amz-content-sha256", payload_hash.clone()));
    }
    if let Some(token) = &credentials.session_token {
        signed.push(("x-amz-security-token", token.expose_secret().to_owned()));
    }
    signed.sort_by(|left, right| left.0.cmp(right.0));
    let query = canonical_query(url);
    let authorization = authorization(
        &Signing {
            method: method.as_str(),
            path: url.path(),
            query: &query,
            headers: &signed,
            payload_hash: &payload_hash,
            region: scope.region,
            service: scope.service,
            amz_date: &amz_date,
        },
        credentials,
    );
    let mut builder = Request::builder().method(method).uri(url.as_str());
    for (name, value) in &signed {
        if *name == "host" {
            continue;
        }
        let value = if *name == "x-amz-security-token" {
            let mut token = HeaderValue::from_str(value).map_err(|_| Unsignable::SessionToken)?;
            token.set_sensitive(true);
            token
        } else {
            HeaderValue::from_str(value)
                .expect("invariant: the caller's headers and the date are valid header values")
        };
        builder = builder.header(*name, value);
    }
    let mut authorization =
        HeaderValue::from_str(&authorization).map_err(|_| Unsignable::AccessKeyId)?;
    authorization.set_sensitive(true);
    Ok(builder
        .header("authorization", authorization)
        .body(body)
        .expect("invariant: a parsed URL and checked headers make a request"))
}

/// Percent-encodes `input` per AWS `SigV4` (unreserved: `A-Z a-z 0-9 - _ . ~`).
fn aws_uri_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Canonical query string for `SigV4`: sorted by encoded key, then encoded value.
fn canonical_query(url: &Url) -> String {
    let Some(raw) = url.query() else {
        return String::new();
    };
    if raw.is_empty() {
        return String::new();
    }
    let mut pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (aws_uri_encode(&k), aws_uri_encode(&v)))
        .collect();
    pairs.sort();
    pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// What one request's Signature Version 4 covers.
struct Signing<'a> {
    method: &'a str,
    /// The URL's path, already encoded.
    path: &'a str,
    /// The canonical query string.
    query: &'a str,
    /// The signed headers, in lower case, sorted by name, with trimmed
    /// values.
    headers: &'a [(&'a str, String)],
    /// Hex-encoded SHA-256 of the request body.
    payload_hash: &'a str,
    region: &'a str,
    service: &'a str,
    /// The request's time, as `YYYYMMDDTHHMMSSZ`.
    amz_date: &'a str,
}

/// The `Authorization` header value of `signing`'s request, signed by
/// `credentials`.
fn authorization(signing: &Signing<'_>, credentials: &Credentials) -> String {
    let hex = |bytes: &[u8]| data_encoding::HEXLOWER.encode(bytes);
    let canonical_headers =
        signing
            .headers
            .iter()
            .fold(String::new(), |mut lines, (name, value)| {
                lines.push_str(name);
                lines.push(':');
                lines.push_str(value.trim());
                lines.push('\n');
                lines
            });
    let signed_headers = signing
        .headers
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(";");
    let canonical_request = format!(
        "{}\n{}\n{}\n{canonical_headers}\n{signed_headers}\n{}",
        signing.method,
        if signing.path.is_empty() {
            "/"
        } else {
            signing.path
        },
        signing.query,
        signing.payload_hash
    );
    let date = signing.amz_date.get(..8).unwrap_or_default();
    let scope = format!("{date}/{}/{}/aws4_request", signing.region, signing.service);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        signing.amz_date,
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let key = format!("AWS4{}", credentials.secret_access_key.expose_secret());
    let signing_key = [date, signing.region, signing.service, "aws4_request"]
        .iter()
        .fold(
            secrecy::zeroize::Zeroizing::new(key.into_bytes()),
            |key, part| secrecy::zeroize::Zeroizing::new(hmac(&key, part.as_bytes())),
        );
    let signature = hex(&hmac(&signing_key, string_to_sign.as_bytes()));
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        credentials.access_key_id
    )
}

/// HMAC-SHA256 of `data` under `key`.
fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).expect("invariant: HMAC takes a key of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}
