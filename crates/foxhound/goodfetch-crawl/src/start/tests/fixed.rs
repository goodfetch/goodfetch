//! The settings fixed for a Run's life (ADRs 0102 and 0120), one case per
//! key of the settings schema's list, through the engine's public
//! interface over the in-memory network (spec #213's Seam 3): a start
//! under the Run's ID or a resume that changes one is refused with
//! `setting_not_applicable`, whose `fix` names a new Run.

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_protocol::entries::MODULES;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Reply, answer_all, crawl, dependencies, seeds};

/// The cases below are the settings schema's list, so a key marked fixed
/// for a Run's life has a case once it is marked.
///
/// Covers: story:3-172
#[test]
fn the_cases_are_every_key_the_settings_schema_fixes_for_a_runs_life() {
    let mut fixed: Vec<&str> = MODULES
        .into_iter()
        .flat_map(|entries| entries.settings.iter())
        .filter(|setting| setting.fixed_for_run)
        .map(|setting| setting.name)
        .collect();
    fixed.sort_unstable();

    assert_eq!(
        fixed,
        [
            "crawl.keep_url_fragment",
            "crawl.scope.exclude",
            "crawl.scope.include",
            "crawl.scope.strategy",
        ]
    );
}

/// What a refusal says, or the start's `started_as` or the resume's
/// `resumed`.
fn outcome(answer: &Value) -> Value {
    answer.get("error").map_or_else(
        || answer["result"]["started_as"].clone(),
        |error| {
            json!([
                error["data"]["code"],
                error["data"]["key"],
                error["data"]["fix"]
            ])
        },
    )
}

/// A start under a Run's ID that changes a key fixed for its life is
/// refused, whatever the Run's state, whether the Run began with the key's
/// default and the start sets it, or began with another value and the
/// start leaves it out.
///
/// Covers: story:3-172, error:setting_not_applicable, operation:crawl.start, setting:crawl.scope.strategy, setting:crawl.scope.include, setting:crawl.scope.exclude, setting:crawl.keep_url_fragment
#[rstest::rstest]
#[case::strategy("crawl.scope.strategy", json!({ "crawl": { "scope": { "strategy": "same_domain" } } }))]
#[case::include("crawl.scope.include", json!({ "crawl": { "scope": { "include": ["https://shop.test/page/**"] } } }))]
#[case::exclude("crawl.scope.exclude", json!({ "crawl": { "scope": { "exclude": ["https://shop.test/private/**"] } } }))]
#[case::keep_url_fragment(
    "crawl.keep_url_fragment",
    // Fragments are kept only on a tier that serves them (ADR 0120).
    json!({ "crawl": { "keep_url_fragment": true }, "fetch": { "tier": "auto" } })
)]
fn a_start_under_a_runs_id_that_changes_a_key_fixed_for_its_life_is_refused(
    #[case] key: &str,
    #[case] changed: Value,
    #[values(false, true)] began_changed: bool,
) {
    let env = TestEnv::new();
    let start = |settings: &Value| json!({ "run_id": "fixed", "seeds": seeds(1), "labels": { "page": {} }, "settings": settings });
    let (began, then) = if began_changed {
        (changed, json!({}))
    } else {
        (json!({}), changed)
    };

    let answers = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let mut answers = Vec::new();
            for settings in [&began, &then] {
                let answer = client
                    .call("crawl.start", start(settings), &mut |_| Reply::Silent)
                    .await;
                answers.push(outcome(&answer));
            }
            answers
        },
    );

    assert_eq!(
        answers,
        [
            json!("new"),
            json!(["setting_not_applicable", key, "a new Run"]),
        ]
    );
}

/// A resume of a paused crawl that changes a key fixed for its life is
/// refused, and one that gives none of them resumes it: a key a resume
/// leaves out is no change.
///
/// Covers: story:3-172, error:setting_not_applicable, operation:run.resume, setting:crawl.scope.strategy, setting:crawl.scope.include, setting:crawl.scope.exclude, setting:crawl.keep_url_fragment
#[rstest::rstest]
#[case::strategy("crawl.scope.strategy", json!({ "crawl": { "scope": { "strategy": "same_domain" } } }))]
#[case::include("crawl.scope.include", json!({ "crawl": { "scope": { "include": ["https://shop.test/page/**"] } } }))]
#[case::exclude("crawl.scope.exclude", json!({ "crawl": { "scope": { "exclude": ["https://shop.test/admin/**"] } } }))]
#[case::keep_url_fragment(
    "crawl.keep_url_fragment",
    json!({ "crawl": { "keep_url_fragment": true }, "fetch": { "tier": "auto" } })
)]
fn a_resume_that_changes_a_key_fixed_for_the_runs_life_is_refused(
    #[case] key: &str,
    #[case] changed: Value,
) {
    let env = TestEnv::new();
    let resume = |settings: Value| json!({ "run_id": "paused", "settings": settings });

    let answers = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "paused",
                        "seeds": seeds(1),
                        "labels": { "page": {} },
                        "settings": { "crawl": { "scope": { "exclude": ["https://shop.test/private/**"] } } },
                    }),
                    &mut answer_all,
                )
                .await;
            client
                .call("run.pause", json!({ "run_id": "paused" }), &mut answer_all)
                .await;
            let mut answers = Vec::new();
            for settings in [changed, json!({})] {
                let answer = client
                    .call("run.resume", resume(settings), &mut answer_all)
                    .await;
                answers.push(outcome(&answer));
            }
            answers
        },
    );

    assert_eq!(
        answers,
        [
            json!(["setting_not_applicable", key, "a new Run"]),
            json!("resumed"),
        ]
    );
}
