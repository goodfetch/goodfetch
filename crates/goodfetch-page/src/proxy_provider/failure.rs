//! What a hosted Proxy provider's failure is, as its adapter's translation
//! point (ADR 0127; spec #599) reads it from the provider's own answer
//! through its proxy: a typed variant for each failure the adapter knows,
//! and [`Unrecognized`] for any other, which keeps the provider's text in a
//! private field no accessor returns, logs it at `warn`, and becomes
//! `proxy_provider_unavailable` as area 7's code has it.

use goodfetch_engine::CallError;

use crate::proxy_status;
use goodfetch_protocol::page::proxy::{
    PROXY_PROVIDER_UNAVAILABLE, ProxyProviderFailure, ProxyProviderUnavailableDetails, ProxyServer,
};

/// The provider's own answer to a request through its proxy: its status
/// and its headers, their names lower-cased.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProviderAnswer {
    /// The status.
    pub(crate) status: u16,
    /// The headers, each name lower-cased, in the order given.
    pub(crate) headers: Vec<(String, String)>,
}

impl ProviderAnswer {
    /// The value of the header `name`, lower-case, if the answer has one.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.trim())
    }

    /// The parameter `name` of the provider's own member of the answer's
    /// `Proxy-Status` field (RFC 9209), the last, which the intermediary
    /// nearest the relay appends: such as `error` or `details`.
    pub(crate) fn proxy_status(&self, name: &str) -> Option<String> {
        let member = proxy_status::nearest(self.header(proxy_status::PROXY_STATUS)?)?;
        proxy_status::parameter(member, name)
    }
}

/// A provider's failure, as its adapter reads it.
#[derive(Debug)]
pub(crate) enum ProviderFailure {
    /// The account's traffic quota or balance is spent.
    QuotaExhausted,
    /// The Session's sticky exit is gone.
    StickySessionExpired,
    /// The provider refused the account's credentials, which a Session
    /// raises as `proxy_auth_failed`.
    BadCredentials,
    /// A failure the adapter does not know.
    Unrecognized(
        #[expect(
            dead_code,
            reason = "ADR 0127: the unknown failure is kept as it was read, and no code reads it"
        )]
        Unrecognized,
    ),
}

/// A failure the adapter does not know: the provider's text, kept and
/// logged, never returned.
pub(crate) struct Unrecognized {
    #[expect(
        dead_code,
        reason = "ADR 0127: the text is kept for the failure's life and logged where it is read, and no code reads it"
    )]
    text: String,
}

impl Unrecognized {
    /// The unknown failure `provider` answered with `text`, logged now.
    pub(crate) fn new(provider: &str, text: String) -> Self {
        tracing::warn!(
            provider,
            answer = %text,
            "a Proxy provider answered with a failure its adapter does not recognize"
        );
        Self { text }
    }
}

impl std::fmt::Debug for Unrecognized {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Unrecognized")
            .finish_non_exhaustive()
    }
}

impl ProviderFailure {
    /// The failure's name in `proxy_provider_unavailable`'s details; none
    /// for a refusal of the credentials, which is `proxy_auth_failed`.
    pub(crate) fn unavailable_as(&self) -> Option<ProxyProviderFailure> {
        match self {
            Self::QuotaExhausted => Some(ProxyProviderFailure::QuotaExhausted),
            Self::StickySessionExpired => Some(ProxyProviderFailure::StickySessionExpired),
            Self::Unrecognized(_) => Some(ProxyProviderFailure::Unrecognized),
            Self::BadCredentials => None,
        }
    }

    /// `proxy_provider_unavailable` for the failure `provider`'s proxy
    /// `proxy` gave a request to `url`; none for a refusal of the
    /// credentials, which is `proxy_auth_failed`.
    pub(crate) fn unavailable(
        &self,
        provider: &str,
        proxy: &ProxyServer,
        url: Option<&str>,
    ) -> Option<CallError> {
        let failure = self.unavailable_as()?;
        let what = match failure {
            ProxyProviderFailure::QuotaExhausted => {
                "says the account's traffic quota or balance is spent: add to it with the provider, or use another provider"
            }
            ProxyProviderFailure::StickySessionExpired => {
                "says the Session's sticky exit is gone: open a new Session for a new exit"
            }
            ProxyProviderFailure::Unrecognized => {
                "answered with a failure Goodfetch does not recognize, which the engine's log holds: retry later"
            }
        };
        Some(CallError::new(
            &PROXY_PROVIDER_UNAVAILABLE,
            format!("the Proxy provider `{provider}`'s proxy {proxy} {what}"),
            &ProxyProviderUnavailableDetails {
                provider: provider.to_owned(),
                proxy: proxy.to_string(),
                url: url.map(str::to_owned),
                failure,
            },
        ))
    }
}
