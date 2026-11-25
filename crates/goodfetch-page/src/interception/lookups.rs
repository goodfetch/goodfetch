//! How interception resolves the host of a paused request of a browser no
//! egress relay is under, so the pipeline judges its addresses (ADRs 0017
//! and 0056; #2435): a Borrowed browser, and a browser the engine launched
//! with no relay.

use goodfetch_engine::dns::HostMapping;
use goodfetch_engine::egress::BrowserLocation;
use goodfetch_engine::http_client;
use tokio::sync::OnceCell;

use crate::borrowed::CdpUrl;

/// The browser of a Session that interception alone holds to the policy,
/// with no egress relay under it.
#[derive(Clone, Debug)]
pub(crate) enum HeldBrowser {
    /// A Borrowed browser at its `cdp_url`, whose addresses tell where it
    /// is.
    Borrowed(CdpUrl),
    /// A browser the engine launched with no relay: with
    /// `security.raw_cdp_egress_proxy` off and nothing else asking for one.
    Launched {
        /// Whether the Session's proxy takes its requests, resolving their
        /// names on its own network.
        proxied: bool,
    },
}

/// Where a name the engine cannot resolve would be resolved.
enum Place {
    /// Known when the Session opens: a launched browser's.
    Known(BrowserLocation),
    /// A Borrowed browser's, told by the addresses of its `cdp_url`,
    /// looked up once, when such a name first comes.
    Borrowed {
        cdp_url: CdpUrl,
        location: OnceCell<BrowserLocation>,
    },
}

/// How the engine resolves the hosts of the requests of a browser no relay
/// is under: through the Run's `network.resolve` mapping, and, for a name
/// it cannot resolve, by where the name would be resolved.
pub(crate) struct Lookups {
    resolve: HostMapping,
    place: Place,
}

impl Lookups {
    /// The lookups of `browser`, under the mapping `resolve`. A launched
    /// browser is on the engine's machine, but behind a proxy, which
    /// resolves names elsewhere.
    pub(crate) fn new(resolve: HostMapping, browser: HeldBrowser) -> Self {
        let place = match browser {
            HeldBrowser::Borrowed(cdp_url) => Place::Borrowed {
                cdp_url,
                location: OnceCell::new(),
            },
            HeldBrowser::Launched { proxied: true } => Place::Known(BrowserLocation::Elsewhere),
            HeldBrowser::Launched { proxied: false } => {
                Place::Known(BrowserLocation::EnginesMachine)
            }
        };
        Self { resolve, place }
    }

    /// The Run's `network.resolve` mapping.
    pub(crate) fn resolve(&self) -> &HostMapping {
        &self.resolve
    }

    /// Whether the browser is a Borrowed one, whose network the engine
    /// holds nothing of.
    pub(crate) fn borrowed(&self) -> bool {
        matches!(self.place, Place::Borrowed { .. })
    }

    /// Where a name the engine cannot resolve would be resolved. A
    /// Borrowed browser is on the engine's machine when its `cdp_url`
    /// resolves, as the engine connected to it, to loopback alone; else, or
    /// when it no longer resolves, elsewhere.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future abandons the lookup; the next call makes it.
    pub(crate) async fn location(&self) -> BrowserLocation {
        let (cdp_url, location) = match &self.place {
            Place::Known(location) => return *location,
            Place::Borrowed { cdp_url, location } => (cdp_url, location),
        };
        *location
            .get_or_init(|| async {
                let Some((host, port)) = cdp_url.host_and_port() else {
                    return BrowserLocation::Elsewhere;
                };
                match http_client::lookup(&host, port, &self.resolve).await {
                    Ok(addresses) => BrowserLocation::of(&addresses),
                    Err(_) => BrowserLocation::Elsewhere,
                }
            })
            .await
    }
}
