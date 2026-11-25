//! Azure Key Vault's provider, `azure-kv:vault/secret`: the current version
//! of the secret `secret` in the vault `vault`, read with
//! `GET <vault URL>/secrets/<secret>?api-version=7.5`, the vault's URL
//! `secret_providers.azure_kv.endpoint` with `{vault}` replaced by its name.
//! The access token comes from the identity the engine's environment gives
//! ([`access_token`]): a service principal's or a workload identity's,
//! which Microsoft Entra ID (`secret_providers.azure_kv.authority`) issues
//! by the client-credentials grant, or the host's managed identity. The
//! vault, the authority and the identity endpoints are Operator-configured
//! endpoints reached through the engine's own HTTP client
//! ([`SecretProviderEndpoint`]).
//!
//! Three translation points (ADR 0127): [`translate`] reads the
//! `error.code` Key Vault answers, and [`access_token`] the authority's and
//! the managed identity endpoints' failures.

mod access_token;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::secret_endpoint::SecretProviderEndpoint;
use goodfetch_engine::secrets::{
    ConfigProblem, PartyText, ProviderFailure, ProviderInputs, Resolving, SecretProvider,
};
use goodfetch_engine::tasks::BlockingThreads;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::{
    SECRET_PROVIDERS_AZURE_KV_AUTHORITY, SECRET_PROVIDERS_AZURE_KV_ENDPOINT,
};
use hyper::Request;
use hyper::body::Bytes;
use hyper::header::HeaderValue;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use self::access_token::Source;
use crate::endpoint::{json_of, server_url, text_of, unreached};
use crate::identity::Reach;

/// The service's name in failures.
const SERVICE: &str = "Azure Key Vault";

/// What stands for the vault's name in the endpoint's URL.
const VAULT: &str = "{vault}";

/// Key Vault's REST API version.
const API_VERSION: &str = "7.5";

/// Azure Key Vault's provider over its REST API.
#[derive(Clone, Debug)]
pub struct AzureKeyVault {
    endpoint_url: String,
    authority: String,
    token: Source,
    endpoint: SecretProviderEndpoint,
    blocking: BlockingThreads,
    clock: Clock,
}

impl AzureKeyVault {
    /// The provider `secret_providers.azure_kv.*` configure in a Run's
    /// `inputs`, which also give how the vault and the authority are
    /// reached, with the identity the engine's environment gives, its
    /// token file read on their blocking threads.
    pub fn configured(inputs: &ProviderInputs<'_>) -> Self {
        let text = |setting| inputs.settings.get::<&str>(setting).to_owned();
        Self {
            endpoint_url: text(&SECRET_PROVIDERS_AZURE_KV_ENDPOINT),
            authority: text(&SECRET_PROVIDERS_AZURE_KV_AUTHORITY),
            token: Source::of(inputs.environment),
            endpoint: SecretProviderEndpoint::of(inputs),
            blocking: inputs.blocking.clone(),
            clock: inputs.clock,
        }
    }

    /// The authority's URL and the token's source, when the settings and
    /// the environment are usable.
    fn checked(&self) -> Result<(Url, &Source), ConfigProblem> {
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
        self.token.check()?;
        Ok((authority, &self.token))
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
            let (authority, source) = self.checked().map_err(ProviderFailure::NotConfigured)?;
            let (vault, secret) = reference
                .rest()
                .split_once('/')
                .expect("invariant: an `azure-kv:` reference holds vault/secret");
            let mut url = self.vault_url(vault)?;
            let reach = Reach {
                endpoint: &self.endpoint,
                blocking: &self.blocking,
                clock: self.clock,
            };
            let token = source.token(&reach, &authority, &self.scope()).await?;
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
                Err(translate(status, answer.body(), source.describe())
                    .retrying_after(self.endpoint.retry_after(&answer)))
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

/// A known failure's meaning.
#[derive(Clone, Copy)]
enum Meaning {
    Missing,
    Denied,
    TokenRefused,
    Throttled,
}

/// What a known failure means, as a provider's failure, `from` naming the
/// identity the token was issued to.
fn failure_of(meaning: Meaning, from: &str) -> ProviderFailure {
    let refused = |fix: String| ProviderFailure::Refused {
        fix: format!("{SERVICE}: {fix}"),
    };
    match meaning {
        Meaning::Missing => ProviderFailure::Missing,
        Meaning::Denied => refused(format!(
            "{from} may not get the secret, or the secret is disabled; give it the Key Vault Secrets User role or a `get` access policy on secrets, and enable the secret."
        )),
        Meaning::TokenRefused => refused(format!(
            "the vault refused the access token of {from}; check that the identity belongs to the vault's tenant."
        )),
        Meaning::Throttled => ProviderFailure::Busy {
            fix: format!("{SERVICE}: the request rate was exceeded; try again later."),
            retry_after: None,
        },
    }
}

/// What Key Vault meant by answering with `status` and `body` a read
/// carrying the token of `from`, in a failure's words.
pub(crate) fn translate(status: u16, body: &[u8], from: &str) -> ProviderFailure {
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
            |(_, _, meaning)| failure_of(*meaning, from),
        )
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
