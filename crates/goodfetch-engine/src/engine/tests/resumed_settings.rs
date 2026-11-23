//! A setting a resume changes applies from the resume on and is recorded
//! in the Run trace with its layer (ADR 0102), through the engine's public
//! interface over the in-memory network (spec #213's Seam 3): the run file
//! keeps the digest of each setting the Run runs with that differs from
//! the catalog default, and a resume writes one line of
//! `trace/settings.jsonl` per key whose value differs from it, with its
//! layer and the resume's run epoch and never its value.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::home::RUN_TRACE_SETTINGS;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::run::{call, run_folder};
use super::{Client, attached_on, engine_with, hello};

const RUN: &str = "resumed";

/// The replies to a start of [`RUN`] under `began`, then a pause and a
/// resume under each of `resumes`, then its finish, and the lines of its
/// Run trace's settings log.
fn resumed(began: &Value, resumes: &[Value]) -> (Vec<Value>, Vec<Value>) {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, None);
    let start = json!({ "run_id": RUN, "settings": began });
    let resumes: Vec<Value> = resumes
        .iter()
        .map(|settings| json!({ "run_id": RUN, "settings": settings }))
        .collect();
    let replies = attached_on(&engine, &env, |mut client: Client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await;
        let mut replies = vec![call(&mut client, 1, "run.start", start).await];
        let mut id = 2;
        for resume in resumes {
            replies.push(call(&mut client, id, "run.pause", json!({ "run_id": RUN })).await);
            replies.push(call(&mut client, id + 1, "run.resume", resume).await);
            id += 2;
        }
        replies.push(call(&mut client, id, "run.finish", json!({ "run_id": RUN })).await);
        replies
    });
    let log = run_folder(&env, RUN).join(RUN_TRACE_SETTINGS);
    let lines = std::fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("a settings line is JSON"))
        .collect();
    (replies, lines)
}

/// The errors among `replies`: none when every call succeeded.
fn errors(replies: &[Value]) -> Vec<&Value> {
    replies
        .iter()
        .map(|reply| &reply["error"])
        .filter(|error| !error.is_null())
        .collect()
}

/// A key the resume sets to another value is recorded at the run layer,
/// with the run epoch the resume began.
///
/// Covers: story:3-175, operation:run.resume
#[test]
fn a_setting_a_resume_changes_is_a_line_of_the_runs_trace_with_its_layer() {
    let began = json!({ "timeout": { "wait": 4_000 } });

    let (replies, lines) = resumed(&began, &[json!({ "timeout": { "wait": 5_000 } })]);

    assert_eq!(
        (errors(&replies), lines),
        (
            Vec::<&Value>::new(),
            vec![json!({ "key": "timeout.wait", "layer": "run", "epoch": 2 })]
        )
    );
}

/// A key the resume leaves out falls back to the layers below the run's
/// own, the catalog default here, and is recorded at that layer.
///
/// Covers: story:3-175, operation:run.resume
#[test]
fn a_setting_a_resume_leaves_out_is_a_line_at_the_layer_it_falls_back_to() {
    let began = json!({ "egress": { "offline": true } });

    let (replies, lines) = resumed(&began, &[json!({})]);

    assert_eq!(
        (errors(&replies), lines),
        (
            Vec::<&Value>::new(),
            vec![json!({ "key": "egress.offline", "layer": "default", "epoch": 2 })]
        )
    );
}

/// Each resume compares with what the one before it applied, not with
/// what the Run began with, and its lines carry its own run epoch.
///
/// Covers: story:3-175, operation:run.resume
#[test]
fn each_resume_compares_with_the_settings_the_one_before_it_applied() {
    let began = json!({ "timeout": { "wait": 4_000 } });
    let raised = json!({ "timeout": { "wait": 5_000 } });

    let (replies, lines) = resumed(
        &began,
        &[
            raised.clone(),
            raised,
            json!({ "timeout": { "wait": 6_000 } }),
        ],
    );

    assert_eq!(
        (errors(&replies), lines),
        (
            Vec::<&Value>::new(),
            vec![
                json!({ "key": "timeout.wait", "layer": "run", "epoch": 2 }),
                json!({ "key": "timeout.wait", "layer": "run", "epoch": 4 }),
            ]
        )
    );
}

/// A resume that gives every setting the Run began with writes nothing to
/// the Run trace's settings log.
///
/// Covers: story:3-175, operation:run.resume
#[test]
fn a_resume_that_changes_no_setting_writes_no_settings_line() {
    let began = json!({ "timeout": { "wait": 4_000 } });

    let (replies, lines) = resumed(&began, std::slice::from_ref(&began));

    assert_eq!(
        (replies[2]["result"]["started_as"].clone(), lines),
        (json!("resumed"), Vec::<Value>::new())
    );
}
