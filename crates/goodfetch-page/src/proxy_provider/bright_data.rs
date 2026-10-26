//! The `bright_data` Proxy provider adapter: Bright Data's super proxy,
//! one zone per proxy kind, named after the kind. Its sticky username is
//! `brd-customer-<account>-zone-<kind>[-country-<cc>]-session-<session>`
//! with the country lower-cased; Bright Data takes no lifetime in it and
//! keeps a session's exit while it is used, up to 5 minutes idle, so that
//! is its longest lifetime. Its failures come as `x-brd-err-code`
//! (Bright Data's error catalog).

use std::time::Duration;

use goodfetch_protocol::page::proxy::{CountryCode, ProxyKind, ProxyProviderConfiguration};

use super::failure::{ProviderAnswer, ProviderFailure, Unrecognized};
use super::{Offer, ProxyProvider, Sticky, account, hosted, sticky_session};

/// The adapter's name, as `proxy.provider` takes it.
pub(crate) const NAME: &str = "bright_data";

/// Bright Data's super proxy.
const PUBLIC: &str = "http://brd.superproxy.io:33335";

/// How long Bright Data keeps an idle session's exit.
const LONGEST: Duration = Duration::from_mins(5);

/// The `bright_data` adapter of one account.
pub(crate) struct BrightData {
    offer: Offer,
    customer: String,
}

impl BrightData {
    /// The adapter of `configuration`'s account, its customer ID.
    ///
    /// # Errors
    ///
    /// What is missing, for `setting_invalid`.
    pub(crate) fn new(configuration: &ProxyProviderConfiguration) -> Result<Self, String> {
        let customer = account(configuration, NAME, "the account's customer ID")?;
        let kinds = [
            ProxyKind::Residential,
            ProxyKind::Datacenter,
            ProxyKind::Isp,
            ProxyKind::Mobile,
        ]
        .map(|kind| (kind, LONGEST));
        Ok(Self {
            offer: hosted(configuration, PUBLIC, &kinds),
            customer,
        })
    }
}

impl ProxyProvider for BrightData {
    fn offer(&self) -> &Offer {
        &self.offer
    }

    fn username(&self, sticky: &Sticky<'_>) -> Option<String> {
        let country = sticky
            .country
            .map(|country| {
                format!(
                    "-country-{}",
                    CountryCode::as_str(country).to_ascii_lowercase()
                )
            })
            .unwrap_or_default();
        Some(format!(
            "brd-customer-{}-zone-{}{country}-session-{}",
            self.customer,
            sticky.kind,
            sticky_session(sticky.session)
        ))
    }

    /// Bright Data's username names no lifetime: its seconds, which it
    /// never sends.
    fn format_lifetime(&self, lifetime: Duration) -> String {
        lifetime.as_secs().to_string()
    }
}

/// Bright Data's failure in `answer` (ADR 0127's translation point).
pub(crate) fn failure(answer: &ProviderAnswer) -> ProviderFailure {
    match answer.header("x-brd-err-code") {
        Some("client_10000" | "client_10001" | "client_10010") => ProviderFailure::BadCredentials,
        Some("client_10100" | "client_10020") => ProviderFailure::QuotaExhausted,
        Some("client_10064") => ProviderFailure::StickySessionExpired,
        None if answer.status == 407 => ProviderFailure::BadCredentials,
        code => ProviderFailure::Unrecognized(Unrecognized::new(
            NAME,
            format!(
                "{} {} {}",
                answer.status,
                code.unwrap_or("-"),
                answer.header("x-brd-error").unwrap_or("-")
            ),
        )),
    }
}
