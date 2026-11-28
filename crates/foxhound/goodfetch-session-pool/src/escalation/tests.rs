//! `escalation`'s decisions (spec #600's seam: a pure function of the
//! cause and where its Session met it).

use goodfetch_engine::transition::Refusal;
use goodfetch_protocol::connection::INTERNAL;
use goodfetch_protocol::foxhound::block::BlockCause;
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::foxhound::session_pool::ProxyTier;
use goodfetch_protocol::page::proxy::ProxyKind;
use rstest::rstest;
use serde_json::json;

use super::{
    Action, Event, Ladder, LadderRung, Met, OBJECT, State, TierPosition, Transition,
    TransitionEffect, apply, on_block, refusal_error,
};

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

const HIGHER: TierPosition = TierPosition {
    has_higher_kind: true,
    below_is_lowest: false,
};

const AT_TOP: TierPosition = TierPosition {
    has_higher_kind: false,
    below_is_lowest: false,
};

const ONE_ABOVE_BASE: TierPosition = TierPosition {
    has_higher_kind: false,
    below_is_lowest: true,
};

/// Every one of the 21 valid pairs of the 27-pair escalation state table
/// (18 acting rows + 3 late rows), copied from spec #600 (ADR 0128).
///
/// Covers: story:8-240
#[rstest]
#[case::base_block_without_challenge_with_higher_kind(
    State::Base,
    Event::BlockWithoutChallenge,
    HIGHER,
    State::Escalated,
    TransitionEffect::EscalateOrRotate
)]
#[case::base_block_without_challenge_at_top(
    State::Base,
    Event::BlockWithoutChallenge,
    AT_TOP,
    State::Base,
    TransitionEffect::EscalateOrRotate
)]
#[case::base_challenge_on_http_tier(
    State::Base,
    Event::ChallengeOnHttpTier,
    HIGHER,
    State::Base,
    TransitionEffect::MoveToBrowser
)]
#[case::base_challenge_on_browser(
    State::Base,
    Event::ChallengeOnBrowser,
    HIGHER,
    State::Base,
    TransitionEffect::RunLadder
)]
#[case::base_challenge_cleared(
    State::Base,
    Event::ChallengeCleared,
    HIGHER,
    State::Base,
    TransitionEffect::RecordLadderStart
)]
#[case::base_stopping_cause(
    State::Base,
    Event::StoppingCause,
    HIGHER,
    State::Base,
    TransitionEffect::StopRequest
)]
#[case::base_block_on_retired_session(
    State::Base,
    Event::BlockOnRetiredSession,
    HIGHER,
    State::Base,
    TransitionEffect::IgnoreLate
)]
#[case::escalated_block_without_challenge(
    State::Escalated,
    Event::BlockWithoutChallenge,
    HIGHER,
    State::Escalated,
    TransitionEffect::EscalateOrRotate
)]
#[case::escalated_challenge_on_http_tier(
    State::Escalated,
    Event::ChallengeOnHttpTier,
    HIGHER,
    State::Escalated,
    TransitionEffect::MoveToBrowser
)]
#[case::escalated_challenge_on_browser(
    State::Escalated,
    Event::ChallengeOnBrowser,
    HIGHER,
    State::Escalated,
    TransitionEffect::RunLadder
)]
#[case::escalated_challenge_cleared(
    State::Escalated,
    Event::ChallengeCleared,
    HIGHER,
    State::Escalated,
    TransitionEffect::RecordLadderStart
)]
#[case::escalated_stopping_cause(
    State::Escalated,
    Event::StoppingCause,
    HIGHER,
    State::Escalated,
    TransitionEffect::StopRequest
)]
#[case::escalated_trial_due(
    State::Escalated,
    Event::TrialDue,
    HIGHER,
    State::Trialing,
    TransitionEffect::StartTrial
)]
#[case::escalated_block_on_retired_session(
    State::Escalated,
    Event::BlockOnRetiredSession,
    HIGHER,
    State::Escalated,
    TransitionEffect::IgnoreLate
)]
#[case::trialing_block_without_challenge(
    State::Trialing,
    Event::BlockWithoutChallenge,
    HIGHER,
    State::Trialing,
    TransitionEffect::EscalateOrRotate
)]
#[case::trialing_challenge_on_http_tier(
    State::Trialing,
    Event::ChallengeOnHttpTier,
    HIGHER,
    State::Trialing,
    TransitionEffect::MoveToBrowser
)]
#[case::trialing_challenge_on_browser(
    State::Trialing,
    Event::ChallengeOnBrowser,
    HIGHER,
    State::Trialing,
    TransitionEffect::RunLadder
)]
#[case::trialing_challenge_cleared(
    State::Trialing,
    Event::ChallengeCleared,
    HIGHER,
    State::Trialing,
    TransitionEffect::RecordLadderStart
)]
#[case::trialing_stopping_cause(
    State::Trialing,
    Event::StoppingCause,
    HIGHER,
    State::Trialing,
    TransitionEffect::StopRequest
)]
#[case::trialing_trial_passed_to_base(
    State::Trialing,
    Event::TrialPassed,
    ONE_ABOVE_BASE,
    State::Base,
    TransitionEffect::StepDown
)]
#[case::trialing_trial_passed_still_escalated(
    State::Trialing,
    Event::TrialPassed,
    AT_TOP,
    State::Escalated,
    TransitionEffect::StepDown
)]
#[case::trialing_trial_blocked(
    State::Trialing,
    Event::TrialBlocked,
    ONE_ABOVE_BASE,
    State::Escalated,
    TransitionEffect::KeepEscalated
)]
#[case::trialing_block_on_retired_session(
    State::Trialing,
    Event::BlockOnRetiredSession,
    HIGHER,
    State::Trialing,
    TransitionEffect::IgnoreLate
)]
fn every_valid_pair_of_the_escalation_state_table_matches_spec_600(
    #[case] state: State,
    #[case] event: Event,
    #[case] position: TierPosition,
    #[case] next: State,
    #[case] effect: TransitionEffect,
) {
    assert_eq!(
        apply(state, event, position),
        Ok(Transition { next, effect })
    );
}

/// The 6 refused pairs of the 27-pair escalation state table each return
/// `Refusal::Internal` and build an `internal` error naming `escalation`,
/// the state and the event (ADR 0128; spec #600).
///
/// Covers: story:8-240
#[rstest]
#[case::base_trial_due(State::Base, Event::TrialDue)]
#[case::base_trial_passed(State::Base, Event::TrialPassed)]
#[case::base_trial_blocked(State::Base, Event::TrialBlocked)]
#[case::escalated_trial_passed(State::Escalated, Event::TrialPassed)]
#[case::escalated_trial_blocked(State::Escalated, Event::TrialBlocked)]
#[case::trialing_trial_due(State::Trialing, Event::TrialDue)]
fn every_refused_pair_of_the_escalation_state_table_returns_internal_naming_object_state_and_event(
    #[case] state: State,
    #[case] event: Event,
) {
    let invalid = apply(state, event, HIGHER).expect_err("pair must be refused");
    assert_eq!(
        (invalid.state, invalid.event, invalid.refusal),
        (state, event, Refusal::Internal)
    );
    let error = refusal_error(invalid);
    assert_eq!(error.code(), INTERNAL.name);
    assert_eq!(
        error.details(),
        Some(
            &json!({
                "object": OBJECT,
                "state": state.as_str(),
                "event": event.as_str(),
            })
            .as_object()
            .unwrap()
            .clone()
        )
    );
}

/// A `trialing` record restores from a Checkpoint as `escalated`; `base`
/// and `escalated` restore unchanged (spec #600).
///
/// Covers: story:8-219, story:8-240
#[rstest]
#[case::base(State::Base, State::Base)]
#[case::escalated(State::Escalated, State::Escalated)]
#[case::trialing_restores_as_escalated(State::Trialing, State::Escalated)]
fn checkpoint_restore_normalizes_trialing_to_escalated(
    #[case] stored: State,
    #[case] restored: State,
) {
    assert_eq!(stored.from_checkpoint(), restored);
}

/// `Ladder::build` orders configured proxy kinds cheapest first, narrows by
/// `escalation.proxy_kinds`, starts at `proxy.kind` when set without
/// narrowing, and stays empty when no paid pool is configured.
///
/// Covers: story:8-221
#[test]
fn ladder_orders_cheapest_first_narrows_and_spends_nothing_when_unconfigured() {
    let rung = |kind| LadderRung {
        kind,
        rotatable: false,
    };
    let configured = [
        rung(ProxyKind::Residential),
        rung(ProxyKind::Datacenter),
        rung(ProxyKind::Isp),
    ];
    let default = Ladder::build(&configured, None, None);
    assert_eq!(
        default.kinds(),
        vec![
            ProxyKind::Datacenter,
            ProxyKind::Isp,
            ProxyKind::Residential
        ]
    );
    assert_eq!(default.base_tier(), ProxyTier::Datacenter);
    assert_eq!(
        default.next_tier(ProxyTier::Datacenter),
        Some(ProxyTier::Isp)
    );
    assert_eq!(
        default.next_tier(ProxyTier::Isp),
        Some(ProxyTier::Residential)
    );
    assert_eq!(default.next_tier(ProxyTier::Residential), None);
    assert_eq!(
        default.lower_tier(ProxyTier::Residential),
        Some(ProxyTier::Isp)
    );
    assert_eq!(default.lower_tier(ProxyTier::Datacenter), None);

    let narrowed = Ladder::build(
        &configured,
        None,
        Some(&[ProxyKind::Datacenter, ProxyKind::Isp, ProxyKind::Mobile]),
    );
    assert_eq!(
        narrowed.kinds(),
        vec![ProxyKind::Datacenter, ProxyKind::Isp]
    );

    let starting_at_isp = Ladder::build(&configured, Some(ProxyKind::Isp), None);
    assert_eq!(
        starting_at_isp.kinds(),
        vec![ProxyKind::Isp, ProxyKind::Residential]
    );

    let empty = Ladder::build(&[], None, None);
    assert!(empty.is_empty());
    assert_eq!(empty.base_tier(), ProxyTier::None);
    assert_eq!(empty.next_tier(ProxyTier::None), None);
    assert_eq!(empty.budget_per_request(), 0);
}

/// `escalation` decides exclusively from typed `BlockCause` and `Event`
/// variants, never from error message text: a reworded transport failure
/// changes only area 5's translation table (ADR 0127).
///
/// Covers: story:8-241
#[test]
fn escalation_decides_from_typed_variants_never_failure_text() {
    let ladder = Ladder::from_kinds(vec![ProxyKind::Datacenter, ProxyKind::Isp]);
    let position = ladder.position(ProxyTier::Datacenter);
    assert_eq!(
        apply(State::Base, Event::BlockWithoutChallenge, position),
        Ok(Transition {
            next: State::Escalated,
            effect: TransitionEffect::EscalateOrRotate,
        })
    );
    assert_eq!(
        on_block(&BlockCause::IpBlock, by_transport(FetchTier::Http, false)),
        Action::Sample
    );
    assert_eq!(
        on_block(&BlockCause::RateLimit, on(FetchTier::Http, false)),
        Action::Blocked
    );
}
