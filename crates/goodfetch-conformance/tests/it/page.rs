//! The page suite (spec #577, tickets #695, #698 and #699) against an
//! Attached engine of Collie's executable, given by path, on the local
//! Proving ground (ADR 0094): its browser from a Browser cache in the test's own folder, which
//! its environment names (ADR 0059).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_conformance::{
    EngineMode, EngineTarget, ProtocolSuite, ProvingGround, run_protocol_suite,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::result_document::{CapabilityResult, CaseOutcome, Outcome};
use goodfetch_test_support::{BROWSERS_PATH_VARIABLE, TestEnv, Tool, browser_cache, tools};

use crate::proving_ground::LocalCopy;

/// Covers: story:5-61, story:5-23, story:5-24, story:5-26, story:5-29, story:5-30, story:5-31, operation:run.new_page, operation:page.goto, operation:page.reload, operation:page.go_back, operation:page.go_forward, error:navigation_failed, error:navigation_interrupted, error:too_many_redirects, error:response_truncated, event:page.download
#[test]
fn the_page_suite_is_proved_against_collies_attached_engine() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed());
    let proving_ground = ProvingGround::local(copy.address());
    let mut environment = env.env().clone();
    environment.insert(
        BROWSERS_PATH_VARIABLE.to_owned(),
        browser_cache(&env).unwrap().display().to_string(),
    );
    let target = EngineTarget {
        executable: tools().unwrap().path(Tool::Collie).unwrap().to_path_buf(),
        mode: EngineMode::Attached,
        environment,
        folder: env.folder().to_path_buf(),
    };

    let document = run_protocol_suite(&ProtocolSuite::page(), &target, &proving_ground).unwrap();

    let cases: Vec<(&str, CaseOutcome)> = document
        .capabilities()
        .iter()
        .flat_map(CapabilityResult::cases)
        .map(|case| (case.facts().id.as_str(), case.outcome()))
        .collect();
    assert_eq!(
        (
            document
                .capabilities()
                .iter()
                .map(|capability| (capability.name(), capability.outcome()))
                .collect::<Vec<_>>(),
            document.header().target.product,
        ),
        (
            vec![
                ("sessions.default", Outcome::Proved),
                ("navigation", Outcome::Proved),
                ("navigation.back_forward_cache", Outcome::Proved),
                ("navigation.failures", Outcome::Proved),
                ("navigation.redirects", Outcome::Proved),
                ("navigation.cut_documents", Outcome::Proved),
                ("navigation.decoding", Outcome::Proved),
                ("navigation.downloads", Outcome::Proved),
            ],
            Some(Product::Collie),
        ),
        "{cases:?}"
    );
}
