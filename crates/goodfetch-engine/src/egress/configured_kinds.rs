//! How the egress policy classes each kind of destination the operator
//! configures (ADRs 0017, 0043, 0047 and 0056): the two [`EgressClass`]es,
//! each [`ConfiguredKind`] and its row of [`CONFIGURED_KINDS`]. Every caller
//! of the engine's own HTTP client but a pinned download from its pinned URL
//! and a run's own requests connects to one of these kinds.

use goodfetch_protocol::settings::SettingsLayer;

/// The two classes of outgoing connection (ADR 0017), which the egress
/// policy judges each by its own rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EgressClass {
    /// An address a page can reach: a start URL, a navigation, a redirect,
    /// a subresource, a worker, a WebSocket, a page's own request. The
    /// run's domain policy judges it, and no address that is not globally
    /// reachable is reached unless `egress.private_ranges` lists it.
    PageReachable,
    /// An Operator-configured endpoint: private ranges are reached unless
    /// `egress.no_private_endpoints` refuses them, and no run's domain
    /// policy or certificate exception applies.
    OperatorConfigured,
}

/// A kind of destination the operator configures (ADRs 0017 and 0043),
/// which the engine connects to on its own behalf.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConfiguredKind {
    /// A Model provider, such as a model on localhost.
    ModelProvider,
    /// An external Secret provider: a vault, a cloud's secret manager.
    SecretProvider,
    /// A Session's upstream proxy: the proxy's own address.
    Proxy,
    /// A Notification webhook.
    NotificationWebhook,
    /// An OTLP collector: `otlp.traces_endpoint`, `otlp.metrics_endpoint`.
    Otlp,
    /// The Plugin index.
    PluginIndex,
    /// A `cdp_url` to connect to a browser through.
    CdpUrl,
    /// A source of the Clean-up rules, which `clean_up.update`'s `daily`
    /// and `upstream` make a configured destination (ADR 0047).
    CleanUpRulesSource,
    /// A `browser.mirror` the pinned Chrome for Testing builds are
    /// downloaded below in place of Chrome for Testing's storage (ADR 0047).
    BrowserMirror,
}

/// The layers whose value makes a destination Operator-configured: the
/// Operator policy and user and project configuration.
const CONFIGURATION: &[SettingsLayer] = &[
    SettingsLayer::OperatorPolicy,
    SettingsLayer::UserConfiguration,
    SettingsLayer::ProjectConfiguration,
];

/// One row of [`CONFIGURED_KINDS`]: a kind of configured destination and
/// how the egress policy classes it.
#[derive(Clone, Copy, Debug)]
pub struct ConfiguredKindClass {
    /// The kind.
    pub kind: ConfiguredKind,
    /// Its name in prose and in the engine's logs.
    pub name: &'static str,
    /// The layers whose value makes it an
    /// [`EgressClass::OperatorConfigured`] endpoint; from any other, it is
    /// an address a page can reach.
    pub operator_layers: &'static [SettingsLayer],
}

impl ConfiguredKindClass {
    /// The class of a destination of this kind whose value `layer` set.
    pub fn class(&self, layer: SettingsLayer) -> EgressClass {
        if self.operator_layers.contains(&layer) {
            EgressClass::OperatorConfigured
        } else {
            EgressClass::PageReachable
        }
    }
}

/// How each kind of configured destination is classed (ADRs 0017, 0043,
/// 0047 and 0056): an Operator-configured endpoint when the Operator policy
/// or user or project configuration sets it. Only a `cdp_url` can come
/// from a Recipe or a run, and is then an address a page can reach unless
/// the Operator policy allows it; so is one in the project configuration a
/// Client acting for a model sends (ADR 0056, read where `cdp_url` is
/// judged). Every caller of the engine's own HTTP client but a pinned
/// download from its pinned URL and a run's own requests connects to one
/// of these kinds.
pub const CONFIGURED_KINDS: [ConfiguredKindClass; 9] = [
    ConfiguredKindClass {
        kind: ConfiguredKind::ModelProvider,
        name: "Model provider",
        operator_layers: CONFIGURATION,
    },
    ConfiguredKindClass {
        kind: ConfiguredKind::SecretProvider,
        name: "Secret provider",
        operator_layers: CONFIGURATION,
    },
    ConfiguredKindClass {
        kind: ConfiguredKind::Proxy,
        name: "proxy",
        operator_layers: CONFIGURATION,
    },
    ConfiguredKindClass {
        kind: ConfiguredKind::NotificationWebhook,
        name: "Notification webhook",
        operator_layers: CONFIGURATION,
    },
    ConfiguredKindClass {
        kind: ConfiguredKind::Otlp,
        name: "OTLP",
        operator_layers: CONFIGURATION,
    },
    ConfiguredKindClass {
        kind: ConfiguredKind::PluginIndex,
        name: "Plugin index",
        operator_layers: CONFIGURATION,
    },
    ConfiguredKindClass {
        kind: ConfiguredKind::CdpUrl,
        name: "cdp_url",
        operator_layers: CONFIGURATION,
    },
    ConfiguredKindClass {
        kind: ConfiguredKind::CleanUpRulesSource,
        name: "Clean-up rules source",
        operator_layers: CONFIGURATION,
    },
    ConfiguredKindClass {
        kind: ConfiguredKind::BrowserMirror,
        name: "browser mirror",
        operator_layers: CONFIGURATION,
    },
];

impl ConfiguredKind {
    /// Its row of [`CONFIGURED_KINDS`].
    pub fn classed(self) -> &'static ConfiguredKindClass {
        CONFIGURED_KINDS
            .iter()
            .find(|row| row.kind == self)
            .expect("invariant: every configured kind has its row")
    }
}

#[cfg(test)]
mod tests;
