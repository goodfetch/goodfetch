//! The `apify` Proxy provider adapter: Apify Proxy, its datacenter proxies
//! (the default, Apify's `auto` groups) and its `RESIDENTIAL` group. Its
//! username names no account, only parameters joined by `,`:
//! `[groups-RESIDENTIAL,]session-<session>[,country-<CC>]`; the password
//! is the account's proxy password. Apify takes no lifetime in it and keeps
//! a datacenter session's exit 26 hours and a residential one's about 30
//! minutes, its longest. Its failures come as the proxy's status.

use std::time::Duration;

use goodfetch_protocol::page::proxy::{CountryCode, ProxyKind, ProxyProviderConfiguration};

use super::failure::{ProviderAnswer, ProviderFailure, Unrecognized};
use super::{Offer, ProxyProvider, Sticky, hosted, sticky_session};

/// The adapter's name, as `proxy.provider` takes it.
pub(crate) const NAME: &str = "apify";

/// Apify Proxy.
const PUBLIC: &str = "http://proxy.apify.com:8000";

/// How long Apify keeps a datacenter session's exit.
const DATACENTER: Duration = Duration::from_hours(26);

/// How long Apify keeps a residential session's exit.
const RESIDENTIAL: Duration = Duration::from_mins(30);

/// The `apify` adapter of one account.
pub(crate) struct Apify {
    offer: Offer,
}

impl Apify {
    /// The adapter of `configuration`'s account.
    pub(crate) fn new(configuration: &ProxyProviderConfiguration) -> Self {
        Self {
            offer: hosted(
                configuration,
                PUBLIC,
                &[
                    (ProxyKind::Datacenter, DATACENTER),
                    (ProxyKind::Residential, RESIDENTIAL),
                ],
            ),
        }
    }
}

impl ProxyProvider for Apify {
    fn offer(&self) -> &Offer {
        &self.offer
    }

    fn username(&self, sticky: &Sticky<'_>) -> Option<String> {
        let mut parameters = Vec::new();
        if sticky.kind == ProxyKind::Residential {
            parameters.push("groups-RESIDENTIAL".to_owned());
        }
        parameters.push(format!("session-{}", sticky_session(sticky.session)));
        if let Some(country) = sticky.country {
            parameters.push(format!(
                "country-{}",
                CountryCode::as_str(country).to_ascii_uppercase()
            ));
        }
        Some(parameters.join(","))
    }

    /// Apify's username names no lifetime: its seconds, which it never
    /// sends.
    fn format_lifetime(&self, lifetime: Duration) -> String {
        lifetime.as_secs().to_string()
    }
}

/// Apify's failure in `answer` (ADR 0127's translation point).
pub(crate) fn failure(answer: &ProviderAnswer) -> ProviderFailure {
    match answer.status {
        407 => ProviderFailure::BadCredentials,
        402 => ProviderFailure::QuotaExhausted,
        status => ProviderFailure::Unrecognized(Unrecognized::new(
            NAME,
            format!(
                "{status} {}",
                answer.header("x-apify-proxy-error").unwrap_or("-")
            ),
        )),
    }
}
