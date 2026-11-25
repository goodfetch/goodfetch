//! Where GCP Secret Manager's provider takes the OAuth 2.0 access token it
//! reads with (issue 1658), in the order Google's Application Default
//! Credentials look, gcloud's own file aside: the engine's environment's
//! token (`GOOGLE_OAUTH_ACCESS_TOKEN`); a service account's key file, which
//! `GOOGLE_APPLICATION_CREDENTIALS` names, whose RS256-signed JWT Google's
//! token endpoint exchanges for a token (RFC 7523); and the metadata server
//! of a Compute Engine instance, a GKE pod or a Cloud Run service, unless
//! `NO_GCE_CHECK` is `true`.
//!
//! The first source the environment names is the one taken, and its
//! failure is the provider's, naming it. Each identity endpoint is an
//! Operator-configured endpoint, reached through the provider's own
//! [`SecretProviderEndpoint`](goodfetch_engine::secret_endpoint::SecretProviderEndpoint):
//! the key file's `token_uri`, and the metadata server at the host
//! `GCE_METADATA_HOST` names, or its documented address. The metadata
//! server, which a host off Google Cloud lacks, is `not_configured` when it
//! gives no token within [`METADATA_WAIT`], or answers without
//! `Metadata-Flavor: Google`. A token is fetched for each value read, as
//! Azure Key Vault's is.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use data_encoding::BASE64URL_NOPAD;
use goodfetch_engine::secret_endpoint::EndpointFailure;
use goodfetch_engine::secrets::{ConfigProblem, PartyText, ProviderFailure};
use hyper::Request;
use hyper::body::Bytes;
use hyper::header::HeaderValue;
use rustls::SignatureScheme;
use rustls::pki_types::PrivateKeyDer;
use rustls::pki_types::pem::PemObject as _;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use crate::endpoint::{metadata_url, server_url, unreached, variable};
use crate::identity::{METADATA_WAIT, Reach, not_configured, unrecognized};

/// The variable that holds an access token.
const TOKEN_VARIABLE: &str = "GOOGLE_OAUTH_ACCESS_TOKEN";

/// The variable that names a service account's key file.
const KEY_FILE: &str = "GOOGLE_APPLICATION_CREDENTIALS";

/// The metadata server's documented address.
const METADATA: &str = "http://169.254.169.254";

/// The token endpoint a key file that names none is exchanged at.
const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";

/// The scope the token is asked for, which Secret Manager takes.
const SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

/// How long the signed JWT lasts, in seconds: the longest Google takes.
const ASSERTION_LIFETIME: u64 = 3600;

/// The services' names in failures.
const TOKEN_ENDPOINT: &str = "Google's token endpoint, exchanging the service account's key,";
const METADATA_SERVER: &str = "The GCP metadata server";

/// What makes a provider with no source answer.
const NO_SOURCE: &str = "Give the engine an account with the Secret Manager Secret Accessor role: set `GOOGLE_OAUTH_ACCESS_TOKEN` in its environment to an access token, such as `gcloud auth print-access-token` prints, or `GOOGLE_APPLICATION_CREDENTIALS` to a service account's key file, or run it on Google Cloud with a service account attached.";

/// Where the access token comes from.
#[derive(Clone, Debug)]
pub(crate) enum Source {
    /// The engine's environment's token.
    Environment(SecretString),
    /// A service account's key file at this path.
    KeyFile(PathBuf),
    /// The metadata server at `address`.
    Metadata { address: String },
    /// No source: the environment names none and turns the metadata server
    /// off.
    None,
}

impl Source {
    /// The source `environment` names.
    pub(crate) fn of(environment: &BTreeMap<OsString, OsString>) -> Self {
        let named = |name: &str| variable(environment, name);
        if let Some(token) = named(TOKEN_VARIABLE) {
            return Self::Environment(SecretString::from(token));
        }
        if let Some(file) = named(KEY_FILE) {
            return Self::KeyFile(PathBuf::from(file));
        }
        if named("NO_GCE_CHECK").is_some_and(|off| off.eq_ignore_ascii_case("true")) {
            return Self::None;
        }
        Self::Metadata {
            address: named("GCE_METADATA_HOST")
                .map_or_else(|| METADATA.to_owned(), |host| format!("http://{host}")),
        }
    }

    /// Whether the source can be asked: its token is one a header may
    /// carry, or its endpoint's address is one a request may go to.
    pub(crate) fn check(&self) -> Result<(), ConfigProblem> {
        match self {
            Self::Environment(token) => bearer(token).map(|_| ()).map_err(|()| ConfigProblem {
                fix: format!(
                    "`{TOKEN_VARIABLE}` holds characters no HTTP header may carry; set it to the access token Google issued."
                ),
            }),
            Self::KeyFile(_) => Ok(()),
            Self::Metadata { address } => {
                metadata_url(address)
                    .map(|_| ())
                    .map_err(|why| ConfigProblem {
                        fix: format!("`GCE_METADATA_HOST` gives `{address}`, {why}."),
                    })
            }
            Self::None => Err(ConfigProblem {
                fix: NO_SOURCE.to_owned(),
            }),
        }
    }

    /// Where the token came from, for a failure's fix.
    pub(crate) fn describe(&self) -> &'static str {
        match self {
            Self::Environment(_) | Self::None => "`GOOGLE_OAUTH_ACCESS_TOKEN`",
            Self::KeyFile(_) => {
                "the service account's key file `GOOGLE_APPLICATION_CREDENTIALS` names"
            }
            Self::Metadata { .. } => "the metadata server's service account",
        }
    }

    /// The `authorization` header carrying the token, fetched through
    /// `reach` when the source gives one.
    pub(crate) async fn header(&self, reach: &Reach<'_>) -> Result<HeaderValue, ProviderFailure> {
        match self {
            Self::Environment(token) => bearer(token).map_err(|()| {
                not_configured(format!(
                    "`{TOKEN_VARIABLE}` holds characters no HTTP header may carry; set it to the access token Google issued."
                ))
            }),
            Self::KeyFile(path) => {
                let token = exchanged(path, reach).await?;
                bearer(&token)
                    .map_err(|()| unrecognized(TOKEN_ENDPOINT, "a token no HTTP header may carry"))
            }
            Self::Metadata { address } => {
                let token = from_metadata(address, reach).await?;
                bearer(&token)
                    .map_err(|()| unrecognized(METADATA_SERVER, "a token no HTTP header may carry"))
            }
            Self::None => Err(not_configured(NO_SOURCE.to_owned())),
        }
    }
}

/// `token` as a sensitive `Bearer` header.
fn bearer(token: &SecretString) -> Result<HeaderValue, ()> {
    let mut header =
        HeaderValue::from_str(&format!("Bearer {}", token.expose_secret())).map_err(|_| ())?;
    header.set_sensitive(true);
    Ok(header)
}

/// A service account's key file, as Google writes it.
pub(crate) struct ServiceAccount {
    client_email: String,
    private_key: SecretString,
    private_key_id: Option<String>,
    token_uri: String,
}

/// The token Google's token endpoint exchanges the JWT the key file at
/// `path` signs for.
async fn exchanged(
    path: &std::path::Path,
    reach: &Reach<'_>,
) -> Result<SecretString, ProviderFailure> {
    let file = reach.read_file(path).await.map_err(|_| {
        not_configured(format!(
            "`{KEY_FILE}` names `{}`, which cannot be read; point it at a service account's key file.",
            path.display()
        ))
    })?;
    let account = service_account(file.expose_secret()).map_err(|problem| {
        not_configured(format!(
            "`{KEY_FILE}` names `{}`, {problem}; point it at a service account's JSON key file, as the console or `gcloud iam service-accounts keys create` writes it.",
            path.display()
        ))
    })?;
    let server = server_url(&account.token_uri).map_err(|why| {
        not_configured(format!(
            "The key file's `token_uri` is `{}`, {why}.",
            account.token_uri
        ))
    })?;
    let now = reach
        .clock
        .wall()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let assertion = assertion(&account, now).map_err(|problem| {
        not_configured(format!(
            "`{KEY_FILE}` names `{}`, whose private key {problem}; create a new key for the service account.",
            path.display()
        ))
    })?;
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer")
        .append_pair("assertion", assertion.expose_secret())
        .finish();
    let request = Request::post(server.as_str())
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Bytes::from(body))
        .expect("invariant: a parsed URL and constant headers make a request");
    let answer = reach
        .endpoint
        .send(request)
        .await
        .map_err(|failure| unreached(failure, TOKEN_ENDPOINT, &server, KEY_FILE))?;
    let status = answer.status().as_u16();
    if status == 200 {
        access_token(answer.body(), TOKEN_ENDPOINT)
    } else {
        Err(translate_token(status, answer.body())
            .retrying_after(reach.endpoint.retry_after(&answer)))
    }
}

/// The service account `text`, a key file, describes, or what keeps it
/// from being one, as a clause.
pub(crate) fn service_account(text: &str) -> Result<ServiceAccount, String> {
    let file: Value = serde_json::from_str(text).map_err(|_| "which holds no JSON".to_owned())?;
    let field = |name: &str| file.get(name).and_then(Value::as_str);
    match field("type") {
        Some("service_account") => {}
        Some(other) => {
            return Err(format!(
                "whose `type` is `{other}`, which the engine does not read"
            ));
        }
        None => return Err("which names no `type`".to_owned()),
    }
    let (Some(client_email), Some(private_key)) = (field("client_email"), field("private_key"))
    else {
        return Err("which lacks its `client_email` or `private_key`".to_owned());
    };
    Ok(ServiceAccount {
        client_email: client_email.to_owned(),
        private_key: SecretString::from(private_key.to_owned()),
        private_key_id: field("private_key_id").map(str::to_owned),
        token_uri: field("token_uri").unwrap_or(TOKEN_URI).to_owned(),
    })
}

/// The JWT `account` signs at `now`, in seconds since the epoch, with
/// RS256, which asks its token endpoint for a token of [`SCOPE`]; or what
/// keeps its private key from signing, as a clause.
pub(crate) fn assertion(account: &ServiceAccount, now: u64) -> Result<SecretString, &'static str> {
    let der = PrivateKeyDer::from_pem_slice(account.private_key.expose_secret().as_bytes())
        .map_err(|_| "is no PEM private key")?;
    let signer = rustls::crypto::ring::default_provider()
        .key_provider
        .load_private_key(der)
        .ok()
        .and_then(|key| key.choose_scheme(&[SignatureScheme::RSA_PKCS1_SHA256]))
        .ok_or("is no RSA key of 2048 bits or more")?;
    let mut header = json!({ "alg": "RS256", "typ": "JWT" });
    if let Some(id) = &account.private_key_id {
        header["kid"] = Value::from(id.as_str());
    }
    let claims = json!({
        "iss": account.client_email,
        "scope": SCOPE,
        "aud": account.token_uri,
        "iat": now,
        "exp": now + ASSERTION_LIFETIME,
    });
    let encode = |value: &Value| BASE64URL_NOPAD.encode(value.to_string().as_bytes());
    let signing_input = format!("{}.{}", encode(&header), encode(&claims));
    let signature = signer
        .sign(signing_input.as_bytes())
        .map_err(|_| "cannot sign")?;
    Ok(SecretString::from(format!(
        "{signing_input}.{}",
        BASE64URL_NOPAD.encode(&signature)
    )))
}

/// The token of the service account the metadata server at `address`
/// serves.
async fn from_metadata(address: &str, reach: &Reach<'_>) -> Result<SecretString, ProviderFailure> {
    let base = metadata_url(address)
        .map_err(|why| not_configured(format!("`GCE_METADATA_HOST` gives `{address}`, {why}.")))?;
    let absent = |what: &str| {
        not_configured(format!(
            "{METADATA_SERVER} at `{base}` {what}, so the host has no service account to take a token from. {NO_SOURCE}"
        ))
    };
    let url = base
        .join("computeMetadata/v1/instance/service-accounts/default/token")
        .expect("invariant: a relative path joins a server's URL");
    let request = Request::get(url.as_str())
        .header("metadata-flavor", "Google")
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
            failure => unreached(failure, METADATA_SERVER, &base, "GCE_METADATA_HOST"),
        })?;
    let flavor = answer
        .headers()
        .get("metadata-flavor")
        .is_some_and(|flavor| flavor == "Google");
    match answer.status().as_u16() {
        404 => Err(absent("has no service account")),
        _ if !flavor => Err(absent("answered without `Metadata-Flavor: Google`")),
        200 => access_token(answer.body(), METADATA_SERVER),
        status => Err(absent(&format!("answered HTTP {status}"))),
    }
}

/// The `access_token` of an OAuth 2.0 token answer, `body`, of `service`'s.
fn access_token(body: &[u8], service: &str) -> Result<SecretString, ProviderFailure> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|answer| {
            answer
                .get("access_token")
                .and_then(Value::as_str)
                .map(|token| SecretString::from(token.to_owned()))
        })
        .ok_or_else(|| unrecognized(service, "no `access_token`"))
}

/// The OAuth 2.0 errors Google's token endpoint is known to answer a JWT
/// with (RFC 6749, section 5.2; RFC 7523, section 3.1).
const KNOWN: &[(&str, Meaning)] = &[
    ("invalid_grant", Meaning::Key),
    ("invalid_client", Meaning::Key),
    ("unauthorized_client", Meaning::Key),
    ("access_denied", Meaning::Denied),
    ("temporarily_unavailable", Meaning::Busy),
];

/// A known token failure's meaning.
#[derive(Clone, Copy)]
enum Meaning {
    Key,
    Denied,
    Busy,
}

/// What Google's token endpoint meant by answering the JWT with `status`
/// and `body`.
pub(crate) fn translate_token(status: u16, body: &[u8]) -> ProviderFailure {
    let error = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|answer| answer.get("error")?.as_str().map(str::to_owned));
    let known = error
        .as_deref()
        .and_then(|error| KNOWN.iter().find(|(known, _)| *known == error))
        .map(|(_, meaning)| *meaning);
    let refused = |fix: &str| ProviderFailure::Refused {
        fix: format!("{TOKEN_ENDPOINT} {fix}"),
    };
    match known {
        Some(Meaning::Key) => refused(&format!(
            "refused the key of `{KEY_FILE}`; check that the key and its service account are neither deleted nor disabled, and that the host's clock is right."
        )),
        Some(Meaning::Denied) => refused(
            "would not give the service account a token; check that it may use the `cloud-platform` scope.",
        ),
        Some(Meaning::Busy) => ProviderFailure::Busy {
            fix: format!("{TOKEN_ENDPOINT} is briefly unable to answer; try again later."),
            retry_after: None,
        },
        None => {
            tracing::warn!(
                provider = "gcp-sm",
                status,
                "an unrecognized failure of Google's token endpoint"
            );
            ProviderFailure::Unrecognized(PartyText::new(String::from_utf8_lossy(body)))
        }
    }
}
