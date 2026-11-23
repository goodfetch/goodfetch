//! Declared Effects through the engine's public interface over the
//! in-memory network (spec #213's Seam 3; ADRs 0106 and 0138):
//! `effect.open` answers `perform` with an entry and an idempotency key,
//! then the journal's answer on a restart once `effect.close` kept the
//! output; a key out of shape, or declared again by its owner, is refused;
//! a body is judged by whether something was sent for it, so one that
//! failed after a send leaves its Effect uncertain, never run again and
//! raised as `effect_uncertain` until a person answers, and the Run report
//! lists it; and a power cut before the sent mark runs the body again,
//! between the mark and the close leaves it uncertain, and after the close
//! answers the journal. The window that marks what is sent, and a person's
//! answer, are the test's operations on the engine's shared [`Effects`].

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach, Stability,
    schema_of,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, BufReader};

use super::consent_walls::audit_entries;
use super::run::call;
use super::{Client, Dependencies, attached_with, dependencies, hello, version};
use crate::call_error::CallError;
use crate::composition::{CallContext, Composition, Contribution, Handler};
use crate::effect::{Effects, Reconciled, Reconciliation};
use crate::engine::Engine;
use crate::product::{Channel, Identity};
use crate::run_state::RunStateOptions;
use crate::runs;
use crate::transport::{self, Network};

/// The window's mark: a command that may change a page or a site goes out
/// for the Run's bodies outside any attempt.
const SEND: Operation = Operation {
    name: "test.effect_send",
    description: "Marks the Run's running bodies sent, then answers.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
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

/// A person's answer about an uncertain Effect.
const RECONCILE: Operation = Operation {
    name: "test.effect_reconcile",
    description: "Gives a person's answer about an uncertain Effect.",
    ..SEND
};

fn failed(error: impl std::fmt::Display) -> CallError {
    CallError::protocol(ProtocolFailure::InternalError, error.to_string())
}

/// The test's operations on `effects`.
fn window(effects: &Effects) -> Contribution {
    let sending = effects.clone();
    let reconciling = effects.clone();
    Contribution::new("test-effect")
        .with_handler(Handler::new(
            &SEND,
            move |params: Value, context: CallContext| {
                let effects = sending.clone();
                async move {
                    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
                    let run = context.runs().live(run_id).await.map_err(failed)?;
                    effects
                        .sending(
                            (runs::identity(&run), runs::state(&run)),
                            None,
                            params["command"].as_str().unwrap(),
                        )
                        .await
                        .map_err(failed)?;
                    Ok::<Value, CallError>(json!({ "marked": true }))
                }
            },
        ))
        .with_handler(Handler::new(
            &RECONCILE,
            move |params: Value, context: CallContext| {
                let effects = reconciling.clone();
                async move {
                    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
                    let run = context.runs().live(run_id).await.map_err(failed)?;
                    let answer = match params["answer"].as_str().unwrap() {
                        "happened" => Reconciliation::Happened,
                        "did_not_happen" => Reconciliation::DidNotHappen,
                        _ => Reconciliation::LeaveUncertain,
                    };
                    let reconciled = effects
                        .reconcile(
                            (runs::identity(&run), runs::state(&run)),
                            params["entry"].as_str().unwrap(),
                            answer,
                            &context,
                        )
                        .await?;
                    Ok::<Value, CallError>(match reconciled {
                        Reconciled::Answered(effect) => json!({ "answered": effect }),
                        Reconciled::NotUncertain => json!({ "answered": null }),
                    })
                }
            },
        ))
}

/// A Collie engine with the engine's own operations, sharing its declared
/// Effects with the test's.
fn collie() -> Engine {
    let effects = Effects::default();
    let identity = Identity {
        product: Product::Collie,
        product_version: version(),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default()
        .with(crate::info::contribution_sharing(&effects))
        .with(window(&effects));
    Engine::new(identity, &composition).unwrap()
}

/// `env`'s dependencies, its run files on `vfs` when one is given.
fn on(env: &TestEnv, vfs: Option<&CrashVfs>) -> Dependencies {
    Dependencies {
        run_state: RunStateOptions {
            vfs: vfs.map(|vfs| vfs.name().to_owned()),
            ..RunStateOptions::default()
        },
        ..dependencies(env)
    }
}

/// The Client's calls, numbered from `first`.
struct Calls {
    client: Client,
    next: i64,
}

impl Calls {
    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        call(&mut self.client, self.next, method, params).await
    }

    async fn open(&mut self, run_id: &str, key: &str) -> Value {
        self.call("effect.open", json!({ "run_id": run_id, "key": key }))
            .await
    }

    async fn close(&mut self, run_id: &str, opened: &Value, outcome: Value) -> Value {
        let entry = opened["result"]["entry"].clone();
        self.call(
            "effect.close",
            json!({ "run_id": run_id, "entry": entry, "outcome": outcome }),
        )
        .await
    }

    async fn send(&mut self, run_id: &str, command: &str) -> Value {
        let marked = self
            .call(
                "test.effect_send",
                json!({ "run_id": run_id, "command": command }),
            )
            .await;
        assert_eq!(marked["result"], json!({ "marked": true }), "{marked}");
        marked
    }
}

/// Opens a Connection with `hello`, then `first` starts or resumes the Run
/// `run_id` with `params` added.
async fn begun(mut client: Client, first: &str, run_id: &str, params: Value) -> Calls {
    client.send(&[hello("collie", "0.1")]).await;
    client.next().await.unwrap();
    let mut calls = Calls { client, next: 0 };
    let mut start = json!({ "run_id": run_id });
    if let (Some(start), Some(more)) = (start.as_object_mut(), params.as_object()) {
        start.extend(more.clone());
    }
    let reply = calls.call(first, start).await;
    assert!(reply.get("result").is_some(), "{reply}");
    calls
}

/// Has an engine on the `Dependencies` that `dependencies` makes on its
/// runtime start the Run `run_id` while `talk` talks to it, then crashes
/// `vfs` as `crash` says, if given, and kills the engine: its runtime is
/// dropped with the Run live.
fn killed<D, F, Fut, T>(
    dependencies: D,
    run_id: &str,
    crash: Option<(&CrashVfs, CrashKind)>,
    talk: F,
) -> T
where
    D: FnOnce() -> Dependencies,
    F: FnOnce(Calls) -> Fut,
    Fut: Future<Output = T>,
{
    let runtime = goodfetch_test_support::runtime();
    let talked = runtime.block_on(async {
        let (network, dialer) = transport::in_memory();
        let engine = collie();
        let serving = engine.run_attached(Network::InMemory(network), dependencies());
        let talking = async {
            let (reader, writer) = dialer.connect().await.unwrap().split();
            let client = Client {
                lines: BufReader::new(reader).lines(),
                writer,
            };
            talk(begun(client, "run.start", run_id, json!({})).await).await
        };
        tokio::select! {
            served = serving => panic!("the engine ended: {served:?}"),
            talked = talking => talked,
        }
    });
    if let Some((vfs, kind)) = crash {
        vfs.crash(kind);
    }
    drop(runtime);
    talked
}

/// Resumes the Run `run_id` on a new engine on the `Dependencies` that
/// `dependencies` makes, which then declares `key` again.
fn declared_again(dependencies: impl FnOnce() -> Dependencies, run_id: &str, key: &str) -> Value {
    let (run_id, key) = (run_id.to_owned(), key.to_owned());
    attached_with(&collie(), dependencies, |client| async move {
        let mut calls = begun(client, "run.resume", &run_id, json!({})).await;
        calls.open(&run_id, &key).await
    })
}

/// Runs a new engine in `env` while `talk` talks to it, after its `hello`
/// and `run.start` of `run_id` with `params` added.
fn started<F, Fut, T>(env: &TestEnv, run_id: &str, params: Value, talk: F) -> T
where
    F: FnOnce(Calls) -> Fut,
    Fut: Future<Output = T>,
{
    let run_id = run_id.to_owned();
    attached_with(
        &collie(),
        || dependencies(env),
        |client| async move { talk(begun(client, "run.start", &run_id, params).await).await },
    )
}

fn code(reply: &Value) -> &Value {
    &reply["error"]["data"]["code"]
}

/// Covers: operation:effect.open, operation:effect.close
#[test]
fn a_performed_effect_answers_its_kept_output_on_a_restart() {
    let env = TestEnv::new();
    let (opened, closed) = killed(
        || on(&env, None),
        "kept",
        None,
        |mut calls| async move {
            let opened = calls.open("kept", "charge-order-7").await;
            let closed = calls
                .close(
                    "kept",
                    &opened,
                    json!({ "ended": "returned", "output": { "charge": "ch_1" } }),
                )
                .await;
            (opened, closed)
        },
    );

    let again = declared_again(|| on(&env, None), "kept", "charge-order-7");

    let key = opened["result"]["idempotency_key"]
        .as_str()
        .unwrap_or_default();
    assert_eq!(
        (
            &opened["result"]["answer"],
            key.len(),
            &closed["result"]["judged"],
            &again["result"]["answer"],
            &again["result"]["output"],
            &again["result"]["performed_at"],
        ),
        (
            &json!("perform"),
            43,
            &json!("done"),
            &json!("journal"),
            &json!({ "charge": "ch_1" }),
            &closed["result"]["performed_at"],
        ),
        "{opened} {closed} {again}"
    );
}

/// The idempotency key is the same for a key on every declaration, and
/// differs between keys.
/// Covers: operation:effect.open
#[test]
fn the_idempotency_key_is_the_same_on_every_declaration_of_a_key() {
    let env = TestEnv::new();
    let (first, other) = killed(
        || on(&env, None),
        "keyed",
        None,
        |mut calls| async move {
            let first = calls.open("keyed", "refund-3").await;
            let other = calls.open("keyed", "refund-4").await;
            (first, other)
        },
    );

    let again = declared_again(|| on(&env, None), "keyed", "refund-3");

    let key = |reply: &Value| reply["result"]["idempotency_key"].clone();
    assert_eq!(
        (key(&again) == key(&first), key(&other) == key(&first)),
        (true, false),
        "{first} {other} {again}"
    );
}

/// Covers: error:effect_key_invalid
#[rstest]
#[case::empty("")]
#[case::longer_than_255_code_points(&"é".repeat(256))]
fn a_key_out_of_shape_is_refused_naming_it(#[case] key: &str) {
    let given = key.to_owned();
    let refused = started(
        &TestEnv::new(),
        "shaped",
        json!({}),
        |mut calls| async move { calls.open("shaped", &given).await },
    );

    assert_eq!(
        (code(&refused), &refused["error"]["data"]["key"]),
        (&json!("effect_key_invalid"), &json!(key)),
        "{refused}"
    );
}

/// A key of exactly 255 code points, each two bytes in UTF-8, is declared.
/// Covers: operation:effect.open
#[test]
fn a_key_of_255_code_points_is_declared() {
    let opened = started(
        &TestEnv::new(),
        "longest",
        json!({}),
        |mut calls| async move { calls.open("longest", &"é".repeat(255)).await },
    );

    assert_eq!(opened["result"]["answer"], json!("perform"), "{opened}");
}

/// Covers: error:effect_key_reused
#[rstest]
#[case::while_its_body_runs(false)]
#[case::after_its_body_was_done(true)]
fn a_key_its_owner_declares_again_is_refused(#[case] closed_first: bool) {
    let again = started(
        &TestEnv::new(),
        "reused",
        json!({}),
        |mut calls| async move {
            let opened = calls.open("reused", "send-mail").await;
            if closed_first {
                calls
                    .close("reused", &opened, json!({ "ended": "returned" }))
                    .await;
            }
            calls.open("reused", "send-mail").await
        },
    );

    assert_eq!(
        (code(&again), &again["error"]["data"]["key"]),
        (&json!("effect_key_reused"), &json!("send-mail")),
        "{again}"
    );
}

/// A body that ended without a value the journal keeps is dropped when
/// nothing was sent for it, or when it is repeatable, and runs again at its
/// key's next declaration.
/// Covers: operation:effect.close
#[rstest]
#[case::threw_with_nothing_sent(false, false, "threw", false)]
#[case::cut_short_with_nothing_sent(false, false, "cut_short", false)]
#[case::repeatable_threw_after_a_send(true, true, "threw", true)]
fn a_dropped_body_runs_again(
    #[case] repeatable: bool,
    #[case] sent: bool,
    #[case] ended: &str,
    #[case] dropped_sent: bool,
) {
    let ended = ended.to_owned();
    let (closed, again) = started(
        &TestEnv::new(),
        "dropped",
        json!({}),
        |mut calls| async move {
            let declared = json!({ "run_id": "dropped", "key": "post", "repeatable": repeatable });
            let opened = calls.call("effect.open", declared.clone()).await;
            if sent {
                calls.send("dropped", "Input.dispatchMouseEvent").await;
            }
            let closed = calls
                .close("dropped", &opened, json!({ "ended": ended }))
                .await;
            let again = calls.call("effect.open", declared).await;
            (closed, again)
        },
    );

    assert_eq!(
        (&closed["result"], &again["result"]["answer"]),
        (
            &json!({ "judged": "dropped", "sent": dropped_sent }),
            &json!("perform")
        ),
        "{closed} {again}"
    );
}

/// A body that failed after a send leaves its Effect uncertain: every
/// later declaration raises `effect_uncertain` with its key and reason,
/// and the Run report lists it and needs review.
/// Covers: error:effect_uncertain, operation:effect.close
#[rstest]
#[case::threw("threw", "failed_after_send")]
#[case::cut_short("cut_short", "interrupted")]
fn a_body_that_failed_after_a_send_leaves_its_effect_uncertain(
    #[case] ended: &str,
    #[case] reason: &str,
) {
    let ended = ended.to_owned();
    let (closed, again, report) = started(
        &TestEnv::new(),
        "unsure",
        json!({}),
        |mut calls| async move {
            let opened = calls.open("unsure", "pay").await;
            calls.send("unsure", "Input.dispatchKeyEvent").await;
            let closed = calls
                .close("unsure", &opened, json!({ "ended": ended }))
                .await;
            let again = calls.open("unsure", "pay").await;
            let report = calls
                .call("run.finish", json!({ "run_id": "unsure" }))
                .await;
            (closed, again, report)
        },
    );

    let effect = &closed["result"]["effect"];
    let report = &report["result"];
    assert_eq!(
        (
            &closed["result"]["judged"],
            &effect["key"],
            &effect["reason"],
            &effect["first_command"],
            code(&again),
            &again["error"]["data"]["key"],
            &again["error"]["data"]["reason"],
            &report["uncertain_effects"],
            &report["verdict"],
        ),
        (
            &json!("uncertain"),
            &json!("pay"),
            &json!(reason),
            &json!("Input.dispatchKeyEvent"),
            &json!("effect_uncertain"),
            &json!("pay"),
            &json!(reason),
            &json!([effect]),
            &json!("needs_review"),
        ),
        "{closed} {again} {report}"
    );
}

/// A value larger than `effect.output.max_size` is `effect_output_invalid`
/// when nothing was sent, and leaves the Effect uncertain after a send.
/// Covers: setting:effect.output.max_size
#[rstest]
/// Covers: error:effect_output_invalid
#[case::nothing_sent(
    false,
    json!([null, null, "effect_output_invalid", ["export", 25, 16], "perform", null])
)]
/// Covers: error:effect_uncertain
#[case::after_a_send(
    true,
    json!(["uncertain", "effect_output_invalid", null, [null, null, null], null, "effect_uncertain"])
)]
fn a_value_too_large_for_the_journal_is_not_kept(#[case] sent: bool, #[case] expected: Value) {
    let settings = json!({ "settings": { "effect": { "output": { "max_size": 16 } } } });
    let (closed, again) = started(&TestEnv::new(), "large", settings, |mut calls| async move {
        let opened = calls.open("large", "export").await;
        if sent {
            calls.send("large", "Page.navigate").await;
        }
        let closed = calls
            .close(
                "large",
                &opened,
                json!({ "ended": "returned", "output": "more than sixteen bytes" }),
            )
            .await;
        let again = calls.open("large", "export").await;
        (closed, again)
    });

    let data = &closed["error"]["data"];
    let shown = json!([
        closed["result"]["judged"],
        closed["result"]["cause"],
        data["code"],
        [&data["key"], &data["size"], &data["limit"]],
        again["result"]["answer"],
        code(&again)
    ]);
    assert_eq!(shown, expected, "{closed} {again}");
}

/// A person's three answers about an uncertain Effect: each is in the
/// Audit log, "it happened" answers `reconciled` and "it did not happen"
/// runs the body again, both off the Run report, while "leave uncertain"
/// changes nothing.
#[rstest]
#[case::happened("happened", json!("reconciled"), json!(null), json!("verified"))]
#[case::did_not_happen("did_not_happen", json!("perform"), json!(null), json!("verified"))]
/// Covers: error:effect_uncertain
#[case::leave_uncertain("leave_uncertain", json!(null), json!(1), json!("needs_review"))]
fn a_persons_answer_reconciles_an_uncertain_effect(
    #[case] answer: &str,
    #[case] declared: Value,
    #[case] listed: Value,
    #[case] verdict: Value,
) {
    let env = TestEnv::new();
    let given = answer.to_owned();
    let (closed, answered, again, report) = started(
        &env,
        "asked",
        json!({}),
        |mut calls| async move {
            let opened = calls.open("asked", "ship").await;
            calls.send("asked", "Runtime.callFunctionOn").await;
            let closed = calls
                .close("asked", &opened, json!({ "ended": "threw" }))
                .await;
            let answered = calls
                .call(
                    "test.effect_reconcile",
                    json!({ "run_id": "asked", "entry": opened["result"]["entry"], "answer": given }),
                )
                .await;
            let again = calls.open("asked", "ship").await;
            let report = calls.call("run.finish", json!({ "run_id": "asked" })).await;
            (closed, answered, again, report)
        },
    );

    let recorded: Vec<Value> = audit_entries(&env)
        .into_iter()
        .filter(|entry| entry["kind"] == "effect_reconciled")
        .map(|entry| {
            json!([
                entry["details"]["answer"],
                entry["details"]["key"],
                entry["run_id"]
            ])
        })
        .collect();
    let report = &report["result"];
    assert_eq!(
        (
            &answered["result"]["answered"],
            recorded,
            again["result"]["answer"].clone(),
            report["uncertain_effects"].as_array().map(Vec::len),
            &report["verdict"],
        ),
        (
            &closed["result"]["effect"],
            vec![json!([answer, "ship", "asked"])],
            declared,
            listed.as_u64().map(|count| usize::try_from(count).unwrap()),
            &verdict,
        ),
        "{answered} {again} {report}"
    );
}

/// An answer about an Effect that is not uncertain changes nothing and is
/// not recorded.
#[test]
fn an_answer_about_an_effect_that_is_not_uncertain_does_nothing() {
    let env = TestEnv::new();
    let answered = started(&env, "sure", json!({}), |mut calls| async move {
        let opened = calls.open("sure", "kept").await;
        calls
            .close("sure", &opened, json!({ "ended": "returned" }))
            .await;
        calls
            .call(
                "test.effect_reconcile",
                json!({ "run_id": "sure", "entry": opened["result"]["entry"], "answer": "happened" }),
            )
            .await
    });

    assert_eq!(
        (
            &answered["result"],
            audit_entries(&env)
                .iter()
                .filter(|entry| entry["kind"] == "effect_reconciled")
                .count()
        ),
        (&json!({ "answered": null }), 0),
        "{answered}"
    );
}

/// What a power cut at each point of an Effect leaves: before the sent
/// mark the body runs again, between the mark and the close the Effect is
/// uncertain, and after the close the journal answers. The mark is synced
/// before the command goes out, and the close is synced before it answers.
/// Covers: operation:effect.open
#[rstest]
#[case::before_the_mark(0, json!("perform"), json!(null))]
/// Covers: error:effect_uncertain
#[case::between_the_mark_and_the_close(1, json!(null), json!("effect_uncertain"))]
#[case::after_the_close(2, json!("journal"), json!(null))]
fn a_power_cut_leaves_each_effect_as_its_last_synced_step(
    #[case] steps: u8,
    #[case] answer: Value,
    #[case] refused: Value,
) {
    let env = TestEnv::new();
    let vfs = CrashVfs::register(&env.unique_name("effect-power-cut"), env.seed()).unwrap();
    killed(
        || on(&env, Some(&vfs)),
        "cut",
        Some((&vfs, CrashKind::PowerCut)),
        |mut calls| async move {
            let opened = calls.open("cut", "transfer").await;
            if steps >= 1 {
                calls.send("cut", "Input.dispatchMouseEvent").await;
            }
            if steps >= 2 {
                calls
                    .close("cut", &opened, json!({ "ended": "returned", "output": 1 }))
                    .await;
            }
        },
    );

    let again = declared_again(|| on(&env, Some(&vfs)), "cut", "transfer");

    assert_eq!(
        (&again["result"]["answer"], code(&again)),
        (&answer, &refused),
        "{again}"
    );
}
