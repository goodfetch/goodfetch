//! AWS Secrets Manager's provider, `aws-sm:name#key` or `aws-sm:name`: the
//! JSON object's `key` of the secret `name`, or its whole text, read with
//! `GetSecretValue` from the Region `secret_providers.aws_sm.region` names
//! (else the engine's environment's `AWS_REGION`, then
//! `AWS_DEFAULT_REGION`), at `secret_providers.aws_sm.endpoint` or the
//! Region's public endpoint. Requests are signed with Signature Version 4
//! by the access key in the engine's environment (`AWS_ACCESS_KEY_ID`,
//! `AWS_SECRET_ACCESS_KEY`, and `AWS_SESSION_TOKEN` for temporary
//! credentials). The endpoint is an Operator-configured endpoint reached
//! through the engine's own HTTP client ([`SecretProviderEndpoint`]).
//!
//! A translation point (ADR 0127): [`translate`] reads the `__type` an
//! `awsJson1.1` failure names.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::time::SystemTime;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::secret_endpoint::SecretProviderEndpoint;
use goodfetch_engine::secrets::{
    ConfigProblem, PartyText, ProviderFailure, Resolving, SecretProvider,
};
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::{
    SECRET_PROVIDERS_AWS_SM_ENDPOINT, SECRET_PROVIDERS_AWS_SM_REGION,
};
use hmac::{Hmac, Mac as _};
use hyper::Request;
use hyper::body::Bytes;
use hyper::header::HeaderValue;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use url::Url;

use crate::endpoint::{json_of, server_url, text_of, unreached, variable};

/// The service's name in failures.
const SERVICE: &str = "AWS Secrets Manager";

/// The signing name of Secrets Manager.
const SIGNING_NAME: &str = "secretsmanager";

/// The operation's target header.
const TARGET: &str = "secretsmanager.GetSecretValue";

/// The protocol's content type.
const CONTENT_TYPE: &str = "application/x-amz-json-1.1";

/// An access key: its ID, its secret and, for temporary credentials, the
/// session token.
#[derive(Clone)]
pub(crate) struct Credentials {
    pub(crate) access_key_id: String,
    pub(crate) secret_access_key: SecretString,
    pub(crate) session_token: Option<SecretString>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("access_key_id", &self.access_key_id)
            .finish_non_exhaustive()
    }
}

/// AWS Secrets Manager's provider over its HTTP API.
#[derive(Clone, Debug)]
pub struct AwsSecretsManager {
    region: Option<String>,
    endpoint_url: Option<String>,
    credentials: Option<Credentials>,
    endpoint: SecretProviderEndpoint,
    clock: Clock,
}

impl AwsSecretsManager {
    /// The provider `secret_providers.aws_sm.*` configure in `resolved`,
    /// which also gives how the endpoint is reached, signed by the access
    /// key in the engine's `environment`.
    pub fn configured(
        resolved: &Resolved,
        environment: &BTreeMap<OsString, OsString>,
        clock: Clock,
    ) -> Self {
        let text = |name: &str| resolved.value(name).as_str().map(str::to_owned);
        Self::new(
            text(SECRET_PROVIDERS_AWS_SM_REGION.name),
            text(SECRET_PROVIDERS_AWS_SM_ENDPOINT.name),
            environment,
            SecretProviderEndpoint::of(resolved, clock),
            clock,
        )
    }

    /// The provider reading from `region`'s Secrets Manager, at
    /// `endpoint_url` when one is given, through `endpoint`, signed by the
    /// access key in `environment` at `clock`'s time.
    pub fn new(
        region: Option<String>,
        endpoint_url: Option<String>,
        environment: &BTreeMap<OsString, OsString>,
        endpoint: SecretProviderEndpoint,
        clock: Clock,
    ) -> Self {
        let region = region
            .or_else(|| variable(environment, "AWS_REGION"))
            .or_else(|| variable(environment, "AWS_DEFAULT_REGION"));
        let credentials = match (
            variable(environment, "AWS_ACCESS_KEY_ID"),
            variable(environment, "AWS_SECRET_ACCESS_KEY"),
        ) {
            (Some(access_key_id), Some(secret)) => Some(Credentials {
                access_key_id,
                secret_access_key: SecretString::from(secret),
                session_token: variable(environment, "AWS_SESSION_TOKEN").map(SecretString::from),
            }),
            _ => None,
        };
        Self {
            region,
            endpoint_url,
            credentials,
            endpoint,
            clock,
        }
    }

    /// The Region, the endpoint's URL and the access key, when all are
    /// usable.
    fn checked(&self) -> Result<(&str, Url, &Credentials), ConfigProblem> {
        let region_setting = SECRET_PROVIDERS_AWS_SM_REGION.name;
        let Some(region) = self.region.as_deref() else {
            return Err(ConfigProblem {
                fix: format!(
                    "Set `{region_setting}` in configuration or the Operator policy, or `AWS_REGION` in the engine's environment, to the AWS Region of the secrets, such as `eu-west-3`."
                ),
            });
        };
        if region.is_empty()
            || !region
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(ConfigProblem {
                fix: format!(
                    "`{region_setting}` is `{region}`, which is no AWS Region; name one such as `eu-west-3`."
                ),
            });
        }
        let endpoint_setting = SECRET_PROVIDERS_AWS_SM_ENDPOINT.name;
        let address = self
            .endpoint_url
            .clone()
            .unwrap_or_else(|| format!("https://secretsmanager.{region}.amazonaws.com"));
        let url = server_url(&address).map_err(|problem| ConfigProblem {
            fix: format!("`{endpoint_setting}` is `{address}`, {problem}."),
        })?;
        let Some(credentials) = &self.credentials else {
            return Err(ConfigProblem {
                fix: "Set `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`, and `AWS_SESSION_TOKEN` for temporary credentials, in the engine's environment to an access key whose policy allows `secretsmanager:GetSecretValue`.".to_owned(),
            });
        };
        Ok((region, url, credentials))
    }
}

impl SecretProvider for AwsSecretsManager {
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a> {
        Box::pin(async move {
            let (region, server, credentials) =
                self.checked().map_err(ProviderFailure::NotConfigured)?;
            let (name, key) = match reference.rest().split_once('#') {
                Some((name, key)) => (name, Some(key)),
                None => (reference.rest(), None),
            };
            let body = serde_json::json!({ "SecretId": name }).to_string();
            let request = signed_request(&server, body, region, credentials, self.clock.wall())?;
            let answer = self.endpoint.send(request).await.map_err(|failure| {
                unreached(
                    failure,
                    SERVICE,
                    &server,
                    SECRET_PROVIDERS_AWS_SM_ENDPOINT.name,
                )
            })?;
            let status = answer.status().as_u16();
            if status == 200 {
                value_of(answer.body(), key)
            } else {
                Err(translate(status, answer.body())
                    .retrying_after(self.endpoint.retry_after(&answer)))
            }
        })
    }

    fn check_config(&self) -> Result<(), ConfigProblem> {
        self.checked().map(|_| ())
    }
}

/// `GetSecretValue`'s request with `body` to `server`, signed for
/// `region` by `credentials` at `now`.
fn signed_request(
    server: &Url,
    body: String,
    region: &str,
    credentials: &Credentials,
    now: SystemTime,
) -> Result<Request<Bytes>, ProviderFailure> {
    let amz_date = jiff::Timestamp::try_from(now)
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
        .strftime("%Y%m%dT%H%M%SZ")
        .to_string();
    let host = match server.port() {
        Some(port) => format!("{}:{port}", server.host_str().unwrap_or_default()),
        None => server.host_str().unwrap_or_default().to_owned(),
    };
    let mut headers = vec![
        ("content-type", CONTENT_TYPE.to_owned()),
        ("host", host),
        ("x-amz-date", amz_date.clone()),
        ("x-amz-target", TARGET.to_owned()),
    ];
    if let Some(token) = &credentials.session_token {
        headers.push(("x-amz-security-token", token.expose_secret().to_owned()));
    }
    headers.sort_by(|left, right| left.0.cmp(right.0));
    let authorization = authorization(
        &Signing {
            method: "POST",
            path: server.path(),
            query: "",
            headers: &headers,
            body: body.as_bytes(),
            region,
            service: SIGNING_NAME,
            amz_date: &amz_date,
        },
        credentials,
    );
    let mut builder = Request::post(server.as_str());
    for (name, value) in &headers {
        if *name == "host" {
            continue;
        }
        let mut value = HeaderValue::from_str(value).map_err(|_| ProviderFailure::Refused {
            fix: "`AWS_SESSION_TOKEN` holds characters no HTTP header may carry; set it to the session token AWS issued.".to_owned(),
        })?;
        if *name == "x-amz-security-token" {
            value.set_sensitive(true);
        }
        builder = builder.header(*name, value);
    }
    let mut authorization =
        HeaderValue::from_str(&authorization).map_err(|_| ProviderFailure::Refused {
            fix: "`AWS_ACCESS_KEY_ID` holds characters no HTTP header may carry; set it to the access key ID AWS issued.".to_owned(),
        })?;
    authorization.set_sensitive(true);
    Ok(builder
        .header("authorization", authorization)
        .body(Bytes::from(body))
        .expect("invariant: a parsed URL and checked headers make a request"))
}

/// What one request's Signature Version 4 covers.
pub(crate) struct Signing<'a> {
    pub(crate) method: &'a str,
    /// The URL's path, already encoded.
    pub(crate) path: &'a str,
    /// The canonical query string.
    pub(crate) query: &'a str,
    /// The signed headers, in lower case, sorted by name, with trimmed
    /// values.
    pub(crate) headers: &'a [(&'a str, String)],
    pub(crate) body: &'a [u8],
    pub(crate) region: &'a str,
    pub(crate) service: &'a str,
    /// The request's time, as `YYYYMMDDTHHMMSSZ`.
    pub(crate) amz_date: &'a str,
}

/// The `Authorization` header value of `signing`'s request, signed by
/// `credentials` (AWS Signature Version 4).
pub(crate) fn authorization(signing: &Signing<'_>, credentials: &Credentials) -> String {
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
        hex(&Sha256::digest(signing.body))
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

/// The value a successful `GetSecretValue` answered, `body`: the secret's
/// text, or its JSON object's `key`.
pub(crate) fn value_of(body: &[u8], key: Option<&str>) -> Result<SecretString, ProviderFailure> {
    let answer = json_of(body, SERVICE)?;
    let Some(text) = answer.get("SecretString").and_then(|text| text.as_str()) else {
        return Err(ProviderFailure::Refused {
            fix:
                "The secret holds binary data, not text; store its value as text (`SecretString`)."
                    .to_owned(),
        });
    };
    let Some(key) = key else {
        return Ok(SecretString::from(text.to_owned()));
    };
    let Ok(serde_json::Value::Object(object)) = serde_json::from_str::<serde_json::Value>(text)
    else {
        return Err(ProviderFailure::Refused {
            fix: "The secret's text is no JSON object, so it has no key; name the whole secret, `aws-sm:name`, or store key/value pairs.".to_owned(),
        });
    };
    text_of(object.get(key))
        .map(SecretString::from)
        .ok_or(ProviderFailure::Missing)
}

/// The failures Secrets Manager is known to answer `GetSecretValue` with,
/// by the `__type` it names, and what each means.
const KNOWN: &[(&str, Meaning)] = &[
    ("ResourceNotFoundException", Meaning::Missing),
    ("AccessDeniedException", Meaning::Denied),
    ("UnrecognizedClientException", Meaning::Credentials),
    ("InvalidClientTokenId", Meaning::Credentials),
    ("InvalidSignatureException", Meaning::Credentials),
    ("IncompleteSignature", Meaning::Credentials),
    ("MissingAuthenticationToken", Meaning::Credentials),
    ("ExpiredTokenException", Meaning::Expired),
    ("SignatureDoesNotMatch", Meaning::Credentials),
    ("InvalidRequestException", Meaning::Unusable),
    ("DecryptionFailure", Meaning::Decryption),
    ("ThrottlingException", Meaning::Throttled),
    ("InternalServiceError", Meaning::ServiceFailed),
];

/// A known failure's meaning.
#[derive(Clone, Copy)]
enum Meaning {
    Missing,
    Denied,
    Credentials,
    Expired,
    Unusable,
    Decryption,
    Throttled,
    ServiceFailed,
}

/// What Secrets Manager meant by answering with `status` and `body`.
pub(crate) fn translate(status: u16, body: &[u8]) -> ProviderFailure {
    let text = String::from_utf8_lossy(body).into_owned();
    let kind = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|answer| answer.get("__type")?.as_str().map(str::to_owned));
    let known = kind.as_deref().and_then(|kind| {
        let kind = kind.rsplit_once('#').map_or(kind, |(_, kind)| kind);
        KNOWN
            .iter()
            .find(|(known, _)| *known == kind)
            .map(|(_, meaning)| *meaning)
    });
    let refused = |fix: &str| ProviderFailure::Refused {
        fix: format!("{SERVICE}: {fix}"),
    };
    match known {
        Some(Meaning::Missing) => ProviderFailure::Missing,
        Some(Meaning::Denied) => refused(
            "the access key may not read the secret, since its policy does not allow `secretsmanager:GetSecretValue` on it; grant it, then try again.",
        ),
        Some(Meaning::Credentials) => refused(
            "the access key was refused; set `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` in the engine's environment to a valid access key.",
        ),
        Some(Meaning::Expired) => refused(
            "the temporary credentials expired; set new `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and `AWS_SESSION_TOKEN` in the engine's environment.",
        ),
        Some(Meaning::Unusable) => refused(
            "the secret cannot be read as it stands, such as one scheduled for deletion; restore it or name another.",
        ),
        Some(Meaning::Decryption) => refused(
            "the secret's KMS key could not decrypt it; allow the access key `kms:Decrypt` on that key.",
        ),
        Some(Meaning::Throttled) => ProviderFailure::Busy {
            fix: format!("{SERVICE}: the request rate was exceeded; try again later."),
            retry_after: None,
        },
        Some(Meaning::ServiceFailed) => refused("the service failed; try again later."),
        None => {
            tracing::warn!(
                provider = "aws-sm",
                status,
                "an unrecognized failure of the Secret provider"
            );
            ProviderFailure::Unrecognized(PartyText::new(text))
        }
    }
}

#[cfg(test)]
mod tests;
