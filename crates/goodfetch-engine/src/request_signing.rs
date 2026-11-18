//! Web Bot Auth (ADRs 0021 and 0045; #1551): an `identified` Session signs
//! each request it sends with its Ed25519 key, by Cloudflare's
//! `web-bot-auth`, as RFC 9421's HTTP Message Signatures over the request's
//! `@authority` and its `Signature-Agent`, which names where a site reads
//! the key directory. The key is resolved once, when the Session opens,
//! from its Secret reference; it is never written anywhere, and dropping
//! the [`RequestSigner`] zeroes it.

use std::fmt;
use std::sync::PoisonError;
use std::time::Duration;

use data_encoding::{BASE64, BASE64URL_NOPAD};
use ed25519_dalek::{Signer as _, SigningKey};
use indexmap::IndexMap;
use secrecy::zeroize::Zeroize as _;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
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

/// What one request's signature covers, and the headers it is given.
struct Covered {
    components: IndexMap<CoveredComponent, String>,
    input: String,
    signature: String,
}

impl UnsignedMessage for Covered {
    fn fetch_components_to_cover(&self) -> IndexMap<CoveredComponent, String> {
        self.components.clone()
    }

    fn register_header_contents(&mut self, signature_input: String, signature_header: String) {
        self.input = format!("{LABEL}={signature_input}");
        self.signature = format!("{LABEL}={signature_header}");
    }
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
        if !agent
            .chars()
            .all(|character| matches!(character, ' '..='~') && !matches!(character, '"' | '\\'))
        {
            return Err(KeyError::BadAgent);
        }
        let key_id = Thumbprintable::OKP {
            crv: "Ed25519".to_owned(),
            x: public,
        }
        .b64_thumbprint();
        Ok(Self {
            key: Key(signing),
            key_id,
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
        let mut covered = Covered {
            components: IndexMap::from([
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
            input: String::new(),
            signature: String::new(),
        };
        #[expect(
            clippy::disallowed_methods,
            reason = "`web-bot-auth` dates a signature by the wall clock itself, which the site that checks it judges it by (ADR 0045)"
        )]
        MessageSigner {
            keyid: self.key_id.clone(),
            nonce: self.nonce(),
            tag: TAG.to_owned(),
        }
        .generate_signature_headers_content(&mut covered, VALID_FOR, Algorithm::Ed25519, &self.key)
        .expect("invariant: what a signature covers is printable ASCII: the agent was checked when the signer was made, the key ID and the nonce are Base64, and the authority is a parsed URL's");
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
