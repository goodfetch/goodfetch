use std::cell::RefCell;

use proptest::prelude::*;
use proptest_state_machine::ReferenceStateMachine;
use rstest::rstest;
use serde_json::json;

use super::{
    FAULT_PLAN_FILE, FAULT_PLAN_SOURCE_FILE, FaultPlan, FaultPlanAction, FaultPlanError,
    FaultPlanFailure, FaultPlanGenerator, FaultPlanStep,
};
use crate::env::TestEnv;

/// The committed schema `xtask codegen` writes from [`FaultPlan`].
const SCHEMA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-support/fixtures/schemas/fault-plan.schema.json"
);

/// Two hosts, `a` and `b`, each step on one of them at or after the one
/// before it.
struct TwoHosts;

fn action(peer: &'static str) -> BoxedStrategy<FaultPlanAction> {
    let peer = || peer.to_owned();
    prop_oneof![
        Just(FaultPlanAction::Crash),
        Just(FaultPlanAction::Restart),
        Just(FaultPlanAction::Partition { peer: peer() }),
        Just(FaultPlanAction::Heal { peer: peer() }),
        (0..500_u64).prop_map(move |milliseconds| FaultPlanAction::Latency {
            peer: peer(),
            milliseconds
        }),
        Just(FaultPlanAction::Drop { peer: peer() }),
        (-60_000..60_000_i64).prop_map(|milliseconds| FaultPlanAction::JumpWall { milliseconds }),
        Just(FaultPlanAction::Operation(json!({ "method": "run.start" }))),
    ]
    .boxed()
}

impl ReferenceStateMachine for TwoHosts {
    /// The simulated moment of the last step.
    type State = u64;
    type Transition = FaultPlanStep;

    fn init_state() -> BoxedStrategy<u64> {
        Just(0).boxed()
    }

    fn transitions(now: &u64) -> BoxedStrategy<FaultPlanStep> {
        let now = *now;
        let on = |host: &'static str, peer| {
            (0..1_000_u64, action(peer)).prop_map(move |(after, action)| FaultPlanStep {
                at: now + after,
                host: host.to_owned(),
                action,
            })
        };
        prop_oneof![on("a", "b"), on("b", "a")].boxed()
    }

    fn apply(_now: u64, step: &FaultPlanStep) -> u64 {
        step.at
    }

    fn preconditions(now: &u64, step: &FaultPlanStep) -> bool {
        step.at >= *now
    }
}

/// The bug the scenario finds: host `a` crashes while partitioned from
/// `b`.
fn crashes_while_partitioned(plan: &FaultPlan) -> Result<(), String> {
    let mut partitioned = false;
    for step in plan.steps.iter().filter(|step| step.host == "a") {
        match &step.action {
            FaultPlanAction::Partition { peer } if peer == "b" => partitioned = true,
            FaultPlanAction::Heal { peer } if peer == "b" => partitioned = false,
            FaultPlanAction::Crash if partitioned => {
                return Err(format!("a crashed at {} while partitioned from b", step.at));
            }
            _ => {}
        }
    }
    Ok(())
}

#[test]
fn a_failing_plan_shrinks_to_a_smaller_one_written_with_its_replay_command() {
    let env = TestEnv::new();
    let first_failing = RefCell::new(None);

    let failure = FaultPlanGenerator::new(&env, "group_formation", 3)
        .check::<TwoHosts, _>(10..30, |plan| {
            let outcome = crashes_while_partitioned(plan);
            if outcome.is_err() && first_failing.borrow().is_none() {
                first_failing.replace(Some(plan.clone()));
            }
            outcome
        })
        .expect_err("a generated plan crashes a while partitioned");

    let FaultPlanFailure::Shrunk {
        plan,
        file,
        command,
        ..
    } = failure
    else {
        panic!("the shrunk plan is not written: {failure}");
    };
    let first_failing = first_failing.into_inner().expect("a plan failed");
    assert!(
        plan.steps.len() < first_failing.steps.len(),
        "{plan:?} is no smaller than {first_failing:?}"
    );
    let shrunk: Vec<(&str, &FaultPlanAction)> = plan
        .steps
        .iter()
        .map(|step| (step.host.as_str(), &step.action))
        .collect();
    let partition = FaultPlanAction::Partition {
        peer: "b".to_owned(),
    };
    assert_eq!(
        shrunk,
        vec![("a", &partition), ("a", &FaultPlanAction::Crash)]
    );
    assert!(crashes_while_partitioned(&plan).is_err());
    assert_eq!((plan.seed, plan.generator_version), (env.seed(), 3));
    assert_eq!(file, env.folder().join(FAULT_PLAN_FILE));
    assert_eq!(FaultPlan::read(&file).unwrap(), *plan);
    assert_valid(&file);
    assert_eq!(
        command,
        format!("just simulate group_formation --plan '{}'", file.display())
    );
    let source: toml::Table = toml::from_str(
        &std::fs::read_to_string(env.folder().join(FAULT_PLAN_SOURCE_FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(source["command"].as_str(), Some(command.as_str()));
    assert_eq!(source["version"].as_integer(), Some(3));
}

/// A committed plan on which the scenario fails: `a` partitioned from `b`,
/// then crashed.
fn committed_plan() -> FaultPlan {
    FaultPlan {
        seed: 7,
        generator_version: 3,
        steps: vec![
            FaultPlanStep {
                at: 10,
                host: "a".to_owned(),
                action: FaultPlanAction::Partition {
                    peer: "b".to_owned(),
                },
            },
            FaultPlanStep {
                at: 20,
                host: "a".to_owned(),
                action: FaultPlanAction::Crash,
            },
        ],
    }
}

#[test]
fn a_plan_to_replay_runs_alone_and_its_failure_names_its_file() {
    let env = TestEnv::new();
    let file = env.folder().join("crash_while_partitioned.json");
    committed_plan().write(&file).unwrap();
    let runs = RefCell::new(Vec::new());

    let failure = FaultPlanGenerator::new(&env, "group_formation", 3)
        .replaying(&file)
        .check::<TwoHosts, _>(10..30, |plan| {
            runs.borrow_mut().push(plan.clone());
            crashes_while_partitioned(plan)
        })
        .expect_err("the committed plan crashes a while partitioned");

    assert_eq!(runs.into_inner(), vec![committed_plan()]);
    assert_eq!(
        failure.to_string(),
        format!(
            "the scenario group_formation fails on the Fault plan {}: a crashed at 20 while partitioned from b",
            file.display()
        )
    );
    assert!(
        !env.folder().join(FAULT_PLAN_FILE).exists(),
        "a replayed plan is already written"
    );
}

#[test]
fn a_plan_to_replay_that_passes_passes_and_one_unreadable_fails() {
    let env = TestEnv::new();
    let file = env.folder().join("healed.json");
    let mut healed = committed_plan();
    healed.steps[1].action = FaultPlanAction::Heal {
        peer: "b".to_owned(),
    };
    healed.write(&file).unwrap();
    let generator = FaultPlanGenerator::new(&env, "group_formation", 3);

    let passed = generator
        .replaying(&file)
        .check::<TwoHosts, _>(10..30, crashes_while_partitioned);
    let missing = FaultPlanGenerator::new(&env, "group_formation", 3)
        .replaying(&env.folder().join("missing.json"))
        .check::<TwoHosts, _>(10..30, crashes_while_partitioned);

    assert!(passed.is_ok(), "{passed:?}");
    assert!(
        matches!(
            missing,
            Err(FaultPlanFailure::Unreadable {
                error: FaultPlanError::Unreadable { .. },
                ..
            })
        ),
        "{missing:?}"
    );
}

#[test]
fn a_scenario_no_plan_fails_writes_no_plan() {
    let env = TestEnv::new();

    let outcome = FaultPlanGenerator::new(&env, "group_formation", 1)
        .check::<TwoHosts, String>(1..10, |_| Ok(()));

    assert!(outcome.is_ok(), "{outcome:?}");
    assert!(!env.folder().join(FAULT_PLAN_FILE).exists());
}

#[test]
fn a_plan_with_every_action_is_valid_and_reads_back_the_same() {
    let env = TestEnv::new();
    let actions = [
        FaultPlanAction::Crash,
        FaultPlanAction::Restart,
        FaultPlanAction::Partition {
            peer: "b".to_owned(),
        },
        FaultPlanAction::Heal {
            peer: "b".to_owned(),
        },
        FaultPlanAction::Latency {
            peer: "b".to_owned(),
            milliseconds: 250,
        },
        FaultPlanAction::Drop {
            peer: "b".to_owned(),
        },
        FaultPlanAction::JumpWall {
            milliseconds: -3_600_000,
        },
        FaultPlanAction::Operation(json!({ "method": "run.start", "params": { "run": "r1" } })),
    ];
    let plan = FaultPlan {
        seed: 7,
        generator_version: 1,
        steps: actions
            .into_iter()
            .zip(0..)
            .map(|(action, at)| FaultPlanStep {
                at,
                host: "a".to_owned(),
                action,
            })
            .collect(),
    };
    let file = env.folder().join(FAULT_PLAN_FILE);

    plan.write(&file).unwrap();

    assert_valid(&file);
    assert_eq!(FaultPlan::read(&file).unwrap(), plan);
}

#[rstest]
#[case::unknown_action(json!({ "seed": 1, "generator_version": 1, "steps": [
    { "at": 0, "host": "a", "action": "explode" }
] }))]
#[case::unknown_field_of_a_step(json!({ "seed": 1, "generator_version": 1, "steps": [
    { "at": 0, "host": "a", "action": "crash", "peer": "b" }
] }))]
#[case::unknown_field_of_a_fault(json!({ "seed": 1, "generator_version": 1, "steps": [
    { "at": 0, "host": "a", "action": { "partition": { "peer": "b", "loss": 0.5 } } }
] }))]
#[case::unknown_field_of_the_plan(json!({ "seed": 1, "generator_version": 1, "steps": [],
    "scenario": "group_formation" }))]
fn a_plan_with_an_unknown_action_or_field_is_refused(#[case] plan: serde_json::Value) {
    let env = TestEnv::new();
    let file = env.folder().join(FAULT_PLAN_FILE);
    std::fs::write(&file, plan.to_string()).unwrap();

    let error = FaultPlan::read(&file).unwrap_err();

    assert!(matches!(error, FaultPlanError::Invalid { .. }), "{error}");
}

fn assert_valid(file: &std::path::Path) {
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(SCHEMA).unwrap()).unwrap();
    let instance: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(&instance)
        .map(|error| error.to_string())
        .collect();
    assert_eq!(errors, Vec::<String>::new());
}
