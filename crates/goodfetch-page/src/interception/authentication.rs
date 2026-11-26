//! How a page answers a 407 or 401 the browser reports (ADR 0053), for the
//! page's own requests and for the engine's loads alike: an HTTP or HTTPS
//! proxy's, or the Session's egress relay's, with the Session's credentials,
//! once per request; a second one for the same request, or one with no
//! credentials to give, is cancelled, and is the proxy's refusal,
//! `proxy_auth_failed`. A site's own 401 goes on as the browser would answer
//! it with no credentials.

use goodfetch_engine::CallError;

use crate::cdp::{AuthAnswer, AuthChallenge};
use crate::driver::PageDriver;

/// The answer to one challenge.
pub(crate) struct Challenged {
    /// What the browser is told.
    pub(crate) response: AuthAnswer,
    /// Whether it gives the Session's credentials: the request leaves only
    /// once the browser has them.
    pub(crate) credentials_given: bool,
    /// `proxy_auth_failed`, when it cancels the Session's proxy's
    /// challenge: the proxy refused the credentials, or asked for some the
    /// Session does not have. None behind a relay with no proxy.
    pub(crate) refused: Option<CallError>,
}

/// The answer `driver`'s page gives `required`. `given_before` records
/// that the Session's credentials go to `required`'s request, and answers
/// whether they went to it already: it is asked only for a proxy's
/// challenge.
pub(crate) fn answer(
    driver: &PageDriver,
    required: &AuthChallenge,
    given_before: impl FnOnce() -> bool,
) -> Challenged {
    let proxied = required.by_proxy() && (driver.proxy().is_some() || driver.relay().is_some());
    if !proxied {
        return Challenged {
            response: AuthAnswer::Default,
            credentials_given: false,
            refused: None,
        };
    }
    let seen = given_before();
    match driver.proxy_credentials() {
        Some((username, password)) if !seen => Challenged {
            response: AuthAnswer::ProvideCredentials {
                username: username.to_owned(),
                password: password.to_owned(),
            },
            credentials_given: true,
            refused: None,
        },
        // Behind the relay, a second 407 is the upstream proxy's refusal of
        // the Session's credentials, which the relay passed on.
        _ => Challenged {
            response: AuthAnswer::Cancel,
            credentials_given: false,
            refused: driver
                .proxy()
                .map(|proxy| proxy.auth_failed(required.url())),
        },
    }
}
