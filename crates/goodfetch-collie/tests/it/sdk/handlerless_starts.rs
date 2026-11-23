//! The handler-less starts through the Rust SDK (ADRs 0036 and 0102), the
//! `handlerless-starts` client conformance case in `linked` mode: `Tags`
//! refuses tags past their bounds before anything is sent, and a Recipe's
//! run and a code-free catalog run with tags at their bounds reach the
//! linked engine, which answers `run_target_unbuilt` naming the start and
//! its target's kind until it runs them; a view opens by a Run's ID with no
//! round trip until the connection closes. The client crate's Daemon test
//! plays every start option, the resumed and ended Runs and their log lines.

use goodfetch_collie::{CatalogTarget, Collie, ErrorCode, RunId, Tags};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::run::{RunTargetUnbuiltDetails, TagsInvalid};
use goodfetch_test_support::{TestEnv, case_logs, real_time_runtime};

use super::options;

/// Covers: story:11-131
#[test]
fn a_handlerless_start_refuses_tags_past_their_bounds_and_reaches_the_linked_engine() {
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "handlerless-starts",
        RustMode::Linked,
    )
    .unwrap();
    let env = TestEnv::new();
    let too_many = Tags::parse(vec!["t"; 17]);
    let too_long = Tags::parse(["t".repeat(65)]);
    let bounds = || Tags::parse(vec!["t".repeat(64); 16]).unwrap();
    let detached = RunId::parse("checkout-replay").unwrap();
    let (refused, viewed, after) = real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let recipe = collie
            .start_recipe("shop-prices")
            .tags(bounds())
            .await
            .unwrap_err();
        let replay = collie
            .start_catalog_run(CatalogTarget::PlanReplay {
                plan: "plans/checkout.json".to_owned(),
            })
            .tags(bounds())
            .await
            .unwrap_err();
        let view = collie.open_run(&detached).unwrap();
        let viewed = view.id().clone();
        view.close();
        collie.close().await.unwrap();
        let after = collie
            .open_run(&detached)
            .map(|_| ())
            .map_err(|error| error.code());
        ([recipe, replay], viewed, after)
    });

    case.check(
        "tags past their bounds are refused before anything is sent",
        || {
            assert_eq!(
                (too_many.unwrap_err(), too_long.unwrap_err()),
                (TagsInvalid::TooMany, TagsInvalid::TooLong)
            );
        },
    );
    case.check(
        "each start reaches the engine, which runs no handler-less target yet",
        || {
            let answered = refused.each_ref().map(|error| {
                let details = error.details::<RunTargetUnbuiltDetails>().unwrap();
                (error.code(), details.operation, details.target)
            });
            assert_eq!(
                answered,
                [
                    (
                        ErrorCode::RUN_TARGET_UNBUILT,
                        "recipe.start".to_owned(),
                        "recipe".to_owned()
                    ),
                    (
                        ErrorCode::RUN_TARGET_UNBUILT,
                        "catalog_run.start".to_owned(),
                        "plan_replay".to_owned()
                    ),
                ]
            );
        },
    );
    case.check(
        "open_run opens a view by its Run ID with no round trip, until the connection closes",
        || assert_eq!((viewed, after), (detached, Err(ErrorCode::CLOSED))),
    );
}
