//! What every HTTP-backed provider does the same way with its server, an
//! Operator-configured endpoint reached through the engine's own HTTP
//! client: which addresses a setting may name, and what a request that
//! reached no answer, or an answer that is no JSON, means.

use std::net::IpAddr;

use goodfetch_engine::secret_endpoint::EndpointFailure;
use goodfetch_engine::secrets::{PartyText, ProviderFailure};
use url::{Host, Url};

/// `address` as a server's URL: `https:`, or `http:` for a loopback host
/// only, as a development server's or a local stand-in's; otherwise why
/// not, as a clause.
pub(crate) fn server_url(address: &str) -> Result<Url, &'static str> {
    let url = Url::parse(address).map_err(|_| "which is no URL")?;
    if url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() {
        return Err(
            "a URL with a query, a fragment or a user name, which a server's address never holds",
        );
    }
    let loopback = match url.host() {
        Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => IpAddr::V4(address).is_loopback(),
        Some(Host::Ipv6(address)) => IpAddr::V6(address).is_loopback(),
        None => return Err("a URL with no host"),
    };
    match url.scheme() {
        "https" => Ok(url),
        "http" if loopback => Ok(url),
        "http" => Err(
            "a plain `http:` URL off loopback, which would send the credentials unencrypted; use `https:`",
        ),
        _ => Err("a URL of neither `https:` nor `http:`"),
    }
}

/// What a request to `service`'s server at `server`, which `setting`
/// names, that reached no answer means.
pub(crate) fn unreached(
    failure: EndpointFailure,
    service: &str,
    server: &Url,
    setting: &str,
) -> ProviderFailure {
    let refused = |fix: String| ProviderFailure::Refused { fix };
    match failure {
        EndpointFailure::Refused(refused_by) => refused(format!(
            "{service} at `{server}` is refused by the egress policy: {refused_by}"
        )),
        EndpointFailure::CertificateInvalid(invalid) => refused(format!(
            "{service} at `{server}` presented a certificate that failed verification ({invalid}); list the authority that signed it under `tls.trusted_cas` for its host. `tls.accept_invalid_certs` makes no exception for a Secret provider."
        )),
        EndpointFailure::Network => refused(format!(
            "{service} at `{server}` did not answer; check `{setting}` and the network, then try again."
        )),
        failure => {
            tracing::warn!(service, "the Secret provider's endpoint failed");
            ProviderFailure::Unrecognized(PartyText::new(failure.to_string()))
        }
    }
}

/// `body` as JSON, or an unrecognized failure of `service`'s.
pub(crate) fn json_of(body: &[u8], service: &str) -> Result<serde_json::Value, ProviderFailure> {
    serde_json::from_slice(body).map_err(|_| {
        tracing::warn!(service, "the Secret provider answered no JSON");
        ProviderFailure::Unrecognized(PartyText::new(String::from_utf8_lossy(body).into_owned()))
    })
}

/// A JSON value as a secret's text: a text value as it is, any other as its
/// JSON text; none for an absent or null value.
pub(crate) fn text_of(value: Option<&serde_json::Value>) -> Option<String> {
    match value {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(text)) => Some(text.clone()),
        Some(other) => Some(other.to_string()),
    }
}

/// The variable `name` of `environment`, when set to text that is not
/// empty.
pub(crate) fn variable(
    environment: &std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString>,
    name: &str,
) -> Option<String> {
    environment
        .get(&std::ffi::OsString::from(name))
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
