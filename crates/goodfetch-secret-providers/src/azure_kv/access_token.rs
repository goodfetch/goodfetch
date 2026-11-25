//! Where Azure Key Vault's provider takes the access token it reads with
//! (issue 1658), in the order of the Azure Identity libraries'
//! `DefaultAzureCredential`, its developer tools aside: a service
//! principal's client secret (`AZURE_TENANT_ID`, `AZURE_CLIENT_ID`,
//! `AZURE_CLIENT_SECRET`); a workload identity's federated token, which
//! `AZURE_FEDERATED_TOKEN_FILE` holds, as Microsoft Entra ID's client
//! assertion; and the managed identity of the App Service, Functions or
//! Container Apps host (`IDENTITY_ENDPOINT`, `IDENTITY_HEADER`), or else of
//! the virtual machine, through the instance metadata service.
//! `AZURE_TOKEN_CREDENTIALS` keeps only the sources it names, as the Azure
//! Identity libraries read it.
//!
//! The first source the environment names is the one taken, and its
//! failure is the provider's, naming it. Each identity endpoint is an
//! Operator-configured endpoint, reached through the provider's own
//! [`SecretProviderEndpoint`](goodfetch_engine::secret_endpoint::SecretProviderEndpoint).
//! The instance metadata service, which a host off Azure lacks, is
//! `not_configured` when it gives no token within [`METADATA_WAIT`], or
//! answers in no shape of Azure's. A token is fetched for each value read.
//!
//! Two translation points (ADR 0127): [`translate_token`] reads the OAuth
//! 2.0 `error` the authority answers, and [`translate_identity`] the status
//! a managed identity endpoint answers.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use goodfetch_engine::secret_endpoint::EndpointFailure;
use goodfetch_engine::secrets::{ConfigProblem, PartyText, ProviderFailure};
use goodfetch_protocol::secrets::SECRET_PROVIDERS_AZURE_KV_AUTHORITY;
use hyper::Request;
use hyper::body::Bytes;
use hyper::header::HeaderValue;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use url::Url;

use crate::endpoint::{metadata_url, unreached, variable};
use crate::identity::{METADATA_WAIT, Reach, access_token, not_configured, unrecognized};

/// The authority's name in failures.
const AUTHORITY: &str = "Microsoft Entra ID";

/// The managed identity endpoints' names in failures.
const APP_SERVICE: &str = "The managed identity endpoint `IDENTITY_ENDPOINT` names";
const IMDS: &str = "The Azure instance metadata service";

/// The instance metadata service's documented address.
const METADATA: &str = "http://169.254.169.254";

/// The variable that keeps only the sources it names.
const CHOICE: &str = "AZURE_TOKEN_CREDENTIALS";

/// The values the Azure Identity libraries take for [`CHOICE`], in lower
/// case: both kinds of credentials, and each credential's name.
const CHOICES: [&str; 12] = [
    "prod",
    "dev",
    "environmentcredential",
    "workloadidentitycredential",
    "managedidentitycredential",
    "azureclicredential",
    "azuredeveloperclicredential",
    "azurepowershellcredential",
    "brokercredential",
    "interactivebrowsercredential",
    "visualstudiocredential",
    "visualstudiocodecredential",
];

/// What makes a provider with no source answer.
const NO_SOURCE: &str = "Give the engine an identity that may get the vault's secrets: set `AZURE_TENANT_ID`, `AZURE_CLIENT_ID` and `AZURE_CLIENT_SECRET` in its environment to a service principal's, or `AZURE_FEDERATED_TOKEN_FILE` with the first two to a workload identity's, or run it on Azure with a managed identity; `AZURE_TOKEN_CREDENTIALS`, when set, must keep the source used.";

/// Where the access token comes from.
#[derive(Clone)]
pub(crate) enum Source {
    /// A service principal's client secret.
    ServicePrincipal {
        tenant: String,
        client_id: String,
        secret: SecretString,
    },
    /// A workload identity's federated token, in the file at `token_file`.
    WorkloadIdentity {
        tenant: String,
        client_id: String,
        token_file: PathBuf,
    },
    /// The host's managed identity endpoint at `endpoint`, which asks for
    /// `header`.
    AppService {
        endpoint: String,
        header: SecretString,
        client_id: Option<String>,
    },
    /// The instance metadata service at `address`.
    Imds {
        address: String,
        client_id: Option<String>,
    },
    /// `AZURE_TOKEN_CREDENTIALS` holds a value the Azure Identity libraries
    /// do not take.
    Unknown(String),
    /// No source: the environment names none and keeps no managed identity.
    None,
}

impl std::fmt::Debug for Source {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.describe())
    }
}

impl Source {
    /// The source `environment` names.
    pub(crate) fn of(environment: &BTreeMap<OsString, OsString>) -> Self {
        let named = |name: &str| variable(environment, name);
        let choice = named(CHOICE).map(|choice| choice.trim().to_ascii_lowercase());
        if let Some(choice) = &choice
            && !CHOICES.contains(&choice.as_str())
        {
            return Self::Unknown(choice.clone());
        }
        let keeps = |credential: &str| {
            choice
                .as_deref()
                .is_none_or(|choice| choice == "prod" || choice == credential)
        };
        let (tenant, client_id) = (named("AZURE_TENANT_ID"), named("AZURE_CLIENT_ID"));
        if let (true, Some(tenant), Some(client_id), Some(secret)) = (
            keeps("environmentcredential"),
            &tenant,
            &client_id,
            named("AZURE_CLIENT_SECRET"),
        ) {
            return Self::ServicePrincipal {
                tenant: tenant.clone(),
                client_id: client_id.clone(),
                secret: SecretString::from(secret),
            };
        }
        if let (true, Some(tenant), Some(client_id), Some(file)) = (
            keeps("workloadidentitycredential"),
            &tenant,
            &client_id,
            named("AZURE_FEDERATED_TOKEN_FILE"),
        ) {
            return Self::WorkloadIdentity {
                tenant: tenant.clone(),
                client_id: client_id.clone(),
                token_file: PathBuf::from(file),
            };
        }
        if !keeps("managedidentitycredential") {
            return Self::None;
        }
        match (named("IDENTITY_ENDPOINT"), named("IDENTITY_HEADER")) {
            (Some(endpoint), Some(header)) => Self::AppService {
                endpoint,
                header: SecretString::from(header),
                client_id,
            },
            _ => Self::Imds {
                address: named("AZURE_POD_IDENTITY_AUTHORITY_HOST")
                    .unwrap_or_else(|| METADATA.to_owned()),
                client_id,
            },
        }
    }

    /// Whether the source can be asked: its endpoint's address is one a
    /// request may go to, and its header one a request may carry.
    pub(crate) fn check(&self) -> Result<(), ConfigProblem> {
        let problem = |fix: String| Err(ConfigProblem { fix });
        match self {
            Self::ServicePrincipal { .. } | Self::WorkloadIdentity { .. } => Ok(()),
            Self::AppService {
                endpoint, header, ..
            } => {
                if let Err(why) = metadata_url(endpoint) {
                    return problem(format!("`IDENTITY_ENDPOINT` is `{endpoint}`, {why}."));
                }
                if HeaderValue::from_str(header.expose_secret()).is_err() {
                    return problem(
                        "`IDENTITY_HEADER` holds characters no HTTP header may carry; leave it as the host sets it.".to_owned(),
                    );
                }
                Ok(())
            }
            Self::Imds { address, .. } => {
                metadata_url(address)
                    .map(|_| ())
                    .map_err(|why| ConfigProblem {
                        fix: format!("`AZURE_POD_IDENTITY_AUTHORITY_HOST` is `{address}`, {why}."),
                    })
            }
            Self::Unknown(choice) => problem(format!(
                "`{CHOICE}` is `{choice}`, which the Azure Identity libraries do not take; set it to `prod`, `EnvironmentCredential`, `WorkloadIdentityCredential` or `ManagedIdentityCredential`, or unset it."
            )),
            Self::None => problem(NO_SOURCE.to_owned()),
        }
    }

    /// Where the token came from, for a failure's fix.
    pub(crate) fn describe(&self) -> &'static str {
        match self {
            Self::ServicePrincipal { .. } => {
                "the service principal of `AZURE_TENANT_ID`, `AZURE_CLIENT_ID` and `AZURE_CLIENT_SECRET`"
            }
            Self::WorkloadIdentity { .. } => {
                "the workload identity of `AZURE_FEDERATED_TOKEN_FILE`"
            }
            Self::AppService { .. } => "the host's managed identity, through `IDENTITY_ENDPOINT`",
            Self::Imds { .. } => "the virtual machine's managed identity",
            Self::Unknown(_) | Self::None => "no identity",
        }
    }

    /// An access token for `scope`, fetched through `reach`, from
    /// `authority` for a service principal or a workload identity.
    pub(crate) async fn token(
        &self,
        reach: &Reach<'_>,
        authority: &Url,
        scope: &str,
    ) -> Result<SecretString, ProviderFailure> {
        let resource = scope.strip_suffix("/.default").unwrap_or(scope);
        match self {
            Self::ServicePrincipal {
                tenant,
                client_id,
                secret,
            } => {
                let pairs = [
                    ("client_id", client_id.as_str()),
                    ("client_secret", secret.expose_secret()),
                ];
                from_authority(reach, authority, tenant, scope, &pairs, self.describe()).await
            }
            Self::WorkloadIdentity {
                tenant,
                client_id,
                token_file,
            } => {
                let assertion = federated_token(reach, token_file).await?;
                let pairs = [
                    ("client_id", client_id.as_str()),
                    (
                        "client_assertion_type",
                        "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
                    ),
                    ("client_assertion", assertion.expose_secret()),
                ];
                from_authority(reach, authority, tenant, scope, &pairs, self.describe()).await
            }
            Self::AppService {
                endpoint,
                header,
                client_id,
            } => from_app_service(reach, endpoint, header, client_id.as_deref(), resource).await,
            Self::Imds { address, client_id } => {
                from_imds(reach, address, client_id.as_deref(), resource).await
            }
            Self::Unknown(_) | Self::None => Err(not_configured(match self.check() {
                Err(problem) => problem.fix,
                Ok(()) => NO_SOURCE.to_owned(),
            })),
        }
    }
}

/// The federated token in the file at `path`.
async fn federated_token(reach: &Reach<'_>, path: &Path) -> Result<SecretString, ProviderFailure> {
    let token = reach.read_file(path).await.map_err(|_| {
        not_configured(format!(
            "`AZURE_FEDERATED_TOKEN_FILE` names `{}`, which cannot be read; point it at the token file the workload identity's host projects.",
            path.display()
        ))
    })?;
    if token.expose_secret().is_empty() {
        return Err(not_configured(format!(
            "`AZURE_FEDERATED_TOKEN_FILE` names `{}`, which is empty; point it at the token file the workload identity's host projects.",
            path.display()
        )));
    }
    Ok(token)
}

/// A token of `scope` the authority at `authority` issues to the client in
/// `tenant` that `pairs` sign in, by the client-credentials grant; `from`
/// names the client in a failure.
async fn from_authority(
    reach: &Reach<'_>,
    authority: &Url,
    tenant: &str,
    scope: &str,
    pairs: &[(&str, &str)],
    from: &str,
) -> Result<SecretString, ProviderFailure> {
    let mut url = authority.clone();
    url.path_segments_mut()
        .expect("invariant: an `http:` or `https:` URL has a path")
        .pop_if_empty()
        .extend([tenant, "oauth2", "v2.0", "token"]);
    let body = secrecy::zeroize::Zeroizing::new(
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "client_credentials")
            .extend_pairs(pairs)
            .append_pair("scope", scope)
            .finish(),
    );
    let request = Request::post(url.as_str())
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Bytes::from(body.as_bytes().to_vec()))
        .expect("invariant: a parsed URL and fixed headers make a request");
    let answer = reach.endpoint.send(request).await.map_err(|failure| {
        unreached(
            failure,
            AUTHORITY,
            authority,
            SECRET_PROVIDERS_AZURE_KV_AUTHORITY.name,
        )
    })?;
    let status = answer.status().as_u16();
    if status != 200 {
        return Err(translate_token(status, answer.body(), from)
            .retrying_after(reach.endpoint.retry_after(&answer)));
    }
    access_token(answer.body(), AUTHORITY)
}

/// A token for `resource` from the host's managed identity endpoint at
/// `endpoint`, for the user-assigned identity `client_id` when one is
/// named.
async fn from_app_service(
    reach: &Reach<'_>,
    endpoint: &str,
    header: &SecretString,
    client_id: Option<&str>,
    resource: &str,
) -> Result<SecretString, ProviderFailure> {
    let mut url = metadata_url(endpoint)
        .map_err(|why| not_configured(format!("`IDENTITY_ENDPOINT` is `{endpoint}`, {why}.")))?;
    let base = url.clone();
    url.query_pairs_mut()
        .append_pair("api-version", "2019-08-01")
        .append_pair("resource", resource)
        .extend_pairs(client_id.map(|id| ("client_id", id)));
    let mut identity = HeaderValue::from_str(header.expose_secret()).map_err(|_| {
        not_configured(
            "`IDENTITY_HEADER` holds characters no HTTP header may carry; leave it as the host sets it.".to_owned(),
        )
    })?;
    identity.set_sensitive(true);
    let request = Request::get(url.as_str())
        .header("x-identity-header", identity)
        .body(Bytes::new())
        .expect("invariant: a parsed URL and checked headers make a request");
    let answer = reach
        .endpoint
        .send(request)
        .await
        .map_err(|failure| unreached(failure, APP_SERVICE, &base, "IDENTITY_ENDPOINT"))?;
    let status = answer.status().as_u16();
    if status == 200 {
        return access_token(answer.body(), APP_SERVICE);
    }
    Err(
        translate_identity(status, answer.body(), APP_SERVICE).map_or_else(
            || unrecognized(APP_SERVICE, "in no shape of Azure's"),
            |failure| failure.retrying_after(reach.endpoint.retry_after(&answer)),
        ),
    )
}

/// A token for `resource` from the instance metadata service at `address`,
/// for the user-assigned identity `client_id` when one is named.
async fn from_imds(
    reach: &Reach<'_>,
    address: &str,
    client_id: Option<&str>,
    resource: &str,
) -> Result<SecretString, ProviderFailure> {
    let base = metadata_url(address).map_err(|why| {
        not_configured(format!(
            "`AZURE_POD_IDENTITY_AUTHORITY_HOST` is `{address}`, {why}."
        ))
    })?;
    let absent = |what: &str| {
        not_configured(format!(
            "{IMDS} at `{base}` {what}, so the host has no managed identity to take a token from. {NO_SOURCE}"
        ))
    };
    let mut url = base
        .join("metadata/identity/oauth2/token")
        .expect("invariant: a relative path joins a server's URL");
    url.query_pairs_mut()
        .append_pair("api-version", "2018-02-01")
        .append_pair("resource", resource)
        .extend_pairs(client_id.map(|id| ("client_id", id)));
    let request = Request::get(url.as_str())
        .header("metadata", "true")
        .body(Bytes::new())
        .expect("invariant: a joined URL and constant headers make a request");
    let answer = reach
        .promptly(reach.endpoint.send(request))
        .await
        .ok_or_else(|| {
            absent(&format!(
                "gave no token within {} s",
                METADATA_WAIT.as_secs()
            ))
        })?
        .map_err(|failure| match failure {
            EndpointFailure::Network => absent("did not answer"),
            failure => unreached(failure, IMDS, &base, "AZURE_POD_IDENTITY_AUTHORITY_HOST"),
        })?;
    let status = answer.status().as_u16();
    if status == 200 {
        return access_token(answer.body(), IMDS);
    }
    match translate_identity(status, answer.body(), IMDS) {
        Some(ProviderFailure::NotConfigured(_)) => Err(absent(
            "has no managed identity of the client `AZURE_CLIENT_ID` names, or none assigned",
        )),
        Some(failure) => Err(failure.retrying_after(reach.endpoint.retry_after(&answer))),
        None => Err(absent(&format!(
            "answered HTTP {status} in no shape of Azure's"
        ))),
    }
}

/// The OAuth 2.0 errors the authority is known to answer, and what each
/// means.
const KNOWN_TOKEN: &[(&str, Meaning)] = &[
    ("invalid_client", Meaning::ClientRefused),
    ("unauthorized_client", Meaning::ClientRefused),
    ("invalid_request", Meaning::ClientRefused),
    ("invalid_scope", Meaning::Scope),
    ("temporarily_unavailable", Meaning::Throttled),
];

/// The statuses a managed identity endpoint is known to answer a token
/// request with, in Azure's shape, and what each means.
const KNOWN_IDENTITY: &[(u16, Meaning)] = &[
    (400, Meaning::NoIdentity),
    (404, Meaning::Updating),
    (410, Meaning::Updating),
    (429, Meaning::Throttled),
    (500, Meaning::Updating),
    (503, Meaning::Updating),
];

/// A known token failure's meaning.
#[derive(Clone, Copy)]
enum Meaning {
    ClientRefused,
    Scope,
    Throttled,
    NoIdentity,
    Updating,
}

/// What a known token failure of `service`'s means, `from` naming the
/// identity asked for.
fn failure_of(meaning: Meaning, service: &str, from: &str) -> ProviderFailure {
    let busy = |why: &str| ProviderFailure::Busy {
        fix: format!("{service}: {why}; try again later."),
        retry_after: None,
    };
    match meaning {
        Meaning::ClientRefused => ProviderFailure::Refused {
            fix: format!(
                "{service}: {from} was refused; check that `AZURE_TENANT_ID` and `AZURE_CLIENT_ID` name an app registered in that tenant, and that its credential is current and trusted by the app."
            ),
        },
        Meaning::Scope => ProviderFailure::Refused {
            fix: format!(
                "{service}: the Key Vault scope was refused; check that `secret_providers.azure_kv.endpoint` and `secret_providers.azure_kv.authority` name the same cloud."
            ),
        },
        Meaning::Throttled => busy("the request rate was exceeded"),
        Meaning::NoIdentity => not_configured(format!(
            "{service} has no managed identity of the client `AZURE_CLIENT_ID` names, or none assigned; assign the host a managed identity, or name an assigned one in `AZURE_CLIENT_ID`."
        )),
        Meaning::Updating => busy("it is briefly unable to give a token"),
    }
}

/// What the authority meant by answering a token request for `from` with
/// `status` and `body`.
pub(crate) fn translate_token(status: u16, body: &[u8], from: &str) -> ProviderFailure {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|answer| answer.get("error")?.as_str().map(str::to_owned))
        .and_then(|error| {
            KNOWN_TOKEN
                .iter()
                .find(|(known, _)| *known == error)
                .map(|(_, meaning)| *meaning)
        })
        .map_or_else(
            || {
                tracing::warn!(
                    provider = "azure-kv",
                    status,
                    "an unrecognized failure of the authority"
                );
                ProviderFailure::Unrecognized(PartyText::new(String::from_utf8_lossy(body)))
            },
            |meaning| failure_of(meaning, AUTHORITY, from),
        )
}

/// What a managed identity endpoint, `service`, meant by answering a token
/// request with `status` and `body`; none when the answer is in no shape
/// of Azure's, a JSON object with its `error`.
pub(crate) fn translate_identity(
    status: u16,
    body: &[u8],
    service: &str,
) -> Option<ProviderFailure> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|answer| answer.get("error").map(Value::is_string))
        .filter(|&shaped| shaped)?;
    Some(
        KNOWN_IDENTITY
            .iter()
            .find(|(known, _)| *known == status)
            .map_or_else(
                || {
                    tracing::warn!(
                        provider = "azure-kv",
                        status,
                        "an unrecognized failure of a managed identity endpoint"
                    );
                    ProviderFailure::Unrecognized(PartyText::new(String::from_utf8_lossy(body)))
                },
                |(_, meaning)| failure_of(*meaning, service, "the managed identity"),
            ),
    )
}
