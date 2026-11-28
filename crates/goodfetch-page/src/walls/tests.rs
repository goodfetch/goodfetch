//! Unit tests for the Wall allowance policy, withholds-content test, refetch
//! eligibility/stops, and `setting_locked` refusals (ADR 0052; ticket #757).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_clean_up_rules::walls::{WITHHOLDS_CONTENT_MIN_WORDS, withholds_content};
use goodfetch_engine::settings::{Layers, Policy, Resolved, Values, resolve as resolve_settings};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::settings::{SETTING_LOCKED, SettingsLayer, WALL_REFETCH_DOMAINS};
use serde_json::{Map, Value, json};

use super::{
    OnWall, RefetchEligibility, RefetchSkipReason, RunWalls, WallPolicy,
    refuse_refetch_answer_widening,
};

fn values(layer: SettingsLayer, settings: &Value) -> Values {
    let nested: Map<String, Value> = serde_json::from_value(settings.clone()).expect("an object");
    Values::read(layer, &nested, Product::Collie).expect("settings read")
}

fn resolved(operator_policy: &Value, user_configuration: &Value) -> Resolved {
    let layers = Layers {
        operator_policy: Policy {
            settings: values(SettingsLayer::OperatorPolicy, operator_policy),
            ..Policy::default()
        },
        user_configuration: values(SettingsLayer::UserConfiguration, user_configuration),
        ..Layers::default()
    };
    resolve_settings(&layers, Product::Collie, Product::Collie).expect("valid settings")
}

/// Covers: story:5-102, setting:wall.domains, setting:wall.refetch_domains, setting:wall.denied_domains, setting:wall.on_wall
#[test]
fn denied_domains_always_win_over_wall_domains_and_refetch_domains_including_star() {
    let settings = resolved(
        &json!({ "wall": { "denied_domains": ["blocked.test"] } }),
        &json!({
            "wall": {
                "domains": ["*"],
                "refetch_domains": ["*"],
                "on_wall": "ask",
            }
        }),
    );
    let policy = WallPolicy::of(&settings);

    assert!(policy.is_allowed("allowed.test"));
    assert!(policy.is_refetch_allowed("allowed.test"));
    assert_eq!(policy.on_wall_for("allowed.test"), OnWall::Ask);

    assert!(policy.is_denied("blocked.test"));
    assert!(!policy.is_allowed("blocked.test"));
    assert!(!policy.is_refetch_allowed("blocked.test"));
    // Denied domain forces `wall.on_wall = ask` to act as `report`.
    assert_eq!(policy.on_wall_for("blocked.test"), OnWall::Report);
}

/// Covers: story:5-102, setting:wall.refetch_domains
#[test]
fn refetch_runs_at_most_once_per_page_and_respects_logged_in_identified_exit_and_stop() {
    let walls = RunWalls::default();
    let session: SessionId = serde_json::from_value(json!("s1")).expect("session id");
    let ok = RefetchEligibility {
        logged_in: false,
        identified: false,
        exit_kept: true,
    };

    assert_eq!(
        walls.try_begin_refetch(
            "https://news.test/a",
            "news.test",
            &session,
            RefetchEligibility {
                logged_in: true,
                ..ok
            }
        ),
        Err(RefetchSkipReason::LoggedIn)
    );
    assert_eq!(
        walls.try_begin_refetch(
            "https://news.test/a",
            "news.test",
            &session,
            RefetchEligibility {
                identified: true,
                ..ok
            }
        ),
        Err(RefetchSkipReason::Identified)
    );
    assert_eq!(
        walls.try_begin_refetch(
            "https://news.test/a",
            "news.test",
            &session,
            RefetchEligibility {
                exit_kept: false,
                ..ok
            }
        ),
        Err(RefetchSkipReason::ExitNotKept)
    );

    // First refetch succeeds; second for the same URL is skipped.
    assert_eq!(
        walls.try_begin_refetch("https://news.test/a", "news.test", &session, ok),
        Ok(())
    );
    assert!(walls.session_cleared(&session, "news.test"));
    assert_eq!(
        walls.try_begin_refetch("https://news.test/a", "news.test", &session, ok),
        Err(RefetchSkipReason::AlreadyRefetched)
    );

    // Stopping the domain stops any new page on that domain for the rest of the run.
    walls.mark_stopped("news.test");
    assert!(walls.is_stopped("news.test"));
    assert_eq!(
        walls.try_begin_refetch("https://news.test/b", "news.test", &session, ok),
        Err(RefetchSkipReason::StoppedForDomain)
    );

    // Audit dedup: once per (run, domain).
    assert!(walls.mark_audited("news.test"));
    assert!(!walls.mark_audited("news.test"));
}

/// Covers: story:5-102, error:setting_locked, setting:wall.refetch_domains
#[test]
fn widening_wall_refetch_domains_by_answer_is_refused_with_setting_locked() {
    let err = refuse_refetch_answer_widening(SettingsLayer::Run);
    assert_eq!(err.code(), SETTING_LOCKED.name);
    assert_eq!(
        err.details().expect("details")["key"],
        WALL_REFETCH_DOMAINS.name
    );

    // Withholds-content test: paid section empty OR < 150 words.
    assert!(withholds_content(true, true, 500));
    assert!(withholds_content(true, false, 149));
    assert!(!withholds_content(true, false, WITHHOLDS_CONTENT_MIN_WORDS));
    assert!(!withholds_content(false, true, 0));
}
