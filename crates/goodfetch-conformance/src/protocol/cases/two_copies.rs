//! The two-copies case (ADR 0003, story 3-43): a crawl's Run checkpoints
//! on the target, a second copy resumes that Checkpoint on another
//! machine at the next run epoch, and the first copy, which nothing
//! stopped, goes on writing. Each copy's Checkpoints and Exports go to a
//! stand-in destination that compares and sets the run epoch, the way a
//! destination fences a zombie: once the second copy's land, the first
//! copy's are refused. The second copy is a second Attached engine on a
//! Goodfetch home of its own, standing in for the other machine.

use std::path::Path;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::checkpoint::{RUN_CHECKPOINT, RUN_RESUME};
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::run::RUN_FINISH;
use serde_json::{Value, json};

use super::delivery::{PLAIN_HOST, is};
use super::{Checks, Site};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::foxhound::{CRAWL_REQUEST_HANDLE, CRAWL_START, RUN_EXPORT};
use crate::protocol::hello_as;
use crate::protocol::target::{DEADLINE, EngineMode, EngineTarget};

/// The Run both copies run.
const RUN_ID: &str = "conformance-two-copies";

/// A two-copies case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    Fenced,
}

impl Case {
    /// The case `id` names, if it is one of these.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        (id == "a-fenced-copys-checkpoints-and-exports-are-refused-where-the-destination-compares-and-sets")
            .then_some(Self::Fenced)
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
            Self::Fenced => fenced(connection, target, site, checks),
        }
    }
}

/// What a copy wrote: a Checkpoint or an Export.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Written {
    Checkpoint,
    Export,
}

/// A destination of the one Run's writes that keeps the highest run epoch
/// it took and refuses a write of a lower one, as ADR 0003's
/// compare-and-set destinations do.
#[derive(Default)]
struct CompareAndSetStore {
    held: u64,
    landed: Vec<(Written, u64)>,
    refused: Vec<(Written, u64)>,
}

impl CompareAndSetStore {
    /// Offers the write that `answer`'s result describes; an answer with
    /// no run epoch lands nowhere.
    fn offer(&mut self, written: Written, answer: &Value) {
        let Some(epoch) = answer["result"]["run_epoch"].as_u64() else {
            return;
        };
        if epoch >= self.held {
            self.held = epoch;
            self.landed.push((written, epoch));
        } else {
            self.refused.push((written, epoch));
        }
    }
}

/// The two copies of one crawl's Run, as the module says.
fn fenced(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let Some(elsewhere) = elsewhere(target) else {
        checks.check(
            "the target is an Attached engine, so a second copy runs in a Goodfetch home of its own",
            false,
        );
        return Ok(());
    };
    let location = target.folder.join("two-copies-checkpoint.sqlite");
    let mut store = CompareAndSetStore::default();

    crawl(connection, site)?;
    store.offer(
        Written::Checkpoint,
        &checkpoint(connection, Some(&location))?,
    );
    store.offer(Written::Export, &export(connection)?);

    let mut other = connection
        .another(&elsewhere)
        .map_err(|error| Gone::Failed(error.to_string()))?;
    let opened = hello_as(&mut other, Product::Foxhound, Surface::Rust).is_ok();
    checks.check(
        "a second copy opens its own Connection on another home",
        opened,
    );
    let resumed = if opened {
        Some(second_copy(&mut other, site, &location, &mut store))
    } else {
        None
    };
    connection.keep(other.take_exchanged());
    let resumed = match resumed {
        Some(Ok(resumed)) => resumed,
        Some(Err(gone)) => {
            other.close();
            return Err(gone);
        }
        None => {
            other.close();
            return Ok(());
        }
    };

    store.offer(Written::Checkpoint, &checkpoint(connection, None)?);
    store.offer(Written::Export, &export(connection)?);
    let finished = served(connection, RUN_FINISH.name, &json!({ "run_id": RUN_ID }))?;
    let other_finished = served(&mut other, RUN_FINISH.name, &json!({ "run_id": RUN_ID }));
    connection.keep(other.take_exchanged());
    other.close();
    let other_finished = other_finished?;

    checks.check(
        "the second copy resumes the Run from its Checkpoint",
        resumed["result"]["started_as"] == json!("resumed"),
    );
    checks.check(
        "the first copy's Checkpoint and Export land, then the second copy's at the next run epoch",
        store.landed
            == [
                (Written::Checkpoint, 1),
                (Written::Export, 1),
                (Written::Checkpoint, 2),
                (Written::Export, 2),
            ],
    );
    checks.check(
        "once the second copy's land, the first copy's Checkpoint and Export are refused",
        store.refused == [(Written::Checkpoint, 1), (Written::Export, 1)],
    );
    checks.check(
        "both copies finish",
        finished["result"]["lifecycle"] == json!("finished")
            && other_finished["result"]["lifecycle"] == json!("finished"),
    );
    Ok(())
}

/// The second copy: resumes the Run from the Checkpoint at `location`
/// under the crawl's settings, its machine mapping the Proving ground's
/// hosts as the first's does, then checkpoints it and exports its
/// Dataset, offering both to `store`; answers the resume.
fn second_copy(
    other: &mut Connection,
    site: &Site,
    location: &Path,
    store: &mut CompareAndSetStore,
) -> Result<Value, Gone> {
    let settings = settings(other, site);
    let resumed = served(
        other,
        RUN_RESUME.name,
        &json!({
            "run_id": RUN_ID,
            "checkpoint": location.display().to_string(),
            "settings": settings,
        }),
    )?;
    store.offer(Written::Checkpoint, &checkpoint(other, None)?);
    store.offer(Written::Export, &export(other)?);
    Ok(resumed)
}

/// The target's twin on a Goodfetch home of its own, beside the
/// target's, configured as the target's home is; none when the target is
/// a Daemon, whose further Connections reach the same engine.
#[expect(
    clippy::disallowed_methods,
    reason = "the harness is synchronous and runs no async worker"
)]
fn elsewhere(target: &EngineTarget) -> Option<EngineTarget> {
    if target.mode != EngineMode::Attached {
        return None;
    }
    let home = target.folder.join("two-copies-elsewhere");
    std::fs::create_dir_all(&home).ok()?;
    if let Some(here) = target.environment.get("GOODFETCH_HOME") {
        for version in std::fs::read_dir(here).ok()?.flatten() {
            let configuration = version.path().join("configuration.toml");
            if configuration.is_file() {
                let there = home.join(version.file_name());
                std::fs::create_dir_all(&there).ok()?;
                std::fs::copy(&configuration, there.join("configuration.toml")).ok()?;
            }
        }
    }
    let mut elsewhere = target.clone();
    elsewhere
        .environment
        .insert("GOODFETCH_HOME".to_owned(), home.display().to_string());
    Some(elsewhere)
}

/// Starts the crawl of one page of the Proving ground's plain host,
/// fetched on the `http` tier, declaring the Dataset `pages`.
fn crawl(connection: &mut Connection, site: &Site) -> Result<Value, Gone> {
    let settings = settings(connection, site);
    served(
        connection,
        CRAWL_START,
        &json!({
            "run_id": RUN_ID,
            "seeds": [{ "url": site.named_url(PLAIN_HOST, "/1"), "label": "page" }],
            "labels": { "page": {} },
            "datasets": [{ "name": "pages" }],
            "settings": settings,
        }),
    )
}

/// The crawl's settings: the `http` tier, and the Proving ground's hosts
/// mapped to its address.
fn settings(connection: &Connection, site: &Site) -> Value {
    site.mapped(
        &json!({ "fetch": { "tier": "http" } }),
        connection.acts_for_model(),
    )
}

/// `run.checkpoint` of the Run, at `location` when given.
fn checkpoint(connection: &mut Connection, location: Option<&Path>) -> Result<Value, Gone> {
    let mut params = json!({ "run_id": RUN_ID });
    if let Some(location) = location {
        params["location"] = json!(location.display().to_string());
    }
    served(connection, RUN_CHECKPOINT.name, &params)
}

/// `run.export` of the Dataset `pages` as JSON Lines.
fn export(connection: &mut Connection) -> Result<Value, Gone> {
    served(
        connection,
        RUN_EXPORT,
        &json!({ "run_id": RUN_ID, "dataset": "pages", "format": "jsonl" }),
    )
}

/// The response to `operation` with `params`, the harness answering each
/// handler request meanwhile with one Record of `pages`.
fn served(connection: &mut Connection, operation: &str, params: &Value) -> Result<Value, Gone> {
    let id = connection.request(operation, params)?;
    loop {
        let message = connection.next_within(DEADLINE)?;
        if message.get("method").is_none() && message.get("id") == Some(&json!(id)) {
            return Ok(message);
        }
        if is(&message, CRAWL_REQUEST_HANDLE) {
            connection.reply(
                &message["id"],
                &json!({ "records": [{ "dataset": "pages", "data": { "page": 1 } }] }),
            )?;
        }
    }
}
