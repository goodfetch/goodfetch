//! Where AWS Secrets Manager's provider takes the access key it signs with
//! (issue 1658), in the order AWS's SDKs look, shared files aside: the
//! engine's environment's own access key (`AWS_ACCESS_KEY_ID`,
//! `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`); a web identity token,
//! such as EKS gives a service account, that AWS STS exchanges for a role's
//! temporary credentials (`AWS_WEB_IDENTITY_TOKEN_FILE`, `AWS_ROLE_ARN`,
//! `AWS_ROLE_SESSION_NAME`); the container credentials endpoint ECS and
//! EKS Pod Identity give a task (`AWS_CONTAINER_CREDENTIALS_RELATIVE_URI`
//! or `AWS_CONTAINER_CREDENTIALS_FULL_URI`, with
//! `AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE` or
//! `AWS_CONTAINER_AUTHORIZATION_TOKEN`); and the EC2 instance's role,
//! through the instance metadata service's version 2 alone, unless
//! `AWS_EC2_METADATA_DISABLED` is `true`.
//!
//! The first source the environment names is the one taken, and its
//! failure is the provider's, naming it. Each identity endpoint is an
//! Operator-configured endpoint, reached through the provider's own
//! [`SecretProviderEndpoint`], at the address the variable AWS documents
//! for it gives (`AWS_ENDPOINT_URL_STS`,
//! `AWS_EC2_METADATA_SERVICE_ENDPOINT`), or its documented default. Plain
//! `http:` is taken for loopback and for the link-local addresses AWS
//! serves an instance's or a task's identity at. The instance metadata
//! service, which a host off EC2 lacks, is `not_configured` when it gives
//! no token within [`METADATA_WAIT`] or no role. Temporary
//! credentials are fetched for each value read, as Azure Key Vault's token
//! is.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use goodfetch_engine::secret_endpoint::EndpointFailure;
use goodfetch_engine::secrets::{ConfigProblem, PartyText, ProviderFailure};
use hyper::Request;
use hyper::body::Bytes;
use hyper::header::HeaderValue;
use quick_xml::events::Event;
use secrecy::{ExposeSecret as _, SecretString};

use crate::endpoint::{metadata_url, server_url, unreached, variable};
use crate::identity::{METADATA_WAIT, Reach, not_configured, unrecognized};
use goodfetch_engine::aws_signing::Credentials;

/// The container credentials endpoint's host, which
/// `AWS_CONTAINER_CREDENTIALS_RELATIVE_URI`'s path is read at.
const CONTAINER_HOST: &str = "http://169.254.170.2";

/// The instance metadata service's default address.
const INSTANCE_METADATA: &str = "http://169.254.169.254";

/// How long an instance metadata token lasts, in seconds: the longest the
/// service gives.
const TOKEN_LIFETIME: &str = "21600";

/// The services' names in failures.
const STS: &str = "AWS STS, exchanging the web identity token,";
const CONTAINER: &str = "The container credentials endpoint";
const INSTANCE: &str = "The EC2 instance metadata service";

/// What makes a provider with no source answer.
const NO_SOURCE: &str = "Give the engine an access key whose policy allows `secretsmanager:GetSecretValue`: set `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`, and `AWS_SESSION_TOKEN` for temporary credentials, in its environment, or run it with a web identity token (`AWS_WEB_IDENTITY_TOKEN_FILE` and `AWS_ROLE_ARN`), in an ECS task or EKS pod with a role, or on an EC2 instance with an IAM role.";

/// Where the access key comes from.
#[derive(Clone, Debug)]
pub(crate) enum Source {
    /// The engine's environment's access key.
    Environment(Credentials),
    /// A role assumed with a web identity token.
    WebIdentity(WebIdentity),
    /// The container credentials endpoint of an ECS task or an EKS pod.
    Container(Container),
    /// The EC2 instance's role, at the instance metadata service's
    /// `address`.
    InstanceMetadata { address: String },
    /// No source: the environment names none and turns the instance
    /// metadata service off.
    None,
}

/// A role to assume with the web identity token in `token_file`.
#[derive(Clone, Debug)]
pub(crate) struct WebIdentity {
    token_file: PathBuf,
    role_arn: String,
    session_name: Option<String>,
    sts_address: Option<String>,
}

/// The container credentials endpoint at `address`, which `variable`
/// gives, asked with `authorization`.
#[derive(Clone, Debug)]
pub(crate) struct Container {
    address: String,
    variable: &'static str,
    authorization: Option<Authorization>,
}

/// The container credentials endpoint's authorization token.
#[derive(Clone, Debug)]
enum Authorization {
    /// Read from this file at each request, which EKS Pod Identity
    /// rotates.
    File(PathBuf),
    /// Given in the environment.
    Token(SecretString),
}

impl Source {
    /// The first source `environment` names.
    pub(crate) fn of(environment: &BTreeMap<OsString, OsString>) -> Self {
        let named = |name| variable(environment, name);
        if let (Some(access_key_id), Some(secret)) =
            (named("AWS_ACCESS_KEY_ID"), named("AWS_SECRET_ACCESS_KEY"))
        {
            return Self::Environment(Credentials {
                access_key_id,
                secret_access_key: SecretString::from(secret),
                session_token: named("AWS_SESSION_TOKEN").map(SecretString::from),
            });
        }
        if let (Some(token_file), Some(role_arn)) =
            (named("AWS_WEB_IDENTITY_TOKEN_FILE"), named("AWS_ROLE_ARN"))
        {
            return Self::WebIdentity(WebIdentity {
                token_file: PathBuf::from(token_file),
                role_arn,
                session_name: named("AWS_ROLE_SESSION_NAME"),
                sts_address: named("AWS_ENDPOINT_URL_STS"),
            });
        }
        let container = named("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI")
            .map(|path| {
                (
                    format!("{CONTAINER_HOST}{path}"),
                    "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
                )
            })
            .or_else(|| {
                named("AWS_CONTAINER_CREDENTIALS_FULL_URI")
                    .map(|address| (address, "AWS_CONTAINER_CREDENTIALS_FULL_URI"))
            });
        if let Some((address, variable)) = container {
            let authorization = named("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE")
                .map(|file| Authorization::File(PathBuf::from(file)))
                .or_else(|| {
                    named("AWS_CONTAINER_AUTHORIZATION_TOKEN")
                        .map(|token| Authorization::Token(SecretString::from(token)))
                });
            return Self::Container(Container {
                address,
                variable,
                authorization,
            });
        }
        if named("AWS_EC2_METADATA_DISABLED").is_some_and(|off| off.eq_ignore_ascii_case("true")) {
            return Self::None;
        }
        Self::InstanceMetadata {
            address: named("AWS_EC2_METADATA_SERVICE_ENDPOINT")
                .unwrap_or_else(|| INSTANCE_METADATA.to_owned()),
        }
    }

    /// Whether the source can be asked: its endpoint's address is one a
    /// request may go to.
    pub(crate) fn check(&self) -> Result<(), ConfigProblem> {
        let problem = |variable: &str, address: &str, problem: &str| ConfigProblem {
            fix: format!("`{variable}` is `{address}`, {problem}."),
        };
        match self {
            Self::Environment(_) => Ok(()),
            Self::WebIdentity(identity) => match &identity.sts_address {
                Some(address) => server_url(address)
                    .map(|_| ())
                    .map_err(|why| problem("AWS_ENDPOINT_URL_STS", address, why)),
                None => Ok(()),
            },
            Self::Container(container) => metadata_url(&container.address)
                .map(|_| ())
                .map_err(|why| problem(container.variable, &container.address, why)),
            Self::InstanceMetadata { address } => metadata_url(address)
                .map(|_| ())
                .map_err(|why| problem("AWS_EC2_METADATA_SERVICE_ENDPOINT", address, why)),
            Self::None => Err(ConfigProblem {
                fix: NO_SOURCE.to_owned(),
            }),
        }
    }

    /// Where the access key comes from, in a failure's words.
    pub(crate) fn describe(&self) -> &'static str {
        match self {
            Self::Environment(_) | Self::None => {
                "the engine's environment (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and `AWS_SESSION_TOKEN`)"
            }
            Self::WebIdentity(_) => {
                "the role `AWS_ROLE_ARN` names, assumed with the web identity token"
            }
            Self::Container(_) => "the container credentials endpoint",
            Self::InstanceMetadata { .. } => "the EC2 instance's IAM role",
        }
    }

    /// The access key, fetched for `region` through `reach` when the source
    /// gives temporary credentials.
    ///
    /// # Errors
    ///
    /// The source's failure, naming it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection it opened.
    pub(crate) async fn credentials(
        &self,
        region: &str,
        reach: &Reach<'_>,
    ) -> Result<Credentials, ProviderFailure> {
        match self {
            Self::Environment(credentials) => Ok(credentials.clone()),
            Self::WebIdentity(identity) => identity.assume(region, reach).await,
            Self::Container(container) => container.ask(reach).await,
            Self::InstanceMetadata { address } => instance_role(address, reach).await,
            Self::None => Err(ProviderFailure::NotConfigured(ConfigProblem {
                fix: NO_SOURCE.to_owned(),
            })),
        }
    }
}

impl WebIdentity {
    /// The role's temporary credentials, which `AssumeRoleWithWebIdentity`
    /// answers for the token in [`Self::token_file`].
    async fn assume(
        &self,
        region: &str,
        reach: &Reach<'_>,
    ) -> Result<Credentials, ProviderFailure> {
        let address = self
            .sts_address
            .clone()
            .unwrap_or_else(|| format!("https://sts.{region}.amazonaws.com"));
        let server = server_url(&address).map_err(|why| {
            not_configured(format!("`AWS_ENDPOINT_URL_STS` is `{address}`, {why}."))
        })?;
        let token = reach
            .read_file(&self.token_file)
            .await
            .map_err(|_| {
                not_configured(format!(
                    "`AWS_WEB_IDENTITY_TOKEN_FILE` names `{}`, which cannot be read; point it at the web identity token the platform writes.",
                    self.token_file.display()
                ))
            })?;
        let session_name = self.session_name.clone().unwrap_or_else(|| {
            let since = reach
                .clock
                .wall()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            format!("goodfetch-{}", since.as_millis())
        });
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("Action", "AssumeRoleWithWebIdentity")
            .append_pair("Version", "2011-06-15")
            .append_pair("RoleArn", &self.role_arn)
            .append_pair("RoleSessionName", &session_name)
            .append_pair("WebIdentityToken", token.expose_secret())
            .finish();
        let request = Request::post(server.as_str())
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Bytes::from(body))
            .expect("invariant: a parsed URL and constant headers make a request");
        let answer = reach
            .endpoint
            .send(request)
            .await
            .map_err(|failure| unreached(failure, STS, &server, "AWS_ENDPOINT_URL_STS"))?;
        let status = answer.status().as_u16();
        if status == 200 {
            assumed(answer.body())
        } else {
            Err(translate_sts(status, answer.body())
                .retrying_after(reach.endpoint.retry_after(&answer)))
        }
    }
}

impl Container {
    /// The task's credentials, which the endpoint answers.
    async fn ask(&self, reach: &Reach<'_>) -> Result<Credentials, ProviderFailure> {
        let server = metadata_url(&self.address).map_err(|why| {
            not_configured(format!("`{}` is `{}`, {why}.", self.variable, self.address))
        })?;
        let mut request = Request::get(server.as_str());
        let token = match &self.authorization {
            Some(Authorization::File(file)) => {
                Some(reach.read_file(file).await.map_err(|_| {
                    not_configured(format!(
                        "`AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE` names `{}`, which cannot be read; point it at the token the platform writes.",
                        file.display()
                    ))
                })?)
            }
            Some(Authorization::Token(token)) => Some(token.clone()),
            None => None,
        };
        if let Some(token) = token {
            let mut value = HeaderValue::from_str(token.expose_secret()).map_err(|_| {
                not_configured(
                    "The container's authorization token holds characters no HTTP header may carry; set it to the token the platform gives."
                        .to_owned(),
                )
            })?;
            value.set_sensitive(true);
            request = request.header("authorization", value);
        }
        let request = request
            .body(Bytes::new())
            .expect("invariant: a parsed URL and checked headers make a request");
        let answer = reach
            .endpoint
            .send(request)
            .await
            .map_err(|failure| unreached(failure, CONTAINER, &server, self.variable))?;
        match answer.status().as_u16() {
            200 => temporary(answer.body(), CONTAINER),
            status => Err(ProviderFailure::Refused {
                fix: format!(
                    "{CONTAINER} at `{server}` answered HTTP {status}; check the task's or the pod's role and its authorization token, then try again."
                ),
            }),
        }
    }
}

/// The EC2 instance role's credentials, which the instance metadata
/// service at `address` answers to its version 2's session token.
async fn instance_role(address: &str, reach: &Reach<'_>) -> Result<Credentials, ProviderFailure> {
    let base = metadata_url(address).map_err(|why| {
        not_configured(format!(
            "`AWS_EC2_METADATA_SERVICE_ENDPOINT` is `{address}`, {why}."
        ))
    })?;
    let at = |path: &str| {
        base.join(path)
            .expect("invariant: a relative path joins a server's URL")
    };
    let absent = |what: &str| {
        not_configured(format!(
            "{INSTANCE} at `{base}` {what}, so the host has no instance role to read. {NO_SOURCE}"
        ))
    };
    let unreachable = |failure| match failure {
        EndpointFailure::Network => absent("did not answer"),
        failure => unreached(
            failure,
            INSTANCE,
            &base,
            "AWS_EC2_METADATA_SERVICE_ENDPOINT",
        ),
    };
    let asking = Request::put(at("latest/api/token").as_str())
        .header("x-aws-ec2-metadata-token-ttl-seconds", TOKEN_LIFETIME)
        .body(Bytes::new())
        .expect("invariant: a joined URL and constant headers make a request");
    let answer = reach
        .promptly(reach.endpoint.send(asking))
        .await
        .ok_or_else(|| {
            absent(&format!(
                "gave no token within {} s",
                METADATA_WAIT.as_secs()
            ))
        })?
        .map_err(unreachable)?;
    if answer.status().as_u16() != 200 {
        return Err(absent(&format!(
            "answered HTTP {} to its token request",
            answer.status().as_u16()
        )));
    }
    let mut token = HeaderValue::from_bytes(answer.body())
        .map_err(|_| unrecognized(INSTANCE, "a token no HTTP header may carry"))?;
    token.set_sensitive(true);
    let read = |path: &str| {
        Request::get(at(path).as_str())
            .header("x-aws-ec2-metadata-token", token.clone())
            .body(Bytes::new())
            .expect("invariant: a joined URL and checked headers make a request")
    };
    let roles = reach
        .endpoint
        .send(read("latest/meta-data/iam/security-credentials/"))
        .await
        .map_err(unreachable)?;
    let role = match roles.status().as_u16() {
        200 => std::str::from_utf8(roles.body())
            .ok()
            .and_then(|names| names.lines().next())
            .map(str::trim)
            .filter(|name| !name.is_empty() && !name.contains(['/', '?', '#']))
            .map(str::to_owned)
            .ok_or_else(|| absent("lists no IAM role"))?,
        404 => return Err(absent("lists no IAM role")),
        status => return Err(absent(&format!("answered HTTP {status} for its roles"))),
    };
    let credentials = reach
        .endpoint
        .send(read(&format!(
            "latest/meta-data/iam/security-credentials/{role}"
        )))
        .await
        .map_err(unreachable)?;
    match credentials.status().as_u16() {
        200 => temporary(credentials.body(), INSTANCE),
        status => Err(ProviderFailure::Refused {
            fix: format!(
                "{INSTANCE} at `{base}` answered HTTP {status} for the role `{role}`'s credentials; try again later."
            ),
        }),
    }
}

/// The temporary credentials `service` answered in `body`, as the container
/// credentials endpoint and the instance metadata service write them.
fn temporary(body: &[u8], service: &str) -> Result<Credentials, ProviderFailure> {
    let answer: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| unrecognized(service, "credentials that are no JSON"))?;
    let field = |name: &str| answer.get(name).and_then(|value| value.as_str());
    if field("Code").is_some_and(|code| code != "Success") {
        return Err(unrecognized(
            service,
            "credentials whose `Code` is not `Success`",
        ));
    }
    match (
        field("AccessKeyId"),
        field("SecretAccessKey"),
        field("Token"),
    ) {
        (Some(access_key_id), Some(secret), token) => Ok(Credentials {
            access_key_id: access_key_id.to_owned(),
            secret_access_key: SecretString::from(secret.to_owned()),
            session_token: token.map(|token| SecretString::from(token.to_owned())),
        }),
        _ => Err(unrecognized(
            service,
            "credentials without `AccessKeyId` or `SecretAccessKey`",
        )),
    }
}

/// The credentials a successful `AssumeRoleWithWebIdentity` answered in
/// `body`.
pub(crate) fn assumed(body: &[u8]) -> Result<Credentials, ProviderFailure> {
    let fields =
        xml_fields(body).ok_or_else(|| unrecognized(STS, "credentials that are no XML"))?;
    match (
        fields.get("AccessKeyId"),
        fields.get("SecretAccessKey"),
        fields.get("SessionToken"),
    ) {
        (Some(access_key_id), Some(secret), Some(token)) => Ok(Credentials {
            access_key_id: access_key_id.clone(),
            secret_access_key: SecretString::from(secret.clone()),
            session_token: Some(SecretString::from(token.clone())),
        }),
        _ => Err(unrecognized(
            STS,
            "credentials without `AccessKeyId`, `SecretAccessKey` or `SessionToken`",
        )),
    }
}

/// The failures AWS STS is known to answer `AssumeRoleWithWebIdentity`
/// with, by the `Code` it names.
const KNOWN: &[(&str, Meaning)] = &[
    ("InvalidIdentityToken", Meaning::Token),
    ("ExpiredTokenException", Meaning::Token),
    ("IDPRejectedClaim", Meaning::Token),
    ("AccessDenied", Meaning::Role),
    ("MalformedPolicyDocument", Meaning::Role),
    ("PackedPolicyTooLarge", Meaning::Role),
    ("RegionDisabledException", Meaning::Region),
    ("IDPCommunicationError", Meaning::Busy),
    ("Throttling", Meaning::Busy),
];

/// A known STS failure's meaning.
#[derive(Clone, Copy)]
enum Meaning {
    Token,
    Role,
    Region,
    Busy,
}

/// What AWS STS meant by answering `AssumeRoleWithWebIdentity` with
/// `status` and `body`.
pub(crate) fn translate_sts(status: u16, body: &[u8]) -> ProviderFailure {
    let code = xml_fields(body).and_then(|fields| fields.get("Code").cloned());
    let known = code
        .as_deref()
        .and_then(|code| KNOWN.iter().find(|(known, _)| *known == code))
        .map(|(_, meaning)| *meaning);
    let refused = |fix: &str| ProviderFailure::Refused {
        fix: format!("{STS} {fix}"),
    };
    match known {
        Some(Meaning::Token) => refused(
            "refused the token of `AWS_WEB_IDENTITY_TOKEN_FILE`; check that the role trusts its issuer and audience and that the platform renews it.",
        ),
        Some(Meaning::Role) => refused(
            "would not let the token assume the role of `AWS_ROLE_ARN`; check its trust policy.",
        ),
        Some(Meaning::Region) => refused(
            "is not enabled in the Region; activate STS there, or set `AWS_ENDPOINT_URL_STS` to one where it is.",
        ),
        Some(Meaning::Busy) => ProviderFailure::Busy {
            fix: format!("{STS} is briefly unable to answer; try again later."),
            retry_after: None,
        },
        None => {
            tracing::warn!(
                provider = "aws-sm",
                status,
                "an unrecognized failure of AWS STS"
            );
            ProviderFailure::Unrecognized(PartyText::new(String::from_utf8_lossy(body)))
        }
    }
}

/// The text of each element of `body` that holds only text, by its local
/// name, the last one's for a name met twice; none when `body` is no XML.
fn xml_fields(body: &[u8]) -> Option<BTreeMap<String, String>> {
    let mut reader = quick_xml::Reader::from_reader(body);
    let mut fields = BTreeMap::new();
    let mut open: Option<(String, String)> = None;
    loop {
        match reader.read_event().ok()? {
            Event::Start(start) => {
                let name = start.local_name();
                let name = std::str::from_utf8(name.as_ref()).ok()?;
                open = Some((name.to_owned(), String::new()));
            }
            Event::Text(text) => {
                if let Some((_, value)) = &mut open {
                    value.push_str(&text.decode().ok()?);
                }
            }
            Event::GeneralRef(reference) => {
                if let Some((_, value)) = &mut open {
                    match reference.resolve_char_ref().ok()? {
                        Some(character) => value.push(character),
                        None => value.push_str(quick_xml::escape::resolve_predefined_entity(
                            &reference.decode().ok()?,
                        )?),
                    }
                }
            }
            Event::End(_) => {
                if let Some((name, value)) = open.take() {
                    fields.insert(name, value.trim().to_owned());
                }
            }
            Event::Eof => return Some(fields),
            _ => {}
        }
    }
}
