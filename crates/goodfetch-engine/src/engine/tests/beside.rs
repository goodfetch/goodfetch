//! A call on one Run answers while another Run's call waits on its disk,
//! through the engine's public interface over the in-memory network (spec
//! #213's Seam 3; ADR 0145, issue 2258): the Runs' registry does a Run's
//! disk work beside its loop, so a slow run file holds that Run's calls
//! only. The holding VFS holds the next read or write of the run file or
//! its WAL.

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::{HeldVfs, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

use super::run::{call, run_folder};
use super::{attached_with, dependencies, engine_with, hello};
use crate::engine::Dependencies;
use crate::run_state::RunStateOptions;
use crate::tasks::BlockingPolicy;

/// The parameters of `method` on `run_id`.
fn params(method: &str, run_id: &str) -> Value {
    if method == "run.set_progress_message" {
        json!({ "run_id": run_id, "message": "on its way" })
    } else {
        json!({ "run_id": run_id })
    }
}

/// While Run `a`'s call waits on its run file, a progress message on the
/// live Run `b` is answered, the run file still held: a start that finds an ended Run, a
/// progress message, the report of an ended Run and a resume reading the
/// stored key each wait beside the registry, whatever they answer.
///
/// Covers: operation:run.set_progress_message
#[rstest]
#[case::start_found(&["run.finish"], "run.start")]
#[case::progress_message(&[], "run.set_progress_message")]
#[case::ended_report(&["run.finish"], "run.finish")]
#[case::stored_key(&["run.finish"], "run.resume")]
fn a_call_on_another_run_answers_while_one_runs_call_waits_on_its_disk(
    #[case] setup: &[&str],
    #[case] held_call: &str,
) {
    let env = TestEnv::new();
    let vfs = HeldVfs::register();
    let run_file = run_folder(&env, "a").join("run.sqlite");

    let replies = attached_with(
        &engine_with(Product::Collie, None),
        || Dependencies {
            blocking: BlockingPolicy::Threads,
            run_state: RunStateOptions {
                vfs: Some(vfs.name().to_owned()),
                ..RunStateOptions::default()
            },
            ..dependencies(&env)
        },
        |mut client| {
            let vfs = &vfs;
            async move {
                client.send(&[hello("collie", "0.1")]).await;
                client.next().await;
                call(&mut client, 1, "run.start", json!({ "run_id": "b" })).await;
                call(&mut client, 2, "run.start", json!({ "run_id": "a" })).await;
                for (id, method) in (3..).zip(setup) {
                    call(&mut client, id, method, params(method, "a")).await;
                }
                let held = vfs.hold_next(&run_file);
                client
                    .send(&[json!({
                        "jsonrpc": "2.0", "id": 10, "method": held_call,
                        "params": params(held_call, "a"),
                    })])
                    .await;
                assert!(
                    held.reached().await,
                    "the call on Run a never reached its run file"
                );
                client
                    .send(&[json!({
                        "jsonrpc": "2.0", "id": 11, "method": "run.set_progress_message",
                        "params": params("run.set_progress_message", "b"),
                    })])
                    .await;
                let mut replies = Vec::new();
                let mut holding = false;
                while replies.len() < 2 {
                    let message = client.next().await.expect("the engine closed");
                    if message["id"] == 11 {
                        holding = held.holding();
                        held.let_go();
                    }
                    if message["id"] == 10 || message["id"] == 11 {
                        replies.push(message);
                    }
                }
                (holding, replies)
            }
        },
    );

    let (holding, replies) = replies;
    let order: Vec<_> = replies.iter().map(|reply| reply["id"].clone()).collect();
    assert_eq!(
        (holding, order),
        (true, vec![json!(11), json!(10)]),
        "{replies:?}"
    );
}
