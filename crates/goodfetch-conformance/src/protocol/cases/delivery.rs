//! The delivery cases (ADRs 0003 and 0093), which need a crawl and so run
//! on Foxhound's executable (ADR 0094): at-least-once delivery, exactly-once
//! commits and a late commit refused. Each starts a crawl whose handler is
//! the harness, with a 1 ms `timeout.handler`, so a lease lasts its 10 s
//! margin and a little more. The harness names the crawl's operations by
//! their wire names, since only `goodfetch-protocol` names its Foxhound
//! module (ADR 0099). Each case reads the Run's Records back through
//! `run.records` once the Run finished, so it checks what committed: the
//! retry's Record once and the late attempt's never; and that a work item
//! whose commit counted is never asked for again and that the Run finishes
//! complete.

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

/// `run.records`, Foxhound's cursor over a Dataset's Records.
const RUN_RECORDS: &str = "run.records";

/// `run.outcomes`, Foxhound's reader of a crawl's outcomes in commit order.
const RUN_OUTCOMES: &str = "run.outcomes";

/// `$/record`, the notification that carries one Record it reads.
const RECORD: &str = "$/record";

/// `$/credit`, by which a Client lets a streaming call send more.
const CREDIT: &str = "$/credit";

/// How long the harness waits for a lease to end: its 10 s margin over the
/// 1 ms handler timeout, and room for a loaded machine.
const LEASE_WAIT: Duration = Duration::from_secs(30);

/// A crawl the harness handles: its Run ID, the engine's requests and
/// notifications not yet read, and the position after which the next
/// outcome it waits for commits.
struct Crawl {
    run_id: &'static str,
    queued: VecDeque<Value>,
    committed: Option<u64>,
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
            committed: None,
        })
    }

    /// The engine's next request or notification, within `deadline`.
    fn next(&mut self, connection: &mut Connection, deadline: Duration) -> Result<Value, Gone> {
        match self.queued.pop_front() {
            Some(message) => Ok(message),
            None => connection.next_within(deadline),
        }
    }

    /// Waits until the engine commits the Run's next outcome, through
    /// `run.outcomes`, which waits for the next commit while the Run is
    /// live, and keeps what the engine sent meanwhile.
    fn next_commit(&mut self, connection: &mut Connection) -> Result<(), Gone> {
        let mut params = json!({ "run_id": self.run_id });
        if let Some(position) = self.committed {
            params["after"] = json!(position);
        }
        let id = connection.request(RUN_OUTCOMES, &params)?;
        let answer = connection.answer(id)?;
        self.queued.extend(answer.others);
        self.committed = answer.message["result"]["next"].as_u64();
        Ok(())
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

/// The values of the Records of `pages` the finished Run `run_id`
/// committed, in number order, as `run.records` reads them, and whether
/// their numbers run from 1 with no gaps.
fn committed(connection: &mut Connection, run_id: &str) -> Result<(Vec<Value>, bool), Gone> {
    let id = connection.request(
        RUN_RECORDS,
        &json!({ "run_id": run_id, "dataset": "pages" }),
    )?;
    connection.notify(CREDIT, &json!({ "id": id, "credits": 64 }))?;
    let answer = connection.answer(id)?;
    let records: Vec<&Value> = answer
        .others
        .iter()
        .filter(|message| is(message, RECORD) && message["params"]["id"] == json!(id))
        .map(|message| &message["params"]["record"])
        .collect();
    let numbered = records
        .iter()
        .zip(1_u64..)
        .all(|(record, number)| record["number"] == json!(number))
        && answer.message["result"]["count"] == json!(records.len());
    let values = records
        .iter()
        .map(|record| record["value"].clone())
        .collect();
    Ok((values, numbered))
}

/// The work items of the finished Run `run_id`'s outcomes, in the order
/// the engine committed them, as `run.outcomes` reads them: a Record
/// commits with its work item's outcome, so this is its Records' commit
/// order.
fn commit_order(connection: &mut Connection, run_id: &str) -> Result<Vec<Value>, Gone> {
    let mut order = Vec::new();
    let mut after = None;
    loop {
        let mut params = json!({ "run_id": run_id });
        if let Some(position) = after {
            params["after"] = json!(position);
        }
        let id = connection.request(RUN_OUTCOMES, &params)?;
        let answer = connection.answer(id)?;
        let page = &answer.message["result"];
        let outcomes = page["outcomes"].as_array().cloned().unwrap_or_default();
        order.extend(
            outcomes
                .iter()
                .map(|outcome| outcome["request"]["id"].clone()),
        );
        if outcomes.is_empty() || page["ended"] != json!(false) {
            return Ok(order);
        }
        after = page["next"].as_u64();
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
    let (read, numbered) = committed(connection, "at-least-once")?;
    checks.check(
        "`run.records` reads the retry's Record once, numbered 1",
        read == [json!({ "page": 1 })] && numbered,
    );
    Ok(())
}

/// Each work item answered in time commits once: it is delivered once,
/// never cancelled, and the Run finishes complete. The second work item's
/// answer commits first: the harness answers the first only once the
/// engine committed the second's, so commit order is never ask order.
pub(super) fn exactly_once(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let mut crawl = Crawl::start(connection, "exactly-once", 2, checks)?;
    let mut asked = Vec::new();
    for _ in 0..2 {
        let request = crawl.next(connection, DEADLINE)?;
        checks.check(
            "each work item's attempt is asked for",
            is(&request, CRAWL_REQUEST_HANDLE),
        );
        asked.push(request);
    }
    let work_items: Vec<Value> = asked
        .iter()
        .map(|request| request["params"]["crawl_request"]["id"].clone())
        .collect();
    checks.check(
        "each work item is asked for once",
        work_items.len() == 2 && work_items[0] != work_items[1],
    );
    for (request, work_item) in asked.iter().zip(&work_items).rev() {
        connection.reply(&request["id"], &records(&json!({ "work_item": work_item })))?;
        crawl.next_commit(connection)?;
    }
    let (answer, sent) = crawl.finish(connection)?;
    checks.check(
        "no attempt answered in time is cancelled",
        !sent.iter().any(|message| is(message, CANCEL)),
    );
    finished_complete(&answer, &sent, checks);
    let (read, numbered) = committed(connection, "exactly-once")?;
    let order = commit_order(connection, "exactly-once")?;
    checks.check(
        "`run.records` reads each work item's Record once, numbered from 1 in commit order",
        order.len() == work_items.len()
            && work_items.iter().all(|work_item| order.contains(work_item))
            && read
                == order
                    .iter()
                    .map(|work_item| json!({ "work_item": work_item }))
                    .collect::<Vec<_>>()
            && numbered,
    );
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
    let (read, numbered) = committed(connection, "late-commit")?;
    checks.check(
        "`run.records` reads the retry's Record once and never the late answer's",
        read == [json!({ "late": false })] && numbered,
    );
    Ok(())
}
