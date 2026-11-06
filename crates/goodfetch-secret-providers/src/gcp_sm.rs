//! Google Cloud Secret Manager's provider, `gcp-sm:project/secret/version`:
//! the payload of the secret's version (`latest` for the newest), read
//! with `GET /v1/projects/<project>/secrets/<secret>/versions/<version>:access`
//! from `secret_providers.gcp_sm.endpoint`, with the OAuth 2.0 access token
//! in the engine's environment's `GOOGLE_OAUTH_ACCESS_TOKEN`. The API is an
//! Operator-configured endpoint reached through the engine's own HTTP
//! client ([`SecretProviderEndpoint`]).
//!
//! A translation point (ADR 0127): [`translate`] reads the `status` of the
//! Google API error a failed access answers.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::secret_endpoint::SecretProviderEndpoint;
use goodfetch_engine::secrets::{
    ConfigProblem, PartyText, ProviderFailure, Resolving, SecretProvider, retry_after,
};
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::tasks::TaskOwner;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::SECRET_PROVIDERS_GCP_SM_ENDPOINT;
use hyper::Request;
use hyper::body::Bytes;
use hyper::header::HeaderValue;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::endpoint::{json_of, server_url, unreached, variable};

/// The service's name in failures.
const SERVICE: &str = "GCP Secret Manager";

/// The variable that holds the access token.
const TOKEN_VARIABLE: &str = "GOOGLE_OAUTH_ACCESS_TOKEN";

/// The public API, where no setting names another.
const PUBLIC_ENDPOINT: &str = "https://secretmanager.googleapis.com";

/// Google Cloud Secret Manager's provider over its REST API.
#[derive(Clone, Debug)]
pub struct GcpSecretManager {
    endpoint_url: String,
    token: Option<SecretString>,
    endpoint: SecretProviderEndpoint,
}

impl GcpSecretManager {
    /// The provider `secret_providers.gcp_sm.endpoint` configures in
    /// `resolved`, which also gives how the API is reached, with the access
    /// token in the engine's `environment`.
    ///
    /// Its endpoint reads `tls.trusted_cas` as `owner`'s blocking work.
    pub fn configured(
        resolved: &Resolved,
        environment: &BTreeMap<OsString, OsString>,
        clock: Clock,
        owner: Arc<TaskOwner>,
    ) -> Self {
        Self::new(
            resolved
                .value(SECRET_PROVIDERS_GCP_SM_ENDPOINT.name)
                .as_str()
                .unwrap_or(PUBLIC_ENDPOINT)
                .to_owned(),
            environment,
            SecretProviderEndpoint::of(resolved, clock, owner),
        )
    }

    /// The provider reading from the API at `endpoint_url` through
    /// `endpoint`, with the access token in `environment`.
    pub fn new(
        endpoint_url: String,
        environment: &BTreeMap<OsString, OsString>,
        endpoint: SecretProviderEndpoint,
    ) -> Self {
        Self {
            endpoint_url,
            token: variable(environment, TOKEN_VARIABLE).map(SecretString::from),
            endpoint,
        }
    }

    /// The API's URL and the token's header, when both are usable.
    fn checked(&self) -> Result<(Url, HeaderValue), ConfigProblem> {
        let name = SECRET_PROVIDERS_GCP_SM_ENDPOINT.name;
        let url = server_url(&self.endpoint_url).map_err(|problem| ConfigProblem {
            fix: format!("`{name}` is `{}`, {problem}.", self.endpoint_url),
        })?;
        let Some(token) = &self.token else {
            return Err(ConfigProblem {
                fix: format!(
                    "Set `{TOKEN_VARIABLE}` in the engine's environment to an OAuth 2.0 access token, such as `gcloud auth print-access-token` prints, of an account with the Secret Manager Secret Accessor role."
                ),
            });
        };
        let mut header = HeaderValue::from_str(&format!("Bearer {}", token.expose_secret()))
            .map_err(|_| ConfigProblem {
                fix: format!(
                    "`{TOKEN_VARIABLE}` holds characters no HTTP header may carry; set it to the access token Google issued."
                ),
            })?;
        header.set_sensitive(true);
        Ok((url, header))
    }
}

/// The URL of the version's `access` on the API at `server`.
fn access_url(server: &Url, project: &str, secret: &str, version: &str) -> Url {
    let mut url = server.clone();
    url.path_segments_mut()
        .expect("invariant: an `http:` or `https:` URL has a path")
        .pop_if_empty()
        .extend([
            "v1",
            "projects",
            project,
            "secrets",
            secret,
            "versions",
            &format!("{version}:access"),
        ]);
    url
}

impl SecretProvider for GcpSecretManager {
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a> {
        Box::pin(async move {
            let (server, token) = self.checked().map_err(ProviderFailure::NotConfigured)?;
            let (project, rest) = reference
                .rest()
                .split_once('/')
                .expect("invariant: a `gcp-sm:` reference holds project/secret/version");
            let (secret, version) = rest
                .split_once('/')
                .expect("invariant: a `gcp-sm:` reference holds project/secret/version");
            let request = Request::get(access_url(&server, project, secret, version).as_str())
                .header("authorization", token)
                .body(Bytes::new())
                .expect("invariant: a parsed URL and checked headers make a request");
            let answer = self.endpoint.send(request).await.map_err(|failure| {
                unreached(
                    failure,
                    SERVICE,
                    &server,
                    SECRET_PROVIDERS_GCP_SM_ENDPOINT.name,
                )
            })?;
            let status = answer.status().as_u16();
            if status == 200 {
                payload_of(answer.body())
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

/// The payload a successful access answered, `body`, as text.
pub(crate) fn payload_of(body: &[u8]) -> Result<SecretString, ProviderFailure> {
    let answer = json_of(body, SERVICE)?;
    let Some(data) = answer
        .pointer("/payload/data")
        .and_then(|data| data.as_str())
    else {
        return Err(ProviderFailure::Missing);
    };
    let bytes = secrecy::zeroize::Zeroizing::new(
        data_encoding::BASE64
            .decode(data.as_bytes())
            .map_err(|_| unrecognized(400, "the payload is no Base64"))?,
    );
    let text = std::str::from_utf8(&bytes).map_err(|_| ProviderFailure::Refused {
        fix: format!(
            "{SERVICE}: the secret's payload is binary, not UTF-8 text; store it as text."
        ),
    })?;
    Ok(SecretString::from(text.to_owned()))
}

/// The failures the API is known to answer an access with, by the error's
/// `status`, and what each means.
const KNOWN: &[(&str, Meaning)] = &[
    ("NOT_FOUND", Meaning::Missing),
    ("PERMISSION_DENIED", Meaning::Denied),
    ("UNAUTHENTICATED", Meaning::Token),
    ("FAILED_PRECONDITION", Meaning::Disabled),
    ("INVALID_ARGUMENT", Meaning::Invalid),
    ("RESOURCE_EXHAUSTED", Meaning::Throttled),
    ("UNAVAILABLE", Meaning::ServiceFailed),
];

/// A known failure's meaning.
#[derive(Clone, Copy)]
enum Meaning {
    Missing,
    Denied,
    Token,
    Disabled,
    Invalid,
    Throttled,
    ServiceFailed,
}

/// What the API meant by answering an access with `status` and `body`.
pub(crate) fn translate(status: u16, body: &[u8]) -> ProviderFailure {
    let text = String::from_utf8_lossy(body).into_owned();
    let known = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|answer| answer.pointer("/error/status")?.as_str().map(str::to_owned))
        .and_then(|kind| {
            KNOWN
                .iter()
                .find(|(known, _)| *known == kind)
                .map(|(_, meaning)| *meaning)
        });
    let refused = |fix: &str| ProviderFailure::Refused {
        fix: format!("{SERVICE}: {fix}"),
    };
    let busy = |fix: &str| ProviderFailure::Busy {
        fix: format!("{SERVICE}: {fix}"),
        retry_after: None,
    };
    match known {
        Some(Meaning::Missing) => ProviderFailure::Missing,
        Some(Meaning::Denied) => refused(
            "the account of the access token may not access the secret; grant it the Secret Manager Secret Accessor role on the secret or its project.",
        ),
        Some(Meaning::Token) => refused(&format!(
            "the access token was refused; set `{TOKEN_VARIABLE}` in the engine's environment to a current token, such as `gcloud auth print-access-token` prints."
        )),
        Some(Meaning::Disabled) => refused(
            "the secret's version is disabled or destroyed; enable it, or name another version.",
        ),
        Some(Meaning::Invalid) => refused(
            "the reference names no valid project, secret or version; write it as `gcp-sm:project/secret/version`.",
        ),
        Some(Meaning::Throttled) => busy("the request quota was exceeded; try again later."),
        Some(Meaning::ServiceFailed) => busy("the service is unavailable; try again later."),
        None => unrecognized(status, &text),
    }
}

/// An unrecognized failure, logged with its status only.
fn unrecognized(status: u16, text: &str) -> ProviderFailure {
    tracing::warn!(
        provider = "gcp-sm",
        status,
        "an unrecognized failure of the Secret provider"
    );
    ProviderFailure::Unrecognized(PartyText::new(text))
}

#[cfg(test)]
mod tests;
