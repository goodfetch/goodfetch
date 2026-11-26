//! AWS Secrets Manager's provider, `aws-sm:name#key` or `aws-sm:name`: the
//! JSON object's `key` of the secret `name`, or its whole text, read with
//! `GetSecretValue` from the Region `secret_providers.aws_sm.region` names
//! (else the engine's environment's `AWS_REGION`, then
//! `AWS_DEFAULT_REGION`), at `secret_providers.aws_sm.endpoint` or the
//! Region's public endpoint. Requests are signed with Signature Version 4
//! by the access key the engine's environment gives, its own or a role's
//! that a web identity token, a container's or an EC2 instance's identity
//! fetches ([`access_key`]). The endpoint is an Operator-configured
//! endpoint reached through the engine's own HTTP client
//! ([`SecretProviderEndpoint`]).
//!
//! A translation point (ADR 0127): [`translate`] reads the `__type` an
//! `awsJson1.1` failure names.

mod access_key;

use std::time::SystemTime;

use goodfetch_engine::aws_signing::{Credentials, Scope, Unsignable, signed_post};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::secret_endpoint::SecretProviderEndpoint;
use goodfetch_engine::secrets::{
    ConfigProblem, PartyText, ProviderFailure, ProviderInputs, Resolving, SecretProvider,
};
use goodfetch_engine::tasks::BlockingThreads;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::{
    SECRET_PROVIDERS_AWS_SM_ENDPOINT, SECRET_PROVIDERS_AWS_SM_REGION,
};
use hyper::Request;
use hyper::body::Bytes;
use secrecy::SecretString;
use url::Url;

use self::access_key::Source;
use crate::endpoint::{json_of, server_url, text_of, unreached, variable};
use crate::identity::Reach;

/// The service's name in failures.
const SERVICE: &str = "AWS Secrets Manager";

/// The signing name of Secrets Manager.
const SIGNING_NAME: &str = "secretsmanager";

/// The operation's target header.
const TARGET: &str = "secretsmanager.GetSecretValue";

/// The protocol's content type.
const CONTENT_TYPE: &str = "application/x-amz-json-1.1";

/// AWS Secrets Manager's provider over its HTTP API.
#[derive(Clone, Debug)]
pub struct AwsSecretsManager {
    region: Option<String>,
    endpoint_url: Option<String>,
    access_key: Source,
    endpoint: SecretProviderEndpoint,
    blocking: BlockingThreads,
    clock: Clock,
}

impl AwsSecretsManager {
    /// The provider `secret_providers.aws_sm.*` configure in a Run's
    /// `inputs`, which also give how the endpoint is reached, signed by the
    /// access key the engine's environment gives, its token files read on
    /// their blocking threads.
    pub fn configured(inputs: &ProviderInputs<'_>) -> Self {
        let text = |setting| {
            inputs
                .settings
                .get::<Option<&str>>(setting)
                .map(str::to_owned)
        };
        let region = text(&SECRET_PROVIDERS_AWS_SM_REGION)
            .or_else(|| variable(inputs.environment, "AWS_REGION"))
            .or_else(|| variable(inputs.environment, "AWS_DEFAULT_REGION"));
        Self {
            region,
            endpoint_url: text(&SECRET_PROVIDERS_AWS_SM_ENDPOINT),
            access_key: Source::of(inputs.environment),
            endpoint: SecretProviderEndpoint::of(inputs),
            blocking: inputs.blocking.clone(),
            clock: inputs.clock,
        }
    }

    /// The Region, the endpoint's URL and the access key's source, when
    /// all are usable.
    fn checked(&self) -> Result<(&str, Url, &Source), ConfigProblem> {
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
        self.access_key.check()?;
        Ok((region, url, &self.access_key))
    }
}

impl SecretProvider for AwsSecretsManager {
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a> {
        Box::pin(async move {
            let (region, server, source) =
                self.checked().map_err(ProviderFailure::NotConfigured)?;
            let (name, key) = match reference.rest().split_once('#') {
                Some((name, key)) => (name, Some(key)),
                None => (reference.rest(), None),
            };
            let reach = Reach {
                endpoint: &self.endpoint,
                blocking: &self.blocking,
                clock: self.clock,
            };
            let credentials = source.credentials(region, &reach).await?;
            let body = serde_json::json!({ "SecretId": name }).to_string();
            let request = signed_request(&server, body, region, &credentials, self.clock.wall())?;
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
                Err(translate(status, answer.body(), source.describe())
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
    signed_post(
        server,
        &[("content-type", CONTENT_TYPE), ("x-amz-target", TARGET)],
        Bytes::from(body),
        Scope {
            region,
            service: SIGNING_NAME,
        },
        credentials,
        now,
    )
    .map_err(|unsignable| ProviderFailure::Refused {
        fix: match unsignable {
            Unsignable::AccessKeyId => "The access key ID holds characters no HTTP header may carry; give the access key ID AWS issued.",
            _ => "The access key's session token holds characters no HTTP header may carry; give the session token AWS issued.",
        }
        .to_owned(),
    })
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

/// What Secrets Manager meant by answering with `status` and `body` a
/// request signed by the access key `from` gave, in a failure's words.
pub(crate) fn translate(status: u16, body: &[u8], from: &str) -> ProviderFailure {
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
        Some(Meaning::Credentials) => refused(&format!(
            "the access key from {from} was refused; give it a valid access key."
        )),
        Some(Meaning::Expired) => refused(&format!(
            "the temporary credentials from {from} expired; renew them, then try again."
        )),
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
