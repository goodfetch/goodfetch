//! The self-test suite against the local copy of the Proving ground: it
//! proves the oracle against reference results read from the pages, and
//! known-wrong results made from them, and fails, naming the case, when a
//! reference result is itself wrong.

use goodfetch_conformance::{ProvingGround, References, run_self_test};
use goodfetch_protocol::journey::TaskId;
use goodfetch_protocol::result_document::{
    CaseOutcome, CaseResult, ConformanceDocument, LoadedDocument, OracleOutcome, Outcome,
    ResultDocument, TargetIdentity, read,
};
use goodfetch_test_support::{ProvingGround as LocalCopy, TestEnv};

use crate::schema_violations;

fn task(id: &str) -> TaskId {
    TaskId::try_from(id.to_owned()).unwrap()
}

/// Each capability's name and outcome.
fn outcomes(document: &ConformanceDocument) -> Vec<(&str, Outcome)> {
    document
        .capabilities()
        .iter()
        .map(|capability| (capability.name(), capability.outcome()))
        .collect()
}

/// Every case of the document whose outcome is `outcome`.
fn cases_with(document: &ConformanceDocument, outcome: CaseOutcome) -> Vec<&CaseResult> {
    document
        .capabilities()
        .iter()
        .flat_map(goodfetch_protocol::result_document::CapabilityResult::cases)
        .filter(|case| case.outcome() == outcome)
        .collect()
}

/// Covers: story:2-46, story:2-50
#[test]
fn the_self_test_proves_every_capability_on_the_reference_results() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let references = References::read(&proving_ground).unwrap();

    let document = run_self_test(&proving_ground, &references).unwrap();

    assert_eq!(
        outcomes(&document),
        [
            ("catalog.verified", Outcome::Proved),
            ("catalog.incomplete", Outcome::Proved),
            ("catalog.false_success", Outcome::Proved),
            ("catalog.failed", Outcome::Proved),
        ]
    );
}

/// Covers: story:2-57
#[test]
fn the_self_test_document_names_the_proving_ground_it_judged_and_matches_its_schema() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let references = References::read(&proving_ground).unwrap();
    let version = proving_ground.task_list().unwrap().proving_ground.version;

    let document = run_self_test(&proving_ground, &references).unwrap();

    let header = document.header();
    assert_eq!(
        (
            header.suite.name.as_str(),
            header.target.name.as_str(),
            &header.target.identity,
            header.proving_ground.seed,
            header.online,
        ),
        (
            "self-test",
            "goodfetch-proving-ground",
            &TargetIdentity::Version(version.clone()),
            env.seed(),
            false,
        )
    );
    assert_eq!(header.proving_ground.version, version);
    let written = ResultDocument::Conformance(document.clone());
    assert_eq!(schema_violations(&written), Vec::<String>::new());
    assert_eq!(
        read(&serde_json::to_vec(&written).unwrap()).unwrap(),
        LoadedDocument::Conformance(document)
    );
}

/// Covers: story:2-50, story:2-57
#[test]
fn a_wrong_reference_result_fails_the_self_test_naming_the_case_and_the_oracles_answer() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let mut references = References::read(&proving_ground).unwrap();
    let record = &mut references.records.get_mut(&task("catalog/2")).unwrap()[0];
    record.price = if record.price == "0.01" {
        "0.02"
    } else {
        "0.01"
    }
    .to_owned();

    let document = run_self_test(&proving_ground, &references).unwrap();

    assert_eq!(document.capabilities()[0].outcome(), Outcome::Failed);
    let failed = cases_with(&document, CaseOutcome::Failed);
    let named: Vec<(&str, OracleOutcome)> = failed
        .iter()
        .map(|case| {
            (
                case.facts().id.as_str(),
                case.facts().entries[0].evidence.oracle_answers[0].outcome,
            )
        })
        .collect();
    assert_eq!(
        named,
        [("catalog-extract-reference", OracleOutcome::FalseSuccess)]
    );
}

/// Covers: story:2-56
#[test]
fn a_task_with_no_reference_result_leaves_its_capabilities_unproved() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let mut references = References::read(&proving_ground).unwrap();
    references.records.remove(&task("catalog/2"));

    let document = run_self_test(&proving_ground, &references).unwrap();

    assert_eq!(
        outcomes(&document),
        [
            ("catalog.verified", Outcome::Unproved),
            ("catalog.incomplete", Outcome::Proved),
            ("catalog.false_success", Outcome::Unproved),
            ("catalog.failed", Outcome::Proved),
        ]
    );
}
