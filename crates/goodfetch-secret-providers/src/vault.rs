//! `HashiCorp` Vault's and `OpenBao`'s provider, `vault:mount/path#field`:
//! `field` of the KV version 2 secret `path` in the secrets engine mounted
//! at `mount`, read with `GET /v1/<mount>/data/<path>` from the server
//! `secret_providers.vault.address` names, signed in by the token in the
//! engine's environment, `VAULT_TOKEN` or `OpenBao`'s `BAO_TOKEN`. The server
//! is an Operator-configured endpoint reached through the engine's own
//! HTTP client ([`SecretProviderEndpoint`]): the egress policy judges its
//! addresses under `egress.no_private_endpoints`, and its certificate
//! chains to Mozilla's roots or its own domain's `tls.trusted_cas`, never
//! to an exception of `tls.accept_invalid_certs` (ADRs 0017 and 0042).
//!
//! A translation point (ADR 0127): [`translate`] reads the status and
//! `errors` a server answers a failed read with, as Vault 1.20 and `OpenBao`
//! 2.4 write them.

use std::collections::BTreeMap;
use std::ffi::OsString;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::secret_endpoint::{EndpointFailure, SecretProviderEndpoint};
use goodfetch_engine::secrets::{
    ConfigProblem, PartyText, ProviderFailure, Resolving, SecretProvider, retry_after,
};
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::SECRET_PROVIDERS_VAULT_ADDRESS;
use hyper::Request;
use hyper::body::Bytes;
use hyper::header::HeaderValue;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::endpoint::server_url;

/// The environment variables that hold the token, in the order they are
/// read: Vault's, then `OpenBao`'s.
const TOKEN_VARIABLES: [&str; 2] = ["VAULT_TOKEN", "BAO_TOKEN"];

/// The header that carries the token.
const TOKEN_HEADER: &str = "x-vault-token";

/// Vault's and `OpenBao`'s provider over their HTTP API.
#[derive(Clone, Debug)]
pub struct Vault {
    /// `secret_providers.vault.address`, if set.
    address: Option<String>,
    /// The token from the engine's environment, if any.
    token: Option<SecretString>,
    /// How the server is reached.
    endpoint: SecretProviderEndpoint,
}

impl Vault {
    /// The provider `secret_providers.vault.address` configures in
    /// `resolved`, which also gives how the server is reached, signed in by
    /// the token in the engine's `environment`.
    pub fn configured(
        resolved: &Resolved,
        environment: &BTreeMap<OsString, OsString>,
        clock: Clock,
    ) -> Self {
        Self::new(
            resolved
                .value(SECRET_PROVIDERS_VAULT_ADDRESS.name)
                .as_str()
                .map(str::to_owned),
            environment,
            SecretProviderEndpoint::of(resolved, clock),
        )
    }

    /// The provider reading from the server at `address`, as
    /// `secret_providers.vault.address` names it, through `endpoint`, with
    /// the token in `environment`.
    pub fn new(
        address: Option<String>,
        environment: &BTreeMap<OsString, OsString>,
        endpoint: SecretProviderEndpoint,
    ) -> Self {
        let token = TOKEN_VARIABLES.iter().find_map(|name| {
            environment
                .get(&OsString::from(name))
                .and_then(|value| value.to_str())
                .filter(|value| !value.is_empty())
                .map(|value| SecretString::from(value.to_owned()))
        });
        Self {
            address,
            token,
            endpoint,
        }
    }

    /// The server's URL and the token's header, when both are usable.
    fn checked(&self) -> Result<(Url, HeaderValue), ConfigProblem> {
        let name = SECRET_PROVIDERS_VAULT_ADDRESS.name;
        let Some(address) = self.address.as_deref() else {
            return Err(ConfigProblem {
                fix: format!(
                    "Set `{name}` in configuration or the Operator policy to the URL of the Vault or OpenBao server, such as `https://vault.example.com:8200`."
                ),
            });
        };
        let url = server_url(address).map_err(|problem| ConfigProblem {
            fix: format!("`{name}` is `{address}`, {problem}."),
        })?;
        let Some(token) = &self.token else {
            return Err(ConfigProblem {
                fix: "Set `VAULT_TOKEN`, or `BAO_TOKEN`, in the engine's environment to a token whose policy may read the secret.".to_owned(),
            });
        };
        let mut header = HeaderValue::from_str(token.expose_secret()).map_err(|_| ConfigProblem {
            fix: "The token in `VAULT_TOKEN` or `BAO_TOKEN` holds characters no HTTP header may carry; set it to the token Vault or OpenBao issued.".to_owned(),
        })?;
        header.set_sensitive(true);
        Ok((url, header))
    }
}

/// The URL of `GET /v1/<mount>/data/<path>` on the server at `server`.
fn read_url(server: &Url, mount: &str, path: &str) -> Url {
    let mut url = server.clone();
    url.path_segments_mut()
        .expect("invariant: an `http:` or `https:` URL has a path")
        .pop_if_empty()
        .push("v1")
        .push(mount)
        .push("data")
        .extend(path.split('/'));
    url
}

impl SecretProvider for Vault {
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a> {
        Box::pin(async move {
            let (server, token) = self.checked().map_err(ProviderFailure::NotConfigured)?;
            let (secret, field) = reference
                .rest()
                .split_once('#')
                .expect("invariant: a `vault:` reference holds `#field`");
            let (mount, path) = secret
                .split_once('/')
                .expect("invariant: a `vault:` reference holds `mount/path`");
            let url = read_url(&server, mount, path);
            let request = Request::get(url.as_str())
                .header(TOKEN_HEADER, token)
                .header("x-vault-request", "true")
                .body(Bytes::new())
                .expect("invariant: a parsed URL and fixed headers make a request");
            let answer = self
                .endpoint
                .send(request)
                .await
                .map_err(|failure| unreached(failure, &server))?;
            let status = answer.status().as_u16();
            if status == 200 {
                field_of(answer.body(), field)
            } else {
                Err(translate(status, answer.body()).retrying_after(retry_after(
                    answer
                        .headers()
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok()),
                )))
            }
        })
    }

    fn check_config(&self) -> Result<(), ConfigProblem> {
        self.checked().map(|_| ())
    }
}

/// `field` of the secret a successful read answered, `body`: a text
/// value as it is, any other JSON value as its JSON text.
pub(crate) fn field_of(body: &[u8], field: &str) -> Result<SecretString, ProviderFailure> {
    let Ok(answer) = serde_json::from_slice::<serde_json::Value>(body) else {
        tracing::warn!(provider = "vault", "the Secret provider answered no JSON");
        return Err(ProviderFailure::Unrecognized(PartyText::new(
            String::from_utf8_lossy(body).into_owned(),
        )));
    };
    match answer
        .pointer("/data/data")
        .and_then(|data| data.get(field))
    {
        None | Some(serde_json::Value::Null) => Err(ProviderFailure::Missing),
        Some(serde_json::Value::String(text)) => Ok(SecretString::from(text.clone())),
        Some(other) => Ok(SecretString::from(other.to_string())),
    }
}

/// What a request that reached no answer means.
fn unreached(failure: EndpointFailure, server: &Url) -> ProviderFailure {
    let refused = |fix: String| ProviderFailure::Refused { fix };
    match failure {
        EndpointFailure::Refused(refused_by) => refused(format!(
            "The Vault server at `{server}` is refused by the egress policy: {refused_by}"
        )),
        EndpointFailure::CertificateInvalid(invalid) => refused(format!(
            "The Vault server at `{server}` presented a certificate that failed verification ({invalid}); list the authority that signed it under `tls.trusted_cas` for its host. `tls.accept_invalid_certs` makes no exception for a Secret provider."
        )),
        EndpointFailure::Network => refused(format!(
            "The Vault server at `{server}` did not answer; check `{}` and the network, then try again.",
            SECRET_PROVIDERS_VAULT_ADDRESS.name
        )),
        failure => {
            tracing::warn!(provider = "vault", "the Secret provider's endpoint failed");
            ProviderFailure::Unrecognized(PartyText::new(failure.to_string()))
        }
    }
}

/// The failures a server is known to answer a read with: the status, the
/// texts one of whose `errors` must hold, matched without regard to case
/// (any text when none is listed), and what it means. The first that
/// matches wins.
const KNOWN: &[(u16, &[&str], Meaning)] = &[
    (404, &["no handler for route"], Meaning::NoSecretsEngine),
    (404, &[], Meaning::Missing),
    (
        403,
        &["permission denied", "invalid token"],
        Meaning::TokenRefused,
    ),
    (400, &["missing client token"], Meaning::TokenRefused),
    (503, &["vault is sealed"], Meaning::Sealed),
    (429, &["rate limit quota exceeded"], Meaning::RateLimited),
];

/// A known failure's meaning.
#[derive(Clone, Copy)]
enum Meaning {
    Missing,
    NoSecretsEngine,
    TokenRefused,
    Sealed,
    RateLimited,
}

/// What a server meant by answering a read with `status` and `body`.
pub(crate) fn translate(status: u16, body: &[u8]) -> ProviderFailure {
    let text = String::from_utf8_lossy(body).into_owned();
    let lower = text.to_lowercase();
    let known = KNOWN
        .iter()
        .find(|(known_status, texts, _)| {
            *known_status == status
                && (texts.is_empty() || texts.iter().any(|known| lower.contains(known)))
        })
        .map(|(_, _, meaning)| *meaning);
    let refused = |fix: &str| ProviderFailure::Refused {
        fix: fix.to_owned(),
    };
    match known {
        Some(Meaning::Missing) => ProviderFailure::Missing,
        Some(Meaning::NoSecretsEngine) => refused(
            "No secrets engine is mounted where the reference's `mount` names; name the mount of a KV version 2 secrets engine.",
        ),
        Some(Meaning::TokenRefused) => refused(
            "The server refused the token: set `VAULT_TOKEN`, or `BAO_TOKEN`, in the engine's environment to a token that is valid and whose policy may read the secret's `data/` path.",
        ),
        Some(Meaning::Sealed) => refused("The server is sealed; unseal it, then try again."),
        Some(Meaning::RateLimited) => ProviderFailure::Busy {
            fix: "The server's rate limit quota refused the read; try again later, or raise the quota."
                .to_owned(),
            retry_after: None,
        },
        None => {
            tracing::warn!(
                provider = "vault",
                status,
                "an unrecognized failure of the Secret provider"
            );
            ProviderFailure::Unrecognized(PartyText::new(text))
        }
    }
}

#[cfg(test)]
mod tests;
