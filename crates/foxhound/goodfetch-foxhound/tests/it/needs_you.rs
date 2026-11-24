//! A kind of "Needs you" items Collie does not know, answered on Foxhound's
//! executable (spec #1937; ADR 0087): its source, its operation and its
//! prompt are the executable's own, as its `main` passes them, and
//! `needs-you answer` takes a person through it as through Collie's kinds.
//! The toy kind stands for a Foxhound kind that waits for a person, which
//! Foxhound has none of yet.

use std::process::ExitCode;
use std::sync::LazyLock;
use std::time::SystemTime;

use goodfetch_collie::executable::{
    Confirmation, ItemAnswer, ItemPrompt, OwnCommands, Start, main as start,
};
use goodfetch_engine::needs_you::{
    Answering, Changes, NeedsYouSource, Primitives, SourceContext, SourceError,
};
use goodfetch_engine::{CallContext, CallError, Composition, Contribution, Handler};
use goodfetch_foxhound::executable::{COMMANDS, SKILLS};
use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach, Stability,
    schema_of,
};
use goodfetch_protocol::needs_you::NeedsYouItem;
use goodfetch_test_support::{
    SEED_VARIABLE, SUITE_VARIABLE, TestEnv, Variables, current_test, pty,
};
use serde_json::{Map, Value, json};
use tokio::sync::watch;

/// The toy kind, which only Foxhound's executable knows.
const TOY: &str = "toy_question";

/// Whether the toy's question is settled, in the process that runs the
/// executable and its Attached engine.
static SETTLED: LazyLock<watch::Sender<bool>> = LazyLock::new(|| watch::Sender::new(false));

/// The toy's answer, a person's.
const TOY_SETTLE: Operation = Operation {
    name: "toy.settle",
    description: "Settles the toy's question.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: PROTOCOL_VERSION,
};

/// The toy's prompt, among Foxhound's own.
const ANSWERS: &[ItemPrompt] = &[ItemPrompt {
    kind: TOY,
    answers: &[ItemAnswer {
        label: "Settle it",
        operation: &TOY_SETTLE,
        input: None,
        confirmation: |item, _| {
            Ok(Confirmation {
                question: format!("Settle {}?", item.id),
                warning: Some("it settles for good".to_owned()),
                params: json!({ "item": item.id }),
                then: None,
            })
        },
    }],
    outside: None,
}];

/// The toy's source: its one question, until it is settled.
struct Toy;

impl Primitives for Toy {
    fn dismissable(&self) -> bool {
        false
    }

    fn items<'a>(
        &'a self,
        _: &'a SourceContext,
    ) -> Answering<'a, Result<Vec<NeedsYouItem>, SourceError>> {
        let waiting = !*SETTLED.borrow();
        Box::pin(async move {
            Ok(waiting
                .then(|| NeedsYouItem {
                    id: "toy:1".to_owned(),
                    kind: TOY.to_owned(),
                    since: SystemTime::UNIX_EPOCH,
                    run: None,
                    schedule: None,
                    summary: "The toy asks a question.".to_owned(),
                    actions: vec![TOY_SETTLE.name.to_owned()],
                    details: Map::new(),
                })
                .into_iter()
                .collect())
        })
    }

    fn changes(&self, _: &SourceContext) -> Changes {
        Changes::of(SETTLED.subscribe())
    }
}

/// Foxhound's composition, with the toy's contribution.
fn with_the_toy() -> Composition {
    goodfetch_foxhound::composition().with(
        Contribution::new("toy")
            .with_handler(Handler::new(
                &TOY_SETTLE,
                |params: Value, _: CallContext| async move {
                    assert_eq!(params, json!({ "item": "toy:1" }));
                    SETTLED.send_replace(true);
                    Ok::<Value, CallError>(json!({ "settled": true }))
                },
            ))
            .with_needs_you_source(NeedsYouSource::new(Toy)),
    )
}

/// The test's setup with no exercise record: the toy's operation is no
/// Feature's.
fn unrecorded() -> TestEnv {
    let process = Variables::from_process();
    let variables: Variables = [SEED_VARIABLE, SUITE_VARIABLE]
        .into_iter()
        .filter_map(|name| process.get(name).map(|value| (name, value.to_owned())))
        .collect();
    TestEnv::create(&current_test(), &variables).unwrap()
}

/// Covers: story:12-139, story:12-23
#[test]
fn a_kind_of_foxhounds_is_answered_on_its_executable() {
    let env = unrecorded();
    env.user_configuration("storage.low_free = 0\nstorage.critical_free = 0\n");

    let mut session = pty(&env, std::env::current_exe().unwrap())
        .args([
            "--exact",
            "needs_you::foxhounds_executable_with_the_toy",
            "--ignored",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .spawn()
        .unwrap();
    session.expect("1. The toy asks a question.").unwrap();
    session.expect("Answers: Settle it.").unwrap();
    session
        .expect("Which item? Its number, or Enter to end:")
        .unwrap();
    session.send_line("1").unwrap();
    session.expect("it settles for good").unwrap();
    session.expect("Settle toy:1? [y/N]").unwrap();
    session.send_line("y").unwrap();
    session
        .expect("Left the list: 1. The toy asks a question. (resolved)")
        .unwrap();
    session.expect("Nothing waits for a person.").unwrap();
    let ended = session.finish().unwrap();

    assert_eq!(ended.exit, Some(0), "{}", ended.stripped());
}

/// Foxhound's executable as its `main` starts it, with the toy.
#[test]
#[ignore = "run as a child by needs_you::a_kind_of_foxhounds_is_answered_on_its_executable"]
#[expect(
    clippy::disallowed_methods,
    reason = "the child's main is its test: it reads the environment its parent gave it"
)]
fn foxhounds_executable_with_the_toy() {
    let exit = start(&Start {
        product: Product::Foxhound,
        version: env!("CARGO_PKG_VERSION"),
        release_date: None,
        composition: with_the_toy,
        commands: OwnCommands {
            answers: ANSWERS,
            ..COMMANDS
        },
        skills: SKILLS,
        arguments: ["goodfetch-foxhound", "needs-you", "answer", "--attached"]
            .map(Into::into)
            .into(),
        environment: std::env::vars_os().collect(),
    });

    assert_eq!(exit, ExitCode::SUCCESS);
}
