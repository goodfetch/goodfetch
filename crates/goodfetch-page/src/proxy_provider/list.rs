//! The `list` Proxy provider: the user's own servers,
//! `proxy.providers.list.servers`, each with its proxy kind and, if the user
//! names it, its exit's country. A server's exit never changes, and every
//! Session gives it `proxy.providers.list.username` as it is.

use std::time::Duration;

use goodfetch_protocol::page::proxy::ProxyProviderConfiguration;

use super::{Endpoint, Offer, ProxyProvider, Sticky};

/// The `list` provider's name, as `proxy.provider` takes it.
pub(crate) const NAME: &str = "list";

/// The `list` provider of one configuration.
pub(crate) struct List {
    offer: Offer,
    username: Option<String>,
}

impl List {
    /// The provider of `configuration`'s servers.
    pub(crate) fn new(configuration: &ProxyProviderConfiguration) -> Self {
        let endpoints = configuration
            .servers
            .iter()
            .map(|server| Endpoint {
                server: server.server.clone(),
                kinds: vec![server.kind],
                countries: Some(server.country.iter().cloned().collect()),
                longest_lifetime: None,
            })
            .collect();
        Self {
            offer: Offer { endpoints },
            username: configuration.username.clone(),
        }
    }
}

impl ProxyProvider for List {
    fn offer(&self) -> &Offer {
        &self.offer
    }

    fn username(&self, _sticky: &Sticky<'_>) -> Option<String> {
        self.username.clone()
    }

    fn format_lifetime(&self, lifetime: Duration) -> String {
        lifetime.as_millis().to_string()
    }
}
