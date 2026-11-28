//! How interception resolves the host of a paused request of a browser no
//! egress relay is under, so the pipeline judges its addresses (ADRs 0017
//! and 0056; #2435, #742): a Borrowed browser, and a browser the engine
//! launched with no relay.
//!
//! Resolved hosts are held in a bounded per-Session cache (`MAX_CACHED_LOOKUPS`
//! entries, `CACHE_TTL` lifetime) so repeated requests to the same host on a
//! Borrowed browser do not spawn tasks or open resolver files per request
//! (ADR 0140, register rows `C5.spawns`, `C5.file_opens`).

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::HostMapping;
use goodfetch_engine::egress::BrowserLocation;
use goodfetch_engine::http_client::{self, Failure};
use tokio::sync::OnceCell;
use tokio::time::Instant;

use crate::cdp::borrowed::CdpUrl;

/// Maximum number of `(host, port)` lookup entries cached per Session.
const MAX_CACHED_LOOKUPS: usize = 256;

/// How long a cached host resolution remains valid inside a Session.
const CACHE_TTL: Duration = Duration::from_secs(30);

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

struct CachedLookup {
    addresses: Arc<[SocketAddr]>,
    expires_at: Instant,
}

#[derive(Default)]
struct LookupCache {
    entries: HashMap<(String, u16), CachedLookup>,
    order: VecDeque<(String, u16)>,
}

impl LookupCache {
    fn get(&mut self, key: &(String, u16), now: Instant) -> Option<Arc<[SocketAddr]>> {
        let valid = self
            .entries
            .get(key)
            .is_some_and(|entry| now < entry.expires_at);
        if valid {
            return self
                .entries
                .get(key)
                .map(|entry| Arc::clone(&entry.addresses));
        }
        self.entries.remove(key);
        None
    }

    fn insert(&mut self, key: (String, u16), addresses: Arc<[SocketAddr]>, expires_at: Instant) {
        if let Some(existing) = self.entries.get_mut(&key) {
            existing.addresses = addresses;
            existing.expires_at = expires_at;
            return;
        }
        while self.entries.len() >= MAX_CACHED_LOOKUPS {
            let Some(oldest) = self.order.pop_front() else {
                self.entries.clear();
                break;
            };
            self.entries.remove(&oldest);
        }
        self.order.push_back(key.clone());
        self.entries.insert(
            key,
            CachedLookup {
                addresses,
                expires_at,
            },
        );
    }
}

/// How the engine resolves the hosts of the requests of a browser no relay
/// is under: through the Run's `network.resolve` mapping, backed by a bounded
/// per-Session cache, and, for a name it cannot resolve, by where the name
/// would be resolved.
pub(crate) struct Lookups {
    resolve: HostMapping,
    place: Place,
    clock: Clock,
    cache: Mutex<LookupCache>,
}

impl Lookups {
    /// The lookups of `browser`, under the mapping `resolve`, bounded by
    /// `clock`. A launched browser is on the engine's machine, but behind a
    /// proxy, which resolves names elsewhere.
    pub(crate) fn new(resolve: HostMapping, browser: HeldBrowser, clock: Clock) -> Self {
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
        Self {
            resolve,
            place,
            clock,
            cache: Mutex::new(LookupCache::default()),
        }
    }

    /// Resolves `(host, port)` through the bounded per-Session cache first,
    /// falling back to `http_client::lookup` under the Run's `network.resolve`
    /// mapping (ADR 0140, register rows `C5.spawns` and `C5.file_opens`).
    pub(crate) async fn lookup(&self, host: &str, port: u16) -> Result<Arc<[SocketAddr]>, Failure> {
        let key = (host.to_ascii_lowercase(), port);
        let now = self.clock.now();
        if let Some(cached) = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key, now)
        {
            return Ok(cached);
        }
        let resolved: Arc<[SocketAddr]> =
            http_client::lookup(host, port, &self.resolve).await?.into();
        let expires_at = self.clock.now() + CACHE_TTL;
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, Arc::clone(&resolved), expires_at);
        Ok(resolved)
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

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;
    use std::time::Duration;

    use super::{CACHE_TTL, LookupCache, MAX_CACHED_LOOKUPS};

    #[test]
    fn lookup_cache_evicts_oldest_when_bounded_capacity_is_reached() {
        goodfetch_test_support::runtime().block_on(async {
            let clock = goodfetch_engine::clock::Clock::system();
            let now = clock.now();
            let expires = now + CACHE_TTL;
            let mut cache = LookupCache::default();
            let addr: Arc<[SocketAddr]> = vec![SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(93, 184, 215, 14)),
                80,
            )]
            .into();

            for i in 0..=MAX_CACHED_LOOKUPS {
                cache.insert((format!("host-{i}.test"), 80), Arc::clone(&addr), expires);
            }

            assert_eq!(cache.entries.len(), MAX_CACHED_LOOKUPS);
            assert!(cache.get(&("host-0.test".to_owned(), 80), now).is_none());
            assert!(
                cache
                    .get(&(format!("host-{MAX_CACHED_LOOKUPS}.test"), 80), now)
                    .is_some()
            );
            assert!(
                cache
                    .get(
                        &(format!("host-{MAX_CACHED_LOOKUPS}.test"), 80),
                        expires + Duration::from_secs(1)
                    )
                    .is_none()
            );
        });
    }
}
