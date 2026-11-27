//! How a page answers a 407 or 401 the browser reports (ADRs 0053 and 0092),
//! for the page's own requests and for the engine's loads alike:
//! - an HTTP or HTTPS proxy's, or the Session's egress relay's, 407 with the
//!   Session's proxy credentials, once per request; a second one for the same
//!   request, or one with no credentials to give, is cancelled as
//!   `proxy_auth_failed`;
//! - a site's 401 with `session.http_credentials` when the request's host is
//!   on a listed entry's registrable domains, never on a browser other code
//!   may read or a host reached through `tls.accept_invalid_certs` (each
//!   cancelled with `secret_input_refused`), and once per request;
//! - an unlisted site's 401 goes on as the browser would answer it with no
//!   credentials.

use goodfetch_engine::CallError;
use goodfetch_engine::run::secret_error;
use goodfetch_engine::secrets::{self, Purpose};
use goodfetch_protocol::secrets::SecretInputRefusedReason;
use goodfetch_protocol::settings::SESSION_HTTP_CREDENTIALS;
use secrecy::ExposeSecret as _;

use crate::cdp::driver::PageDriver;
use crate::cdp::{AuthAnswer, AuthChallenge};
use crate::placeholders::{host_of, is_tls_exception, secret_input_refused};

/// The answer to one challenge.
pub(crate) struct Challenged {
    /// What the browser is told.
    pub(crate) response: AuthAnswer,
    /// Whether it gives credentials: the request leaves only once the browser
    /// has them.
    pub(crate) credentials_given: bool,
    /// `proxy_auth_failed` when it cancels the Session's proxy's challenge, or
    /// `secret_input_refused` / a Secret resolution error when it refuses a
    /// site's 401 challenge.
    pub(crate) refused: Option<CallError>,
}

/// The answer `driver`'s page gives `required`. `given_before` records that
/// credentials went to `required`'s request, and answers whether they went to
/// it already.
pub(crate) async fn answer(
    driver: &PageDriver,
    required: &AuthChallenge,
    given_before: impl FnOnce() -> bool,
) -> Challenged {
    let proxied = required.by_proxy() && (driver.proxy().is_some() || driver.relay().is_some());
    if proxied {
        let seen = given_before();
        return match driver.proxy_credentials() {
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
        };
    }
    if !required.by_proxy()
        && let Some(credentials) = driver.http_credentials()
    {
        let host = host_of(required.url());
        if let Some(entry) = credentials
            .entries
            .iter()
            .find(|entry| secrets::allowed(&host, &entry.domains))
        {
            if driver.other_code_may_read() {
                return Challenged {
                    response: AuthAnswer::Cancel,
                    credentials_given: false,
                    refused: Some(secret_input_refused(
                        SESSION_HTTP_CREDENTIALS.name.to_owned(),
                        entry.password.to_string(),
                        host,
                        SecretInputRefusedReason::OtherCodeMayRead,
                    )),
                };
            }
            if is_tls_exception(driver, required.url(), &host) {
                return Challenged {
                    response: AuthAnswer::Cancel,
                    credentials_given: false,
                    refused: Some(secret_input_refused(
                        SESSION_HTTP_CREDENTIALS.name.to_owned(),
                        entry.password.to_string(),
                        host,
                        SecretInputRefusedReason::InvalidCertificateException,
                    )),
                };
            }
            if given_before() {
                return Challenged {
                    response: AuthAnswer::Cancel,
                    credentials_given: false,
                    refused: None,
                };
            }
            let purpose = Purpose {
                placeholder: SESSION_HTTP_CREDENTIALS.name,
                domain: Some(&host),
                domains: &entry.domains,
                layer: credentials.layer,
                run_id: Some(&credentials.run_id),
            };
            return match credentials.secrets.resolve(&entry.password, &purpose).await {
                Ok(password) => Challenged {
                    response: AuthAnswer::ProvideCredentials {
                        username: entry.username.clone(),
                        password: password.expose_secret().to_owned(),
                    },
                    credentials_given: true,
                    refused: None,
                },
                Err(error) => Challenged {
                    response: AuthAnswer::Cancel,
                    credentials_given: false,
                    refused: Some(secret_error(&error)),
                },
            };
        }
    }
    Challenged {
        response: AuthAnswer::Default,
        credentials_given: false,
        refused: None,
    }
}
