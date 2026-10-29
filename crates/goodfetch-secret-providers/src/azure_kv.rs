//! Azure Key Vault's provider, `azure-kv:vault/secret`: the current version
//! of the secret `secret` in the vault `vault`, read with
//! `GET <vault URL>/secrets/<secret>?api-version=7.5`, the vault's URL
//! `secret_providers.azure_kv.endpoint` with `{vault}` replaced by its name.
//! The access token comes from Microsoft Entra ID
//! (`secret_providers.azure_kv.authority`) by the client-credentials grant,
//! for the service principal in the engine's environment's
//! `AZURE_TENANT_ID`, `AZURE_CLIENT_ID` and `AZURE_CLIENT_SECRET`. Both are
//! Operator-configured endpoints reached through the engine's own HTTP
//! client ([`SecretProviderEndpoint`]).
//!
//! Two translation points (ADR 0127): [`translate_token`] reads the OAuth
//! 2.0 `error` the authority answers, and [`translate`] the `error.code`
//! Key Vault answers.

use std::collections::BTreeMap;
use std::ffi::OsString;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::secret_endpoint::SecretProviderEndpoint;
use goodfetch_engine::secrets::{
    ConfigProblem, PartyText, ProviderFailure, Resolving, SecretProvider, retry_after,
};
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::{
    SECRET_PROVIDERS_AZURE_KV_AUTHORITY, SECRET_PROVIDERS_AZURE_KV_ENDPOINT,
};
use hyper::Request;
use hyper::body::Bytes;
use hyper::header::HeaderValue;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::endpoint::{json_of, server_url, text_of, unreached, variable};

/// The service's name in failures.
const SERVICE: &str = "Azure Key Vault";

/// The authority's name in failures.
const AUTHORITY: &str = "Microsoft Entra ID";

/// What stands for the vault's name in the endpoint's URL.
const VAULT: &str = "{vault}";

/// Key Vault's REST API version.
const API_VERSION: &str = "7.5";

/// The public cloud's defaults, where no setting names others.
const PUBLIC_ENDPOINT: &str = "https://{vault}.vault.azure.net";
const PUBLIC_AUTHORITY: &str = "https://login.microsoftonline.com";

/// The service principal the token is issued to.
#[derive(Clone)]
struct ServicePrincipal {
    tenant: String,
    client_id: String,
    client_secret: SecretString,
}

impl std::fmt::Debug for ServicePrincipal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServicePrincipal")
            .field("tenant", &self.tenant)
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

/// Azure Key Vault's provider over its REST API.
#[derive(Clone, Debug)]
pub struct AzureKeyVault {
    endpoint_url: String,
    authority: String,
    principal: Option<ServicePrincipal>,
    endpoint: SecretProviderEndpoint,
}

impl AzureKeyVault {
    /// The provider `secret_providers.azure_kv.*` configure in `resolved`,
    /// which also gives how the vault and the authority are reached, for
    /// the service principal in the engine's `environment`.
    pub fn configured(
        resolved: &Resolved,
        environment: &BTreeMap<OsString, OsString>,
        clock: Clock,
    ) -> Self {
        let text =
            |name: &str, default: &str| resolved.value(name).as_str().unwrap_or(default).to_owned();
        Self::new(
            text(SECRET_PROVIDERS_AZURE_KV_ENDPOINT.name, PUBLIC_ENDPOINT),
            text(SECRET_PROVIDERS_AZURE_KV_AUTHORITY.name, PUBLIC_AUTHORITY),
            environment,
            SecretProviderEndpoint::of(resolved, clock),
        )
    }

    /// The provider reading from the vaults `endpoint_url` names, with a
    /// token from `authority`, through `endpoint`, for the service
    /// principal in `environment`.
    pub fn new(
        endpoint_url: String,
        authority: String,
        environment: &BTreeMap<OsString, OsString>,
        endpoint: SecretProviderEndpoint,
    ) -> Self {
        let principal = match (
            variable(environment, "AZURE_TENANT_ID"),
            variable(environment, "AZURE_CLIENT_ID"),
            variable(environment, "AZURE_CLIENT_SECRET"),
        ) {
            (Some(tenant), Some(client_id), Some(secret)) => Some(ServicePrincipal {
                tenant,
                client_id,
                client_secret: SecretString::from(secret),
            }),
            _ => None,
        };
        Self {
            endpoint_url,
            authority,
            principal,
            endpoint,
        }
    }

    /// The authority's URL and the service principal, when the settings
    /// and the environment are usable.
    fn checked(&self) -> Result<(Url, &ServicePrincipal), ConfigProblem> {
        let endpoint = SECRET_PROVIDERS_AZURE_KV_ENDPOINT.name;
        if !self.endpoint_url.contains(VAULT) {
            return Err(ConfigProblem {
                fix: format!(
                    "`{endpoint}` is `{}`, which has no `{VAULT}` for the vault's name; write it as `https://{VAULT}.vault.azure.net`.",
                    self.endpoint_url
                ),
            });
        }
        server_url(&self.endpoint_url.replace(VAULT, "vault")).map_err(|problem| {
            ConfigProblem {
                fix: format!("`{endpoint}` is `{}`, {problem}.", self.endpoint_url),
            }
        })?;
        let authority = server_url(&self.authority).map_err(|problem| ConfigProblem {
            fix: format!(
                "`{}` is `{}`, {problem}.",
                SECRET_PROVIDERS_AZURE_KV_AUTHORITY.name, self.authority
            ),
        })?;
        let Some(principal) = &self.principal else {
            return Err(ConfigProblem {
                fix: "Set `AZURE_TENANT_ID`, `AZURE_CLIENT_ID` and `AZURE_CLIENT_SECRET` in the engine's environment to a service principal that may get the vault's secrets.".to_owned(),
            });
        };
        Ok((authority, principal))
    }

    /// The vault `vault`'s URL.
    fn vault_url(&self, vault: &str) -> Result<Url, ProviderFailure> {
        if !is_vault_name(vault) {
            return Err(ProviderFailure::Refused {
                fix: format!(
                    "{SERVICE}: `{vault}` is no vault name, which holds 3 to 24 letters, digits and dashes and starts with a letter."
                ),
            });
        }
        server_url(&self.endpoint_url.replace(VAULT, vault)).map_err(|problem| {
            ProviderFailure::NotConfigured(ConfigProblem {
                fix: format!(
                    "`{}` is `{}`, {problem}.",
                    SECRET_PROVIDERS_AZURE_KV_ENDPOINT.name, self.endpoint_url
                ),
            })
        })
    }

    /// The scope of a Key Vault token: the public cloud's, or the vault
    /// domain's that the endpoint's URL names after `{vault}.`.
    fn scope(&self) -> String {
        let domain = self
            .endpoint_url
            .split_once("{vault}.")
            .and_then(|(_, rest)| rest.split(['/', ':']).next())
            .filter(|domain| !domain.is_empty())
            .unwrap_or("vault.azure.net");
        format!("https://{domain}/.default")
    }

    /// An access token for Key Vault from the authority at `authority`.
    async fn token(
        &self,
        authority: &Url,
        principal: &ServicePrincipal,
    ) -> Result<SecretString, ProviderFailure> {
        let mut url = authority.clone();
        url.path_segments_mut()
            .expect("invariant: an `http:` or `https:` URL has a path")
            .pop_if_empty()
            .extend([principal.tenant.as_str(), "oauth2", "v2.0", "token"]);
        let body = secrecy::zeroize::Zeroizing::new(
            url::form_urlencoded::Serializer::new(String::new())
                .append_pair("grant_type", "client_credentials")
                .append_pair("client_id", &principal.client_id)
                .append_pair("client_secret", principal.client_secret.expose_secret())
                .append_pair("scope", &self.scope())
                .finish(),
        );
        let request = Request::post(url.as_str())
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Bytes::from(body.as_bytes().to_vec()))
            .expect("invariant: a parsed URL and fixed headers make a request");
        let answer = self.endpoint.send(request).await.map_err(|failure| {
            unreached(
                failure,
                AUTHORITY,
                authority,
                SECRET_PROVIDERS_AZURE_KV_AUTHORITY.name,
            )
        })?;
        let status = answer.status().as_u16();
        if status != 200 {
            return Err(translate_token(status, answer.body()));
        }
        json_of(answer.body(), AUTHORITY)?
            .get("access_token")
            .and_then(|token| token.as_str())
            .map(|token| SecretString::from(token.to_owned()))
            .ok_or_else(|| unrecognized("azure-kv", status, "no access_token"))
    }
}

/// Whether `name` is a vault's name: 3 to 24 letters, digits and dashes,
/// starting with a letter, so it can stand in a host name.
fn is_vault_name(name: &str) -> bool {
    (3..=24).contains(&name.len())
        && name.starts_with(|first: char| first.is_ascii_alphabetic())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

impl SecretProvider for AzureKeyVault {
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a> {
        Box::pin(async move {
            let (authority, principal) = self.checked().map_err(ProviderFailure::NotConfigured)?;
            let (vault, secret) = reference
                .rest()
                .split_once('/')
                .expect("invariant: an `azure-kv:` reference holds vault/secret");
            let mut url = self.vault_url(vault)?;
            let token = self.token(&authority, principal).await?;
            url.path_segments_mut()
                .expect("invariant: an `http:` or `https:` URL has a path")
                .pop_if_empty()
                .extend(["secrets", secret]);
            url.query_pairs_mut()
                .append_pair("api-version", API_VERSION);
            let mut header = HeaderValue::from_str(&format!("Bearer {}", token.expose_secret()))
                .map_err(|_| unrecognized("azure-kv", 200, "a token no header may carry"))?;
            header.set_sensitive(true);
            let request = Request::get(url.as_str())
                .header("authorization", header)
                .body(Bytes::new())
                .expect("invariant: a parsed URL and checked headers make a request");
            let answer = self.endpoint.send(request).await.map_err(|failure| {
                unreached(
                    failure,
                    SERVICE,
                    &url,
                    SECRET_PROVIDERS_AZURE_KV_ENDPOINT.name,
                )
            })?;
            let status = answer.status().as_u16();
            if status == 200 {
                value_of(answer.body())
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

/// The value a successful read answered, `body`.
pub(crate) fn value_of(body: &[u8]) -> Result<SecretString, ProviderFailure> {
    text_of(json_of(body, SERVICE)?.get("value"))
        .map(SecretString::from)
        .ok_or(ProviderFailure::Missing)
}

/// The failures Key Vault is known to answer a read with, by status and
/// `error.code` (any code when none is given), and what each means. The
/// first that matches wins.
const KNOWN: &[(u16, Option<&str>, Meaning)] = &[
    (404, Some("SecretNotFound"), Meaning::Missing),
    (404, Some("NotFound"), Meaning::Missing),
    (403, Some("Forbidden"), Meaning::Denied),
    (401, Some("Unauthorized"), Meaning::TokenRefused),
    (429, Some("Throttled"), Meaning::Throttled),
];

/// The OAuth 2.0 errors the authority is known to answer, and what each
/// means.
const KNOWN_TOKEN: &[(&str, Meaning)] = &[
    ("invalid_client", Meaning::ClientRefused),
    ("unauthorized_client", Meaning::ClientRefused),
    ("invalid_request", Meaning::ClientRefused),
    ("invalid_scope", Meaning::Scope),
    ("temporarily_unavailable", Meaning::Throttled),
];

/// A known failure's meaning.
#[derive(Clone, Copy)]
enum Meaning {
    Missing,
    Denied,
    TokenRefused,
    ClientRefused,
    Scope,
    Throttled,
}

/// What a known failure means, as a provider's failure.
fn failure_of(meaning: Meaning) -> ProviderFailure {
    let refused = |service: &str, fix: &str| ProviderFailure::Refused {
        fix: format!("{service}: {fix}"),
    };
    match meaning {
        Meaning::Missing => ProviderFailure::Missing,
        Meaning::Denied => refused(
            SERVICE,
            "the service principal may not get the secret, or the secret is disabled; give it the Key Vault Secrets User role or a `get` access policy on secrets, and enable the secret.",
        ),
        Meaning::TokenRefused => refused(
            SERVICE,
            "the vault refused the access token; check that `AZURE_TENANT_ID` is the vault's tenant.",
        ),
        Meaning::ClientRefused => refused(
            AUTHORITY,
            "the service principal was refused; set `AZURE_TENANT_ID`, `AZURE_CLIENT_ID` and `AZURE_CLIENT_SECRET` in the engine's environment to a current client secret of an app registered in that tenant.",
        ),
        Meaning::Scope => refused(
            AUTHORITY,
            "the Key Vault scope was refused; check that `secret_providers.azure_kv.endpoint` and `secret_providers.azure_kv.authority` name the same cloud.",
        ),
        Meaning::Throttled => ProviderFailure::Busy {
            fix: format!("{SERVICE}: the request rate was exceeded; try again later."),
            retry_after: None,
        },
    }
}

/// What Key Vault meant by answering a read with `status` and `body`.
pub(crate) fn translate(status: u16, body: &[u8]) -> ProviderFailure {
    let text = String::from_utf8_lossy(body).into_owned();
    let code = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|answer| answer.pointer("/error/code")?.as_str().map(str::to_owned));
    KNOWN
        .iter()
        .find(|(known_status, known_code, _)| {
            *known_status == status && known_code.is_none_or(|known| Some(known) == code.as_deref())
        })
        .map_or_else(
            || unrecognized("azure-kv", status, &text),
            |(_, _, meaning)| failure_of(*meaning),
        )
}

/// What the authority meant by answering a token request with `status`
/// and `body`.
pub(crate) fn translate_token(status: u16, body: &[u8]) -> ProviderFailure {
    let text = String::from_utf8_lossy(body).into_owned();
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|answer| answer.get("error")?.as_str().map(str::to_owned))
        .and_then(|error| {
            KNOWN_TOKEN
                .iter()
                .find(|(known, _)| *known == error)
                .map(|(_, meaning)| *meaning)
        })
        .map_or_else(|| unrecognized("azure-kv", status, &text), failure_of)
}

/// An unrecognized failure, logged with its status only.
fn unrecognized(provider: &str, status: u16, text: &str) -> ProviderFailure {
    tracing::warn!(
        provider,
        status,
        "an unrecognized failure of the Secret provider"
    );
    ProviderFailure::Unrecognized(PartyText::new(text))
}

#[cfg(test)]
mod tests;
