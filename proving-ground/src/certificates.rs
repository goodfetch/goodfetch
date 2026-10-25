//! The `certificates` family: one host name per certificate a client must
//! refuse (pitfall H03, ADR 0042), each served over TLS with a leaf that
//! fails verification in one way only, so a refusal names that one
//! failure. A client that accepts the certificate anyway, as a per-domain
//! exception allows, reads a page naming the defect.
//!
//! - `expired.certificates.test`: signed by the seed's authority, its
//!   validity ended the day before the authority was made.
//! - `not-yet-valid.certificates.test`: signed by the seed's authority, valid
//!   from 30 days after it was made.
//! - `unknown-issuer.certificates.test`: signed by another authority from the
//!   seed, which the Proving ground never publishes.
//! - `name-mismatch.certificates.test`: signed by the seed's authority for
//!   `elsewhere.certificates.test` only.
//! - `self-signed.certificates.test`: signed by its own key.
//!
//! `GET /set-cookie` on any host of the family sets `session=c00k1e` on the
//! whole `certificates.test` domain, so a client holds a cookie each host of
//! the family would be sent: the Session cookie an exception host must not
//! receive (ADR 0042).
//!
//! Beside them, two hosts with a valid leaf require mutual TLS (story
//! 4-34): `client-certificate.certificates.test` and
//! `other-client-certificate.certificates.test` each refuse a handshake
//! without a client certificate that the seed's client authority signed,
//! which `GET /client-certificate` and `GET /client-key` on the Proving
//! ground's own address hand a test, and the observations list each
//! handshake that presented one.

use crate::exchange::{Reply, encode};

/// The host that requires a client certificate.
pub(crate) const CLIENT_CERTIFICATE_HOST: &str = "client-certificate.certificates.test";

/// A second host that requires one, outside the first's name: a client
/// certificate listed for the first goes to it only when listed for it too.
pub(crate) const OTHER_CLIENT_CERTIFICATE_HOST: &str = "other-client-certificate.certificates.test";

/// Whether `host`, a lowercased name without its port, requires a client
/// certificate.
pub(crate) fn requires_client_certificate(host: &str) -> bool {
    host == CLIENT_CERTIFICATE_HOST || host == OTHER_CLIENT_CERTIFICATE_HOST
}

/// How a host's leaf certificate fails verification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Defect {
    /// Its validity has ended.
    Expired,
    /// Its validity has not begun.
    NotYetValid,
    /// An authority no client trusts signed it.
    UnknownIssuer,
    /// It names another host.
    NameMismatch,
    /// Its own key signed it.
    SelfSigned,
}

impl Defect {
    /// Every defect, in the order the family lists its hosts.
    pub(crate) const ALL: [Self; 5] = [
        Self::Expired,
        Self::NotYetValid,
        Self::UnknownIssuer,
        Self::NameMismatch,
        Self::SelfSigned,
    ];

    /// The host name that serves it.
    pub(crate) fn host(self) -> &'static str {
        match self {
            Self::Expired => "expired.certificates.test",
            Self::NotYetValid => "not-yet-valid.certificates.test",
            Self::UnknownIssuer => "unknown-issuer.certificates.test",
            Self::NameMismatch => "name-mismatch.certificates.test",
            Self::SelfSigned => "self-signed.certificates.test",
        }
    }

    /// The defect `host`, a lowercased name without its port, serves.
    pub(crate) fn of(host: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|defect| defect.host() == host)
    }

    fn describe(self) -> &'static str {
        match self {
            Self::Expired => "an expired certificate",
            Self::NotYetValid => "a certificate not valid yet",
            Self::UnknownIssuer => "a certificate from an unknown issuer",
            Self::NameMismatch => "a certificate for another host name",
            Self::SelfSigned => "a self-signed certificate",
        }
    }
}

/// The name the leaf of `name-mismatch.certificates.test` names instead.
pub(crate) const MISMATCHED_NAME: &str = "elsewhere.certificates.test";

/// Every host name of the family, none a task's.
pub(crate) fn hosts() -> Vec<(String, Option<String>)> {
    Defect::ALL
        .into_iter()
        .map(Defect::host)
        .chain([CLIENT_CERTIFICATE_HOST, OTHER_CLIENT_CERTIFICATE_HOST])
        .map(|host| (host.to_owned(), None))
        .collect()
}

/// The path of any host of the family that sets a cookie on the whole
/// `certificates.test` domain, which a client then sends every host of the
/// family: the Session cookie no exception host may receive (ADR 0042).
pub(crate) const SET_COOKIE_PATH: &str = "/set-cookie";

/// The cookie [`SET_COOKIE_PATH`] sets.
pub(crate) const DOMAIN_COOKIE: &str = "session=c00k1e; Domain=certificates.test; Path=/";

/// The page every path of a host of the family answers, for a client that
/// accepted its certificate or presented its own, and the domain cookie at
/// [`SET_COOKIE_PATH`]; `None` for any other host.
pub(crate) fn respond(name: &str, path: &str) -> Option<Reply> {
    if path == SET_COOKIE_PATH && hosts().iter().any(|(host, _)| host == name) {
        let mut reply = Reply::html(
            "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Cookie set</title></head>\n<body><main><h1>Cookie set</h1><p>This page set a cookie on the whole certificates.test domain.</p></main></body>\n</html>\n".to_owned(),
        );
        reply.headers.push(("set-cookie", DOMAIN_COOKIE.to_owned()));
        return Some(reply);
    }
    if requires_client_certificate(name) {
        return Some(Reply::html(
            "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Reached with a client certificate</title></head>\n<body><main><h1>Reached with a client certificate</h1><p>This host requires mutual TLS; a client reads this page only when it presented the client certificate the Proving ground's client authority signed.</p></main></body>\n</html>\n".to_owned(),
        ));
    }
    let defect = Defect::of(name)?;
    let described = encode(defect.describe());
    Some(Reply::html(format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Reached past {described}</title></head>\n<body><main><h1>Reached past {described}</h1><p>This host serves {described}; a client reads this page only when an exception lets it through.</p></main></body>\n</html>\n"
    )))
}
