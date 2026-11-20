//! `escalation`'s decisions (spec #600's seam: a pure function of the
//! cause and where its Session met it).

use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::fetch::FetchTier;
use rstest::rstest;

use super::{Action, Met, on_block};

const fn on(tier: FetchTier, forced: bool) -> Met {
    Met {
        tier: Some(tier),
        forced,
        from_transport: false,
    }
}

/// Met on `tier`, sent by its Session's own transport on the tier its
/// Label's comparison chose.
const fn by_transport(tier: FetchTier, forced: bool) -> Met {
    Met {
        tier: Some(tier),
        forced,
        from_transport: true,
    }
}

#[rstest]
/// Covers: story:8-209
#[case::a_challenge_on_http(
    BlockCause::Challenge,
    on(FetchTier::Http, false),
    Action::MoveToBrowser
)]
/// Covers: story:8-209
#[case::a_captcha_on_http(BlockCause::Captcha, on(FetchTier::Http, false), Action::MoveToBrowser)]
/// Covers: story:8-209
#[case::a_challenge_on_discovered_api(
    BlockCause::Challenge,
    on(FetchTier::DiscoveredApi, false),
    Action::MoveToBrowser
)]
/// Covers: story:8-209
#[case::a_challenge_on_browser_fetch(
    BlockCause::Challenge,
    on(FetchTier::BrowserFetch, false),
    Action::MoveToBrowser
)]
/// Covers: story:8-209
#[case::a_challenge_on_light_browser(
    BlockCause::Challenge,
    on(FetchTier::LightBrowser, false),
    Action::MoveToBrowser
)]
/// Covers: story:8-210
#[case::a_challenge_on_forced_http(
    BlockCause::Challenge,
    on(FetchTier::Http, true),
    Action::Blocked
)]
/// Covers: story:8-210
#[case::a_challenge_on_forced_light_browser(
    BlockCause::Challenge,
    on(FetchTier::LightBrowser, true),
    Action::Blocked
)]
#[case::a_challenge_on_the_browser(
    BlockCause::Challenge,
    on(FetchTier::Browser, false),
    Action::Blocked
)]
#[case::a_challenge_on_the_forced_browser(
    BlockCause::Challenge,
    on(FetchTier::Browser, true),
    Action::Blocked
)]
#[case::a_challenge_the_handler_fetched(BlockCause::Challenge, Met { tier: None, forced: false, from_transport: false }, Action::Blocked)]
#[case::an_ip_block_on_http(BlockCause::IpBlock, on(FetchTier::Http, false), Action::Blocked)]
#[case::a_rate_limit_on_http(BlockCause::RateLimit, on(FetchTier::Http, false), Action::Blocked)]
#[case::payment_required_on_http(
    BlockCause::PaymentRequired,
    on(FetchTier::Http, false),
    Action::Blocked
)]
#[case::consent_required_on_http(
    BlockCause::ConsentRequired,
    on(FetchTier::Http, false),
    Action::Blocked
)]
#[case::a_decoy_on_http(BlockCause::Decoy, on(FetchTier::Http, false), Action::Blocked)]
fn a_block_moves_its_request_to_the_browser_only_for_a_challenge_on_a_tier_short_of_it_not_forced(
    #[case] cause: BlockCause,
    #[case] met: Met,
    #[case] action: Action,
) {
    assert_eq!(on_block(&cause, met), action);
}

/// A refusal of a Label's transport is answered on the fetch tier first:
/// an IP block met by the request a Session's own transport sent on the
/// tier its Label's comparison chose drops that choice, and the request
/// runs again as a sample (#2419). A rate limit, a Challenge and a refusal
/// of anything else keep their paths.
///
/// Covers: story:6-251
#[rstest]
#[case::an_ip_block_on_http(
    BlockCause::IpBlock,
    by_transport(FetchTier::Http, false),
    Action::Sample
)]
#[case::an_ip_block_on_discovered_api(
    BlockCause::IpBlock,
    by_transport(FetchTier::DiscoveredApi, false),
    Action::Sample
)]
#[case::a_rate_limit_on_http(
    BlockCause::RateLimit,
    by_transport(FetchTier::Http, false),
    Action::Blocked
)]
#[case::a_challenge_on_http(
    BlockCause::Challenge,
    by_transport(FetchTier::Http, false),
    Action::MoveToBrowser
)]
#[case::an_ip_block_on_forced_http(
    BlockCause::IpBlock,
    by_transport(FetchTier::Http, true),
    Action::Blocked
)]
#[case::an_ip_block_on_http_from_the_browser(
    BlockCause::IpBlock,
    on(FetchTier::Http, false),
    Action::Blocked
)]
fn a_refusal_of_the_transport_samples_its_label_again(
    #[case] cause: BlockCause,
    #[case] met: Met,
    #[case] action: Action,
) {
    assert_eq!(on_block(&cause, met), action);
}
