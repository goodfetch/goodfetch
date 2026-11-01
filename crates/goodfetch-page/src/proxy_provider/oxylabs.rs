//! The `oxylabs` Proxy provider adapter: Oxylabs' Residential Proxies
//! entry. Its sticky username is
//! `customer-<account>[-cc-<CC>]-sessid-<session>-sesstime-<minutes>`, the
//! country upper-cased and the lifetime in whole minutes, at most 1,440
//! (24 hours), its longest. Its failures come as `X-Error-Description`
//! (Oxylabs' response codes).

use std::time::Duration;

use goodfetch_protocol::page::proxy::{CountryCode, ProxyKind, ProxyProviderConfiguration};

use super::failure::{ProviderAnswer, ProviderFailure, Unrecognized};
use super::{Offer, ProxyProvider, Sticky, account, hosted, sticky_session};

/// The adapter's name, as `proxy.provider` takes it.
pub(crate) const NAME: &str = "oxylabs";

/// Oxylabs' Residential Proxies entry.
const PUBLIC: &str = "http://pr.oxylabs.io:7777";

/// The longest `sesstime` Oxylabs takes.
const LONGEST: Duration = Duration::from_hours(24);

/// The `oxylabs` adapter of one account.
pub(crate) struct Oxylabs {
    offer: Offer,
    customer: String,
}

impl Oxylabs {
    /// The adapter of `configuration`'s account, its proxy user.
    ///
    /// # Errors
    ///
    /// What is missing, for `setting_invalid`.
    pub(crate) fn new(configuration: &ProxyProviderConfiguration) -> Result<Self, String> {
        let customer = account(configuration, NAME, "the proxy user's name")?;
        Ok(Self {
            offer: hosted(configuration, PUBLIC, &[(ProxyKind::Residential, LONGEST)]),
            customer,
        })
    }
}

impl ProxyProvider for Oxylabs {
    fn offer(&self) -> &Offer {
        &self.offer
    }

    fn username(&self, sticky: &Sticky<'_>) -> Option<String> {
        let country = sticky
            .country
            .map(|country| format!("-cc-{}", CountryCode::as_str(country).to_ascii_uppercase()))
            .unwrap_or_default();
        let lifetime = sticky
            .lifetime
            .as_ref()
            .map(|minutes| format!("-sesstime-{minutes}"))
            .unwrap_or_default();
        Some(format!(
            "customer-{}{country}-sessid-{}{lifetime}",
            self.customer,
            sticky_session(sticky.session)
        ))
    }

    /// Whole minutes, rounded up, at least one.
    fn format_lifetime(&self, lifetime: Duration) -> String {
        lifetime.as_secs().div_ceil(60).max(1).to_string()
    }
}

/// Oxylabs' failure in `answer` (ADR 0127's translation point).
pub(crate) fn failure(answer: &ProviderAnswer) -> ProviderFailure {
    let description = answer
        .header("x-error-description")
        .map(str::to_ascii_lowercase);
    match description.as_deref() {
        Some(text) if text.contains("authentication") => ProviderFailure::BadCredentials,
        Some(text) if text.contains("traffic limit") => ProviderFailure::QuotaExhausted,
        Some(text) if text.contains("session") => ProviderFailure::StickySessionExpired,
        None if answer.status == 407 => ProviderFailure::BadCredentials,
        text => ProviderFailure::Unrecognized(Unrecognized::new(
            NAME,
            format!("{} {}", answer.status, text.unwrap_or("-")),
        )),
    }
}
