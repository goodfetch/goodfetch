//! The start table's cases (ADR 0102): a start under a Run ID reaches the
//! Run it names, by what the Run index holds of it. Each case starts a Run
//! under an ID of its own, brings it to the row it checks (live, stopped,
//! killed with its engine, finished, stopped by its Limit, or listed with
//! its run file removed from the target's Goodfetch home) and starts it
//! again under that ID, with the same inputs or other ones. The crawl
//! cases run on Foxhound's executable, whose crawls take seeds, and fetch
//! pages of the Proving ground's plain host, the harness answering each
//! handler request with no Record.

use std::path::{Path, PathBuf};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::home::{RUN_FILE, RUNS};
use goodfetch_protocol::run::{RUN_FINISH, RUN_REPORT, RUN_START, RUN_STOP};
use goodfetch_protocol::usage::USAGE_SET;
use serde_json::{Value, json};

use super::consent::walls::major_of;
use super::delivery::{PLAIN_HOST, is};
use super::{Checks, Site};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::foxhound::{CRAWL_REQUEST_HANDLE, CRAWL_START};
use crate::protocol::hello_as;
use crate::protocol::target::{DEADLINE, EngineTarget};

/// A start table case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    New,
    Stopped,
    Killed,
    Finished,
    Live,
    StateGone,
    CrawlEnded,
    OtherSeeds,
    OtherKind,
    LimitReached,
}

impl Case {
    /// The case `id` names, if it is one of these.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-start-under-a-new-run-id-begins-it" => Self::New,
            "a-start-under-a-stopped-runs-id-resumes-it" => Self::Stopped,
            "a-start-under-the-id-of-a-run-whose-engine-was-killed-resumes-it" => Self::Killed,
            "a-start-under-a-finished-runs-id-answers-it-as-it-ended" => Self::Finished,
            "a-start-under-a-live-runs-id-is-run-in-use" => Self::Live,
            "a-start-under-a-listed-runs-id-whose-state-is-gone-is-run-state-unavailable" => {
                Self::StateGone
            }
            "a-crawl-start-under-a-finished-crawls-id-answers-it-calling-no-handler" => {
                Self::CrawlEnded
            }
            "a-crawl-start-under-a-crawls-id-with-other-seeds-is-run-conflict-naming-seeds" => {
                Self::OtherSeeds
            }
            "a-collie-start-under-a-crawls-id-is-run-conflict-naming-product-kind-and-seeds" => {
                Self::OtherKind
            }
            "a-start-under-the-id-of-a-crawl-its-limit-stopped-is-limit-reached" => {
                Self::LimitReached
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection to `target`, against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        target: &EngineTarget,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::New => new(connection, checks),
            Self::Stopped => stopped(connection, checks),
            Self::Killed => killed(connection, target, checks),
            Self::CrawlEnded => crawl_ended(connection, site, checks),
            Self::OtherSeeds => other_seeds(connection, site, checks),
            Self::OtherKind => other_kind(connection, site, checks),
            Self::LimitReached => limit_reached(connection, site, checks),
            Self::Finished => finished(connection, checks),
            Self::Live => live(connection, checks),
            Self::StateGone => {
                let home = target.environment.get("GOODFETCH_HOME").cloned();
                checks.check(
                    "the target runs in a Goodfetch home of its own",
                    home.is_some(),
                );
                match home {
                    Some(home) => state_gone(connection, &home, checks),
                    None => Ok(()),
                }
            }
        }
    }
}

/// The answer of `operation` with `params`.
fn answer_of(connection: &mut Connection, operation: &str, params: &Value) -> Result<Value, Gone> {
    let id = connection.request(operation, params)?;
    Ok(connection.answer(id)?.message)
}

/// The answer of `run.start` under `run_id`.
fn start(connection: &mut Connection, run_id: &str) -> Result<Value, Gone> {
    answer_of(connection, RUN_START.name, &json!({ "run_id": run_id }))
}

/// The answer of `operation` on the Run `run_id`.
fn end(connection: &mut Connection, operation: &str, run_id: &str) -> Result<Value, Gone> {
    answer_of(connection, operation, &json!({ "run_id": run_id }))
}

/// Checks that `answer` is `run.start`'s, started as `started_as`.
fn started_as(answer: &Value, started_as: &str, check: &str, checks: &mut Checks) {
    checks.check(
        format!("{check}, `{started_as}` (answered: {answer})"),
        answer["result"]["started_as"] == json!(started_as),
    );
}

/// Checks that `answer` failed with the Error code `code`.
fn failed_with(answer: &Value, code: &str, check: &str, checks: &mut Checks) {
    checks.check(
        format!("{check}, `{code}` (answered: {answer})"),
        answer["error"]["data"]["code"] == json!(code),
    );
}

/// A start under a Run ID no Run has begins the Run.
fn new(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-start-new";
    let answer = start(connection, run_id)?;
    started_as(
        &answer,
        "new",
        "a start under a new Run ID begins it",
        checks,
    );
    end(connection, RUN_FINISH.name, run_id)?;
    Ok(())
}

/// A start under the ID of a Run `run.stop` stopped resumes it, live again
/// until it finishes.
fn stopped(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-start-stopped";
    start(connection, run_id)?;
    let stop = end(connection, RUN_STOP.name, run_id)?;
    checks.check(
        "`run.stop` stops the Run",
        stop["result"]["lifecycle"] == json!("stopped"),
    );
    let again = start(connection, run_id)?;
    started_as(
        &again,
        "resumed",
        "a start under the stopped Run's ID resumes it",
        checks,
    );
    let finish = end(connection, RUN_FINISH.name, run_id)?;
    checks.check(
        "the resumed Run finishes",
        finish["result"]["lifecycle"] == json!("finished"),
    );
    Ok(())
}

/// A start under the ID of a Run whose engine was killed while it ran, its
/// run file left with no live writer, resumes it, live again until it
/// finishes. The Run runs on a second engine, a process of its own on the
/// same Goodfetch home, which the harness kills as a crash would.
fn killed(
    connection: &mut Connection,
    target: &EngineTarget,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-start-killed";
    let mut other = connection
        .another(target)
        .map_err(|error| Gone::Failed(error.to_string()))?;
    let opened = hello_as(&mut other, Product::Collie, Surface::Rust).is_ok();
    checks.check("a second engine opens its own Connection", opened);
    let first = if opened {
        Some(start(&mut other, run_id)?)
    } else {
        None
    };
    connection.keep(other.take_exchanged());
    let Some(first) = first else {
        other.close();
        return Ok(());
    };
    started_as(&first, "new", "the second engine begins the Run", checks);
    checks.check(
        "the second engine is a process of its own, killed while the Run runs",
        other.kill(),
    );
    let again = start(connection, run_id)?;
    started_as(
        &again,
        "resumed",
        "a start under the killed Run's ID resumes it",
        checks,
    );
    let finish = end(connection, RUN_FINISH.name, run_id)?;
    checks.check(
        "the resumed Run finishes",
        finish["result"]["lifecycle"] == json!("finished"),
    );
    Ok(())
}

/// A start under the ID of a finished Run answers it as it ended, and
/// leaves it finished.
fn finished(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-start-finished";
    start(connection, run_id)?;
    end(connection, RUN_FINISH.name, run_id)?;
    let again = start(connection, run_id)?;
    started_as(
        &again,
        "ended",
        "a start under the finished Run's ID answers it as it ended",
        checks,
    );
    checks.check(
        "the Run it answers is finished",
        again["result"]["lifecycle"] == json!("finished"),
    );
    Ok(())
}

/// A start under the ID of a Run the engine runs fails with `run_in_use`,
/// and leaves that Run live.
fn live(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-start-live";
    start(connection, run_id)?;
    let again = start(connection, run_id)?;
    failed_with(
        &again,
        "run_in_use",
        "a start under the ID of a Run the engine runs fails",
        checks,
    );
    let finish = end(connection, RUN_FINISH.name, run_id)?;
    checks.check(
        "the live Run still finishes",
        finish["result"]["lifecycle"] == json!("finished"),
    );
    Ok(())
}

/// A start under the ID of a finished Run whose run file was removed, the
/// Run index still listing it, fails with `run_state_unavailable` and
/// begins no Run.
fn state_gone(connection: &mut Connection, home: &str, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-start-state-gone";
    start(connection, run_id)?;
    end(connection, RUN_FINISH.name, run_id)?;
    let folder = major_of(home, run_id).map(|major| major.join(RUNS).join(run_id));
    checks.check(
        "the Run's folder is found in the Goodfetch home",
        folder.is_some(),
    );
    let Some(folder) = folder else {
        return Ok(());
    };
    let removed = remove_run_file(&folder);
    checks.check(
        format!("the Run's run file is removed ({removed:?})"),
        removed.is_ok(),
    );
    let again = start(connection, run_id)?;
    failed_with(
        &again,
        "run_state_unavailable",
        "a start under the ID of a listed Run whose run file is gone fails",
        checks,
    );
    let after = start(connection, run_id)?;
    failed_with(
        &after,
        "run_state_unavailable",
        "a second start fails the same, no Run having begun",
        checks,
    );
    Ok(())
}

/// Removes the run file in `folder` and its write-ahead log and shared
/// memory files, those that are there.
#[expect(
    clippy::disallowed_methods,
    reason = "the harness is synchronous and runs no async worker"
)]
fn remove_run_file(folder: &Path) -> std::io::Result<()> {
    let file: PathBuf = folder.join(RUN_FILE);
    std::fs::remove_file(&file)?;
    for suffix in ["-wal", "-shm"] {
        let mut companion = file.clone().into_os_string();
        companion.push(suffix);
        match std::fs::remove_file(&companion) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

/// The response to the request `id`, the harness answering meanwhile each
/// handler request of a crawl with no Record, so the crawl's work items
/// commit and nothing the engine asks goes unanswered.
fn served(connection: &mut Connection, id: i64) -> Result<Value, Gone> {
    Ok(served_counting(connection, id)?.0)
}

/// [`served`], with how many handler requests came before the response.
fn served_counting(connection: &mut Connection, id: i64) -> Result<(Value, usize), Gone> {
    let mut handled = 0;
    loop {
        let message = connection.next_within(DEADLINE)?;
        if message.get("method").is_none() && message.get("id") == Some(&json!(id)) {
            return Ok((message, handled));
        }
        if is(&message, CRAWL_REQUEST_HANDLE) {
            handled += 1;
            connection.reply(&message["id"], &json!({ "records": [] }))?;
        }
    }
}

/// `crawl.start` under `run_id` of the Proving ground's plain host's
/// `pages`, fetched on the `http` tier, and its answer.
fn crawl(target: (&mut Connection, &Site), run_id: &str, pages: &[&str]) -> Result<Value, Gone> {
    Ok(crawl_counting(target, run_id, pages)?.0)
}

/// [`crawl`], with how many handler requests came before its answer.
fn crawl_counting(
    (connection, site): (&mut Connection, &Site),
    run_id: &str,
    pages: &[&str],
) -> Result<(Value, usize), Gone> {
    let seeds: Vec<Value> = pages
        .iter()
        .map(|page| json!({ "url": site.named_url(PLAIN_HOST, page), "label": "page" }))
        .collect();
    let settings = site.mapped(
        &json!({ "fetch": { "tier": "http" } }),
        connection.acts_for_model(),
    );
    let id = connection.request(
        CRAWL_START,
        &json!({
            "run_id": run_id,
            "seeds": seeds,
            "labels": { "page": {} },
            "settings": settings,
        }),
    )?;
    served_counting(connection, id)
}

/// `run.finish` of `run_id`, its handler requests answered, so a case
/// leaves no crawl running.
fn finished_crawl(connection: &mut Connection, run_id: &str) -> Result<Value, Gone> {
    let id = connection.request(RUN_FINISH.name, &json!({ "run_id": run_id }))?;
    served(connection, id)
}

/// Checks that `answer` failed with `run_conflict` for the Run `run_id`,
/// listing `differences`.
fn conflict(answer: &Value, run_id: &str, differences: &[&str], check: &str, checks: &mut Checks) {
    failed_with(answer, "run_conflict", check, checks);
    let data = &answer["error"]["data"];
    checks.check(
        format!("its details name the Run and list {differences:?} (answered: {answer})"),
        data["run_id"] == json!(run_id) && data["differences"] == json!(differences),
    );
}

/// A crawl start under the ID of a finished crawl, with its seeds, answers
/// it as it ended and calls no handler: none is asked before the answer,
/// nor before a `run.report` sent after it.
fn crawl_ended(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-start-crawl-ended";
    let first = crawl((connection, site), run_id, &["/1"])?;
    started_as(
        &first,
        "new",
        "a crawl start under a new Run ID begins it",
        checks,
    );
    finished_crawl(connection, run_id)?;
    let again = crawl_counting((connection, site), run_id, &["/1"])?;
    started_as(
        &again.0,
        "ended",
        "a crawl start under the finished crawl's ID answers it as it ended",
        checks,
    );
    let id = connection.request(RUN_REPORT.name, &json!({ "run_id": run_id }))?;
    let (report, after) = served_counting(connection, id)?;
    checks.check(
        format!(
            "no handler is called, before the answer or after it ({} and {after} handler requests)",
            again.1
        ),
        again.1 == 0 && after == 0,
    );
    checks.check(
        format!("the crawl stays finished (answered: {report})"),
        report["result"]["lifecycle"] == json!("finished"),
    );
    Ok(())
}

/// A crawl start under the ID of a crawl begun with other seeds fails with
/// `run_conflict` listing `seeds`, and leaves that crawl as it was.
fn other_seeds(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-start-other-seeds";
    let first = crawl((connection, site), run_id, &["/1"])?;
    started_as(
        &first,
        "new",
        "a crawl start under a new Run ID begins it",
        checks,
    );
    let again = crawl((connection, site), run_id, &["/2"])?;
    conflict(
        &again,
        run_id,
        &["seeds"],
        "a crawl start under its ID with other seeds fails",
        checks,
    );
    let finish = finished_crawl(connection, run_id)?;
    checks.check(
        "the crawl it named still finishes",
        finish["result"]["lifecycle"] == json!("finished"),
    );
    Ok(())
}

/// A Collie Run's start under the ID of a crawl fails with `run_conflict`
/// listing `product`, `kind` and `seeds`: the crawl needs Foxhound, the
/// start a Collie Run, which has no seeds.
fn other_kind(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-start-other-kind";
    let first = crawl((connection, site), run_id, &["/1"])?;
    started_as(
        &first,
        "new",
        "a crawl start under a new Run ID begins it",
        checks,
    );
    let again = start(connection, run_id)?;
    conflict(
        &again,
        run_id,
        &["product", "kind", "seeds"],
        "a Collie Run's start under the crawl's ID fails",
        checks,
    );
    let finish = finished_crawl(connection, run_id)?;
    checks.check(
        "the crawl it named still finishes",
        finish["result"]["lifecycle"] == json!("finished"),
    );
    Ok(())
}

/// A crawl whose `run` Limit of one request is used up stops `stopped`
/// for it, and a start under its ID with the same seeds fails with
/// `limit_reached` naming the Limit's scope, the scope's ID and its Usage
/// kind, while no person raised it, never resuming the crawl.
fn limit_reached(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-start-limit";
    let scope = json!({ "kind": "run", "id": run_id });
    let set = answer_of(
        connection,
        USAGE_SET.name,
        &json!({ "scope": "run", "scope_id": run_id, "usage_kind": "requests", "amount": 1 }),
    )?;
    checks.check(
        format!("`usage.set` gives the Run a Limit of one request (answered: {set})"),
        set.get("result").is_some(),
    );
    let pages = ["/1", "/2", "/3"];
    let first = crawl((connection, site), run_id, &pages)?;
    started_as(
        &first,
        "new",
        "a crawl start under a new Run ID begins it",
        checks,
    );
    let id = connection.request(RUN_REPORT.name, &json!({ "run_id": run_id }))?;
    let report = served(connection, id)?;
    let stop = &report["result"]["stop_reason"];
    checks.check(
        format!("the crawl stops `stopped` for its Limit (answered: {report})"),
        report["result"]["lifecycle"] == json!("stopped")
            && stop["kind"] == json!("limit")
            && stop["scope"] == scope,
    );
    for check in [
        "a start under its ID fails",
        "a second start fails the same",
    ] {
        let again = crawl((connection, site), run_id, &pages)?;
        failed_with(&again, "limit_reached", check, checks);
        let data = &again["error"]["data"];
        checks.check(
            format!(
                "its details name the Limit's scope, its ID and its Usage kind (answered: {again})"
            ),
            data["scope"] == scope && data["usage_kind"] == json!("requests"),
        );
    }
    Ok(())
}
