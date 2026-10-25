//! The delivery cases (ADRs 0003 and 0093), which need a crawl and so run
//! on Foxhound's executable (ADR 0094): at-least-once delivery, exactly-once
//! commits and a late commit refused. Each starts a crawl whose handler is
//! the harness, with a 1 ms `timeout.handler`, so a lease lasts its 10 s
//! margin and a little more. The harness names the crawl's operations by
//! their wire names, since only `goodfetch-protocol` names its Foxhound
//! module (ADR 0099). Records are not read back yet: `run.records` (#961)
//! will let these cases check that the late attempt's Records are absent
//! and the retry's present once; until then they check what the protocol
//! shows of each commit, that a work item whose commit counted is never
//! asked for again and that the Run finishes complete.

use std::collections::VecDeque;
use std::time::Duration;

use goodfetch_protocol::connection::CANCEL;
use goodfetch_protocol::run::RUN_FINISH;
use serde_json::{Value, json};

use super::Checks;
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::DEADLINE;

/// `crawl.start`, Foxhound's.
const CRAWL_START: &str = "crawl.start";

/// `crawl_request.handle`, Foxhound's handler request.
const CRAWL_REQUEST_HANDLE: &str = "crawl_request.handle";

/// How long the harness waits for a lease to end: its 10 s margin over the
/// 1 ms handler timeout, and room for a loaded machine.
const LEASE_WAIT: Duration = Duration::from_secs(30);

/// A crawl the harness handles: its Run ID, and the engine's requests and
/// notifications not yet read.
struct Crawl {
    run_id: &'static str,
    queued: VecDeque<Value>,
}

impl Crawl {
    /// Starts a crawl of `seeds` pages under `run_id`, declaring the
    /// Dataset `pages`, its handler timeout 1 ms.
    fn start(
        connection: &mut Connection,
        run_id: &'static str,
        seeds: usize,
        checks: &mut Checks,
    ) -> Result<Self, Gone> {
        let seeds: Vec<Value> = (1..=seeds)
            .map(|page| json!({ "url": format!("https://shop.test/{page}"), "label": "page" }))
            .collect();
        let id = connection.request(
            CRAWL_START,
            &json!({
                "run_id": run_id,
                "seeds": seeds,
                "datasets": [{ "name": "pages" }],
                "settings": { "timeout": { "handler": 1 } },
            }),
        )?;
        let answer = connection.answer(id)?;
        checks.check(
            "`crawl.start` starts a new Run",
            answer.message["result"]["started_as"] == json!("new"),
        );
        Ok(Self {
            run_id,
            queued: answer.others.into(),
        })
    }

    /// The engine's next request or notification, within `deadline`.
    fn next(&mut self, connection: &mut Connection, deadline: Duration) -> Result<Value, Gone> {
        match self.queued.pop_front() {
            Some(message) => Ok(message),
            None => connection.next_within(deadline),
        }
    }

    /// `run.finish`, and every request or notification the engine sent
    /// before its answer.
    fn finish(mut self, connection: &mut Connection) -> Result<(Value, Vec<Value>), Gone> {
        let id = connection.request(RUN_FINISH.name, &json!({ "run_id": self.run_id }))?;
        let answer = connection.answer(id)?;
        let mut sent: Vec<Value> = self.queued.drain(..).collect();
        sent.extend(answer.others);
        Ok((answer.message, sent))
    }
}

fn is(message: &Value, method: &str) -> bool {
    message["method"] == json!(method)
}

/// One Record of the Dataset `pages`.
fn records(data: &Value) -> Value {
    json!({ "records": [{ "dataset": "pages", "data": data }] })
}

/// Checks that the Run finished complete and that the engine sent nothing
/// more for its work items before it did.
fn finished_complete(answer: &Value, sent: &[Value], checks: &mut Checks) {
    checks.check(
        "the Run finishes",
        answer["result"]["lifecycle"] == json!("finished"),
    );
    checks.check(
        "its results cover its scope",
        answer["result"]["completeness"] == json!("complete"),
    );
    checks.check(
        "no work item whose commit counted is asked for again",
        !sent.iter().any(|message| is(message, CRAWL_REQUEST_HANDLE)),
    );
}

/// The first attempt goes unanswered: when its lease ends, the engine
/// cancels it and asks again under a later lease epoch. Answers the first
/// request and the second.
fn lost_then_retried(
    connection: &mut Connection,
    crawl: &mut Crawl,
    checks: &mut Checks,
) -> Result<(Value, Value), Gone> {
    let first = crawl.next(connection, DEADLINE)?;
    checks.check(
        "the work item's first attempt is asked for",
        is(&first, CRAWL_REQUEST_HANDLE) && first["params"]["attempt"] == json!(1),
    );
    let cancel = crawl.next(connection, LEASE_WAIT)?;
    checks.check(
        "when its lease ends, the engine cancels the attempt with `$/cancel`",
        is(&cancel, CANCEL) && cancel["params"]["id"] == first["id"],
    );
    let second = crawl.next(connection, DEADLINE)?;
    let request = &second["params"];
    checks.check(
        "the work item is asked for again",
        is(&second, CRAWL_REQUEST_HANDLE)
            && request["crawl_request"]["id"] == first["params"]["crawl_request"]["id"]
            && request["attempt"] == json!(2),
    );
    checks.check(
        "under a later lease epoch",
        request["lease_epoch"].as_u64() > first["params"]["lease_epoch"].as_u64(),
    );
    Ok((first, second))
}

/// A work item whose attempt goes unanswered is delivered again once its
/// lease ends, and the retry's commit finishes the Run.
pub(super) fn at_least_once(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let mut crawl = Crawl::start(connection, "at-least-once", 1, checks)?;
    let (_, second) = lost_then_retried(connection, &mut crawl, checks)?;
    connection.reply(&second["id"], &records(&json!({ "page": 1 })))?;
    let (answer, sent) = crawl.finish(connection)?;
    finished_complete(&answer, &sent, checks);
    Ok(())
}

/// Each work item answered in time commits once: it is delivered once,
/// never cancelled, and the Run finishes complete.
pub(super) fn exactly_once(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let mut crawl = Crawl::start(connection, "exactly-once", 2, checks)?;
    let mut work_items = Vec::new();
    for _ in 0..2 {
        let request = crawl.next(connection, DEADLINE)?;
        checks.check(
            "each work item's attempt is asked for",
            is(&request, CRAWL_REQUEST_HANDLE),
        );
        work_items.push(request["params"]["crawl_request"]["id"].clone());
        connection.reply(
            &request["id"],
            &records(&json!({ "work_item": request["params"]["crawl_request"]["id"] })),
        )?;
    }
    checks.check(
        "each work item is asked for once",
        work_items.len() == 2 && work_items[0] != work_items[1],
    );
    let (answer, sent) = crawl.finish(connection)?;
    checks.check(
        "no attempt answered in time is cancelled",
        !sent.iter().any(|message| is(message, CANCEL)),
    );
    finished_complete(&answer, &sent, checks);
    Ok(())
}

/// The answer to an attempt cancelled at its lease's end is a late commit:
/// it is refused, so it changes nothing; the retry's commit is the one that
/// counts.
pub(super) fn late_commit(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let mut crawl = Crawl::start(connection, "late-commit", 1, checks)?;
    let (first, second) = lost_then_retried(connection, &mut crawl, checks)?;
    connection.reply(&first["id"], &records(&json!({ "late": true })))?;
    connection.reply(&second["id"], &records(&json!({ "late": false })))?;
    let (answer, sent) = crawl.finish(connection)?;
    checks.check(
        "the late answer gets no answer of its own",
        !sent.iter().any(|message| {
            message.get("method").is_none() && message.get("id") == Some(&first["id"])
        }),
    );
    finished_complete(&answer, &sent, checks);
    Ok(())
}
