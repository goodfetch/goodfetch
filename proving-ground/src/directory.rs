//! Web Bot Auth signature verification (draft
//! `meunier-web-bot-auth-architecture`, on HTTP Message Signatures, RFC
//! 9421), as a request arrives, with Cloudflare's `web-bot-auth`: the
//! signature must be tagged `web-bot-auth`, cover `@authority` (or
//! `@target-uri`) and `Signature-Agent` (plus any extra headers a caller
//! requires, such as `crawler-exact-price` or `payment-signature`), be
//! unexpired, and verify against a key of the directory its `Signature-Agent`
//! names. That directory is read as the lab's identity site read it: fetched
//! at arrival from its `http://` URL
//! (`/.well-known/http-message-signatures-directory` added to an origin),
//! served as `application/http-message-signatures-directory+json`, every key
//! named by its RFC 7638 thumbprint and proven by the directory response's
//! own signature (tag `http-message-signatures-directory`). A `data:`
//! directory is read inline. A directory the Proving ground cannot fetch
//! refuses the signature; nothing else is reached.
#![expect(
    clippy::disallowed_types,
    reason = "the Proving ground fetches an agent's key directory itself, outside the engine's transport (ADR 0060)"
)]

use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use web_bot_auth::SignatureAgentLink;
use web_bot_auth::WebBotAuthVerifier;
use web_bot_auth::components::{
    CoveredComponent, DerivedComponent, HTTPField, HTTPFieldParameters,
};
use web_bot_auth::keyring::{JSONWebKeySet, KeyRing, Thumbprintable};
use web_bot_auth::message_signatures::{MessageVerifier, SignedMessage};

use crate::observation::Verification;

const DIRECTORY_PATH: &str = "/.well-known/http-message-signatures-directory";
const DIRECTORY_TYPE: &str = "application/http-message-signatures-directory+json";

/// How long fetching a directory may take.
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// The largest directory read.
const DIRECTORY_LIMIT: usize = 256 * 1024;

/// A verified Web Bot Auth signature's metadata, including its replay-tracking
/// token (the signature's nonce, or keyid + created when absent).
pub(crate) struct VerifiedSignature {
    pub(crate) key_id: String,
    pub(crate) replay_token: String,
    pub(crate) covered_headers: Vec<String>,
}

/// Checks `headers` for a Web Bot Auth signature on `(authority, method, target)`.
/// Returns `None` when no signature header is present.
pub(crate) async fn verify_headers(
    authority: &str,
    method: &str,
    target: &str,
    headers: &[(String, String)],
) -> Option<Verification> {
    let signed = ["signature", "signature-input", "signature-agent"]
        .iter()
        .any(|name| {
            headers
                .iter()
                .any(|(field, _)| field.eq_ignore_ascii_case(name))
        });
    if !signed {
        return None;
    }
    Some(match check(authority, method, target, headers).await {
        Ok(verified) => Verification {
            verified: true,
            key_id: Some(verified.key_id),
            error: None,
        },
        Err(error) => Verification {
            verified: false,
            key_id: None,
            error: Some(error),
        },
    })
}

/// Verifies a request's Web Bot Auth signature and returns its key ID, replay
/// token, and the lowercase names of all HTTP headers covered by the signature.
pub(crate) async fn check(
    authority: &str,
    method: &str,
    target: &str,
    headers: &[(String, String)],
) -> Result<VerifiedSignature, String> {
    let message = Incoming {
        authority,
        method,
        target,
        headers,
    };
    let verifier = WebBotAuthVerifier::parse(&message).map_err(|error| error.to_string())?;
    let parsed = verifier.get_parsed_label();
    let details = &parsed.base.parameters.details;
    if details.possibly_insecure(|_| false).is_expired != Some(false) {
        return Err("the signature has expired or has no expiry".to_owned());
    }
    let key_id = details
        .keyid
        .clone()
        .ok_or_else(|| "the signature names no key".to_owned())?;
    let covered_headers = parsed
        .base
        .components
        .iter()
        .filter_map(|(component, _)| match component {
            CoveredComponent::HTTP(field) => Some(field.name.to_ascii_lowercase()),
            CoveredComponent::Derived(_) => None,
        })
        .collect();
    let replay_token = details.nonce.clone().unwrap_or_else(|| {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("signature"))
            .map_or_else(|| format!("{key_id}:{target}"), |(_, value)| value.clone())
    });
    let keys = match verifier.get_signature_agents().first() {
        Some(SignatureAgentLink::Inline(keys)) => keys.clone(),
        Some(SignatureAgentLink::External(url)) => {
            tokio::time::timeout(FETCH_TIMEOUT, directory(url))
                .await
                .map_err(|_| "the key directory did not answer in time".to_owned())??
        }
        None => return Err("no Signature-Agent names a key directory".to_owned()),
    };
    let mut keyring = KeyRing::default();
    keyring.import_jwks(keys);
    verifier
        .verify(&keyring, None)
        .map_err(|error| error.to_string())?;
    Ok(VerifiedSignature {
        key_id,
        replay_token,
        covered_headers,
    })
}

/// The keys of the directory at `url`, each proven by the directory's own
/// signature.
async fn directory(url: &str) -> Result<JSONWebKeySet, String> {
    let url = if url.ends_with(DIRECTORY_PATH) {
        url.to_owned()
    } else {
        format!("{}{DIRECTORY_PATH}", url.trim_end_matches('/'))
    };
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("the key directory {url} is not an http:// URL"))?;
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let address = if authority.contains(':') {
        authority.to_owned()
    } else {
        format!("{authority}:80")
    };
    let response = get(&address, authority, path)
        .await
        .map_err(|error| format!("fetching the key directory {url}: {error}"))?;
    if response.status != 200 {
        return Err(format!("the key directory answered {}", response.status));
    }
    if !response
        .header("content-type")
        .is_some_and(|kind| kind.starts_with(DIRECTORY_TYPE))
    {
        return Err("the key directory is not served as a directory".to_owned());
    }
    let named: Named = serde_json::from_slice(&response.body)
        .map_err(|_| "the key directory is not a key set".to_owned())?;
    let keys: JSONWebKeySet = serde_json::from_slice(&response.body)
        .map_err(|_| "the key directory is not a key set".to_owned())?;
    let mut keyring = KeyRing::default();
    for (key, kid) in keys.keys.iter().zip(&named.keys) {
        if kid.kid.as_deref() != Some(key.b64_thumbprint().as_str()) {
            return Err("a directory key's ID is not its thumbprint".to_owned());
        }
        keyring
            .try_import_jwk(key)
            .map_err(|_| "a directory key cannot be read".to_owned())?;
    }
    let proven = prove(&response, authority, &keyring);
    if let Some(key) = keys
        .keys
        .iter()
        .map(Thumbprintable::b64_thumbprint)
        .find(|kid| !proven.contains(kid))
    {
        return Err(format!(
            "no proof of possession for the directory key {key}"
        ));
    }
    Ok(keys)
}

/// The IDs of the keys whose signature of the directory response verifies.
fn prove(response: &Response, authority: &str, keyring: &KeyRing) -> Vec<String> {
    let message = Directory {
        response,
        authority,
    };
    let labels: Vec<String> = response
        .header("signature-input")
        .map(|input| {
            input
                .split(',')
                .filter_map(|member| member.split_once("=("))
                .map(|(label, _)| label.trim().to_owned())
                .collect()
        })
        .unwrap_or_default();
    labels
        .iter()
        .filter_map(|label| {
            let verifier = MessageVerifier::parse(&message, |(key, inner)| {
                key.as_str() == label
                    && inner
                        .params
                        .get("tag")
                        .and_then(|tag| tag.as_string())
                        .is_some_and(|tag| tag.as_str() == "http-message-signatures-directory")
            })
            .ok()?;
            let key = verifier.parsed.base.parameters.details.keyid.clone()?;
            verifier.verify(keyring, None).ok().map(|_| key)
        })
        .collect()
}

/// The `kid` of each key, which the library's key set leaves out.
#[derive(Deserialize)]
struct Named {
    keys: Vec<Kid>,
}

#[derive(Deserialize)]
struct Kid {
    kid: Option<String>,
}

/// A response's status, header fields (names in lowercase) and body.
struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }
}

/// One plain HTTP/1.1 `GET`, read to the connection's close.
async fn get(address: &str, authority: &str, path: &str) -> std::io::Result<Response> {
    let mut stream = TcpStream::connect(address).await?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nAccept: {DIRECTORY_TYPE}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;
    let mut bytes = Vec::new();
    (&mut stream)
        .take(DIRECTORY_LIMIT as u64)
        .read_to_end(&mut bytes)
        .await?;
    let invalid = || std::io::Error::other("an unreadable response");
    let at = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(invalid)?;
    let head = std::str::from_utf8(&bytes[..at]).map_err(|_| invalid())?;
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(invalid)?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let mut body = bytes[at + 4..].to_vec();
    if headers
        .iter()
        .any(|(name, value)| name == "transfer-encoding" && value.eq_ignore_ascii_case("chunked"))
    {
        body = dechunk(&body).ok_or_else(invalid)?;
    }
    Ok(Response {
        status,
        headers,
        body,
    })
}

/// A chunked body's data.
fn dechunk(mut bytes: &[u8]) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let at = bytes.windows(2).position(|window| window == b"\r\n")?;
        let size = usize::from_str_radix(
            std::str::from_utf8(&bytes[..at])
                .ok()?
                .split(';')
                .next()?
                .trim(),
            16,
        )
        .ok()?;
        bytes = &bytes[at + 2..];
        if size == 0 {
            return Some(body);
        }
        body.extend_from_slice(bytes.get(..size)?);
        bytes = bytes.get(size + 2..)?;
    }
}

/// The values of header `field` among `headers`, or of its dictionary
/// member when the component names one (`;key=`).
fn field_values(headers: &[(String, String)], field: &HTTPField) -> Vec<String> {
    let values = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(&field.name))
        .map(|(_, value)| value.clone());
    let key = field
        .parameters
        .0
        .iter()
        .find_map(|parameter| match parameter {
            HTTPFieldParameters::Key(key) => Some(key.clone()),
            _ => None,
        });
    match key {
        None => values.collect(),
        Some(key) => values
            .flat_map(|value| {
                value
                    .split(',')
                    .filter_map(|member| member.trim().split_once('='))
                    .filter(|(name, _)| *name == key)
                    .map(|(_, value)| value.to_owned())
                    .collect::<Vec<_>>()
            })
            .collect(),
    }
}

/// A request as the signature's components read it, over TLS.
struct Incoming<'a> {
    authority: &'a str,
    method: &'a str,
    target: &'a str,
    headers: &'a [(String, String)],
}

impl SignedMessage for Incoming<'_> {
    fn lookup_component(&self, name: &CoveredComponent) -> Vec<String> {
        let path = self
            .target
            .split_once('?')
            .map_or(self.target, |(path, _)| path);
        let one = |value: String| vec![value];
        match name {
            CoveredComponent::HTTP(field) => field_values(self.headers, field),
            CoveredComponent::Derived(derived) => match derived {
                DerivedComponent::Authority { .. } => one(self.authority.to_ascii_lowercase()),
                DerivedComponent::TargetUri { .. } => one(format!(
                    "https://{}{}",
                    self.authority.to_ascii_lowercase(),
                    self.target
                )),
                DerivedComponent::RequestTarget { .. } => one(self.target.to_owned()),
                DerivedComponent::Method { .. } => one(self.method.to_owned()),
                DerivedComponent::Path { .. } => one(path.to_owned()),
                DerivedComponent::Scheme { .. } => one("https".to_owned()),
                DerivedComponent::Query { .. } => one(format!(
                    "?{}",
                    self.target.split_once('?').map_or("", |(_, query)| query)
                )),
                DerivedComponent::QueryParams { .. } | DerivedComponent::Status { .. } => {
                    Vec::new()
                }
            },
        }
    }
}

/// A directory response as its proof's components read it.
struct Directory<'a> {
    response: &'a Response,
    authority: &'a str,
}

impl SignedMessage for Directory<'_> {
    fn lookup_component(&self, name: &CoveredComponent) -> Vec<String> {
        match name {
            CoveredComponent::HTTP(field) => field_values(&self.response.headers, field),
            CoveredComponent::Derived(DerivedComponent::Authority { .. }) => {
                vec![self.authority.to_ascii_lowercase()]
            }
            CoveredComponent::Derived(DerivedComponent::Status { .. }) => {
                vec![self.response.status.to_string()]
            }
            CoveredComponent::Derived(_) => Vec::new(),
        }
    }
}
