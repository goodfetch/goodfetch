//! What a crawl does on a block (ADRs 0020, 0021, 0035 and 0093; spec
//! #600's `escalation`): [`on_block`] decides, from the Block cause and
//! where its Session met it, whether the request moves to the browser tier
//! or ends `blocked`. A Challenge met on a tier short of the browser
//! (`http`, `discovered_api`, `browser_fetch`, `light_browser`) moves the
//! request to the browser, in the same Session when it sends from its
//! browser, else in one whose client is its browser (#2416), and its
//! domain learns the move ([`crate::SessionPool::moved_to_browser`]), so a
//! Challenge is met where it can be passed; under a `fetch.tier` forced to
//! that tier, it ends `blocked` instead, since a forced tier is never left
//! (ADR 0093).
//! An IP block met by a request a Session's own transport sent, on the
//! tier its Label's comparison chose (`fetch.http_from: learned` or
//! `transport`), is answered on the fetch tier first (#2419): the Label's
//! choice on the domain is dropped and the request runs again as a
//! sample, whose comparison moves the Label to `browser_fetch` or the
//! browser; the proxy tier moves only if the browser is refused too. A
//! sample's own runs, a forced tier and a request sent from the browser
//! keep the paths below.
//! A block the program's own code names (a handler's `blocked`, as an
//! after-response Hook's `block(cause)` will, ADR 0104) is decided the
//! same way as one the Block classification rules detected. Every other
//! block ends the request `blocked`; the pool's `settle` then retires its
//! Session as the cause says. Until Collie's Challenge ladder is wired
//! into a Session pool, a Challenge on the browser ends `blocked` too.
//! The per-domain record and the proxy-tier climb arrive with the next
//! tickets of the spec.

use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::fetch::FetchTier;

/// Where a Session met a block: the tier its request was fetched on, none
/// when the handler fetched it itself, whether `fetch.tier` forced that
/// tier, and whether the Session's own transport sent it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Met {
    /// The tier.
    pub tier: Option<FetchTier>,
    /// Whether `fetch.tier` forced it.
    pub forced: bool,
    /// Whether the Session's own transport sent the request, on the tier
    /// its Label's comparison chose, not as one of a sample's runs.
    pub from_transport: bool,
}

/// What a crawl does with a request that met a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The request runs again on the browser tier, in the same attempt
    /// and Session, and its domain's later requests start there.
    MoveToBrowser,
    /// The transport was refused: the Label's choice on the domain is
    /// dropped, and the request runs again as a sample.
    Sample,
    /// The request ends `blocked` with its cause.
    Blocked,
}

/// What a crawl does with a request whose Session met a block of `cause`
/// where `met` says (the module's doc).
pub fn on_block(cause: &BlockCause, met: Met) -> Action {
    let challenge = matches!(cause, BlockCause::Challenge | BlockCause::Captcha);
    if *cause == BlockCause::IpBlock && met.from_transport && !met.forced {
        return Action::Sample;
    }
    match met.tier {
        Some(
            FetchTier::Http
            | FetchTier::DiscoveredApi
            | FetchTier::BrowserFetch
            | FetchTier::LightBrowser,
        ) if challenge && !met.forced => Action::MoveToBrowser,
        Some(_) | None => Action::Blocked,
    }
}

#[cfg(test)]
mod tests;
