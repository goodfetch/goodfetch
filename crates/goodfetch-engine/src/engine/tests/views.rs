//! Reading a Run the program does not hold, through the engine's public
//! interface over the in-memory network (spec #213's Seam 3; ADRs 0031 and
//! 0102): `run.report` answers an ended Run's report at once and a live
//! one's at its end, without ending it; a Run whose engine died says
//! `writer: none`, its report raises `run_interrupted`, and a resume with
//! its Run ID takes it up again; a Run another engine writes is read from
//! its committed state without taking or blocking its writer lock. An
//! engine dies here as a killed one does: its runtime is dropped with the
//! Run live, so its lock and its run file close and the Run stays
//! `running`.

use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, BufReader};

use super::run::{call, run_folder};
use super::{Client, attached, attached_on, dependencies, engine, hello};
use crate::transport::{self, Network};

/// Has an engine of `env` answer `calls`, then kills it: its runtime is
/// dropped with its Runs live.
fn killed_after(env: &TestEnv, calls: &[(&str, Value)]) {
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let (network, dialer) = transport::in_memory();
        let engine = engine(Product::Collie);
        let serving = engine.run_attached(Network::InMemory(network), dependencies(env));
        let talk = async {
            let (reader, writer) = dialer.connect().await.unwrap().split();
            let mut client = Client {
                lines: BufReader::new(reader).lines(),
                writer,
            };
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            for (id, (method, params)) in (1..).zip(calls) {
                let reply = call(&mut client, id, method, params.clone()).await;
                assert!(reply.get("result").is_some(), "{reply}");
            }
        };
        tokio::select! {
            served = serving => panic!("the engine ended: {served:?}"),
            () = talk => {}
        }
    });
    drop(runtime);
}

fn lock_file(folder: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(folder.join("run.lock"))
        .unwrap()
}

/// Reads messages until the answer to the call `id`, and answers it.
async fn answer_to(client: &mut Client, id: i64) -> Value {
    loop {
        let message = client.next().await.expect("the engine closed");
        if message["id"] == json!(id) {
            return message;
        }
    }
}

/// A report read of a live Run waits for its end, and the Run goes on
/// meanwhile: the read never ends it.
///
/// Covers: story:3-165, operation:run.report
#[test]
fn a_live_runs_report_arrives_at_its_end_and_the_read_never_ends_it() {
    let (noted, report) = attached(&engine(Product::Collie), |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await.unwrap();
        call(&mut client, 1, "run.start", json!({ "run_id": "watched" })).await;
        client
            .send(&[json!({
                "jsonrpc": "2.0", "id": 2, "method": "run.report",
                "params": { "run_id": "watched" },
            })])
            .await;
        let noted = call(
            &mut client,
            3,
            "run.set_progress_message",
            json!({ "run_id": "watched", "message": "Still going" }),
        )
        .await;
        client
            .send(&[json!({
                "jsonrpc": "2.0", "id": 4, "method": "run.finish",
                "params": { "run_id": "watched" },
            })])
            .await;
        (noted, answer_to(&mut client, 2).await)
    });

    assert_eq!(
        (
            noted["result"]["lifecycle"].clone(),
            report["result"]["lifecycle"].clone()
        ),
        (json!("running"), json!("finished"))
    );
}

/// An ended Run's report read after its ending call's answer is the one
/// its run file keeps, in the engine that ended it as in any other: that
/// engine keeps no copy of it, so a Daemon that runs Run after Run holds
/// no report, with its settings explained, for each Run it ended (#1729).
/// The ending call alone answers its settings (ADR 0102's note); the
/// engine that ended it adds only the last Checkpoint it wrote of it.
///
/// Covers: story:3-165, operation:run.report
#[test]
fn an_ended_runs_later_report_is_the_one_its_run_file_keeps_in_the_engine_that_ended_it_too() {
    let env = TestEnv::new();
    let (finished, here) = attached_on(&engine(Product::Collie), &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await.unwrap();
        call(&mut client, 1, "run.start", json!({ "run_id": "done" })).await;
        let finished = call(&mut client, 2, "run.finish", json!({ "run_id": "done" })).await;
        let here = call(&mut client, 3, "run.report", json!({ "run_id": "done" })).await;
        (finished, here)
    });
    let elsewhere = attached_on(&engine(Product::Collie), &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await.unwrap();
        call(&mut client, 1, "run.report", json!({ "run_id": "done" })).await
    });

    assert_eq!(
        (
            finished["result"]["settings"].is_array(),
            &here["result"],
            elsewhere["result"]["lifecycle"].clone()
        ),
        (true, &elsewhere["result"], json!("finished"))
    );
}

/// A Run whose engine died has no live writer: its progress says so as
/// its last value, its report raises `run_interrupted`, and a resume with
/// its Run ID takes it up at the next run epoch.
///
/// Covers: story:3-164, story:3-165, operation:run.progress, operation:run.report, error:run_interrupted
#[test]
fn a_run_whose_engine_died_says_writer_none_until_a_resume_with_its_id_takes_it_up() {
    let env = TestEnv::new();
    killed_after(
        &env,
        &[
            ("run.start", json!({ "run_id": "orphan" })),
            (
                "run.set_progress_message",
                json!({ "run_id": "orphan", "message": "Half way" }),
            ),
        ],
    );

    let (dead, report, resumed) =
        attached_on(&engine(Product::Collie), &env, |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await.unwrap();
            let dead = call(
                &mut client,
                1,
                "run.progress",
                json!({ "run_id": "orphan" }),
            )
            .await;
            let report = call(&mut client, 2, "run.report", json!({ "run_id": "orphan" })).await;
            call(&mut client, 3, "run.resume", json!({ "run_id": "orphan" })).await;
            let resumed = call(
                &mut client,
                4,
                "run.progress",
                json!({ "run_id": "orphan" }),
            )
            .await;
            (dead["result"].clone(), report, resumed["result"].clone())
        });

    assert_eq!(
        (
            [
                &dead["writer"],
                &dead["lifecycle"],
                &dead["epoch"],
                &dead["message"],
                &dead["last"]
            ],
            &report["error"]["data"]["code"],
            [&resumed["writer"], &resumed["epoch"], &resumed["last"]],
        ),
        (
            [
                &json!("none"),
                &json!("running"),
                &json!(1),
                &json!("Half way"),
                &json!(true)
            ],
            &json!("run_interrupted"),
            [&json!("live"), &json!(2), &json!(false)],
        )
    );
}

/// A Run another engine writes, one this engine cannot reach, is read from
/// its committed state: the read takes no lock, so the writer keeps its
/// own throughout.
///
/// Covers: story:3-165, operation:run.progress
#[test]
fn a_read_of_a_run_another_engine_writes_never_takes_its_writer_lock() {
    let env = TestEnv::new();
    killed_after(&env, &[("run.start", json!({ "run_id": "elsewhere" }))]);
    let folder = run_folder(&env, "elsewhere");
    let writer = lock_file(&folder);
    writer.try_lock().unwrap();

    let progress = attached_on(&engine(Product::Collie), &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await.unwrap();
        call(
            &mut client,
            1,
            "run.progress",
            json!({ "run_id": "elsewhere" }),
        )
        .await
    });

    assert_eq!(
        (
            progress["result"]["writer"].clone(),
            lock_file(&folder).try_lock_shared().is_err()
        ),
        (json!("live"), true)
    );
}

/// A report read of a Run another engine writes waits while that engine
/// holds the Run, reading its run file again, and raises
/// `run_interrupted` once the writer is gone without an end.
///
/// Covers: story:3-165, operation:run.report, error:run_interrupted
#[test]
fn a_report_read_of_a_run_another_engine_writes_waits_for_the_writer() {
    let env = TestEnv::new();
    killed_after(&env, &[("run.start", json!({ "run_id": "elsewhere" }))]);
    let folder = run_folder(&env, "elsewhere");
    let writer = lock_file(&folder);
    writer.try_lock().unwrap();

    let report = attached_on(&engine(Product::Collie), &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await.unwrap();
        client
            .send(&[json!({
                "jsonrpc": "2.0", "id": 1, "method": "run.report",
                "params": { "run_id": "elsewhere" },
            })])
            .await;
        let unlocked = std::cell::Cell::new(false);
        let gone = async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            unlocked.set(true);
            writer.unlock().unwrap();
        };
        let read = async {
            let report = answer_to(&mut client, 1).await;
            (report, unlocked.get())
        };
        let (report, ()) = tokio::join!(read, gone);
        report
    });

    assert_eq!(
        (report.0["error"]["data"]["code"].clone(), report.1),
        (json!("run_interrupted"), true)
    );
}
