//! The hostile pages' passes (#2439): the Proving ground's tasks each tool
//! crawls there, and the rules it broke on each, counted from the Proving
//! ground's findings and requests, never from a tool's report. The counts
//! carry no bar until the owner sets one (#2439, Q3).
//!
//! Each count reads what its family already records, without adding to
//! it. A family that flags an address refuses it with `403` from then on
//! and records no finding of its requests, so the findings stop at the
//! flag: whether the address ended flagged is the family's own rule
//! applied to its findings (a honeypot on `traps`, the third
//! `retry_after_ignored` on `limits/1`, never on `limits/3`), and the
//! requests it refused afterwards are counted apart. `honeypot` is every
//! request for a path `robots.txt` disallows (`/hp/` on `traps`, nothing on
//! `limits`), read from the requests, which hold the refused ones too; a
//! task's waste, its duplicates and trap requests, is read over the
//! requests its host answered, the only ones the family judged; and a
//! wanted page is a task's item answered `200`, read with the family's own
//! rule for an item's URLs.

use std::collections::BTreeSet;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// When the family flags an address, refusing it from then on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Flagging {
    /// On its first honeypot request.
    Honeypot,
    /// On its `n`-th request inside a `Retry-After` pause.
    RetryAfterIgnored(u64),
    /// Never.
    Never,
}

/// A task of the Proving ground a hostile pass crawls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Task {
    /// Its name, `<family>/<ordinal>`, as each request and finding is
    /// labelled.
    pub(crate) name: &'static str,
    /// The host it is served on.
    pub(crate) host: &'static str,
    /// Its items, the pages it asks a crawl for: `/items/<prefix><nnnn>`,
    /// numbered from 1 on four digits.
    pub(crate) wanted: u64,
    pub(crate) item_prefix: &'static str,
    /// Whether the family reads an item's other URLs as the same page: a
    /// `;jsessionid=` dropped, `%2D` decoded.
    pub(crate) item_variants: bool,
    pub(crate) flagging: Flagging,
    /// The path its `robots.txt` disallows, if any.
    pub(crate) disallowed: Option<&'static str>,
    /// Whether its waste is read: the `traps` family records duplicates and
    /// trap requests; `limits` records neither.
    pub(crate) records_waste: bool,
}

/// `traps/1`: duplicate URLs of one item, a calendar and an archive
/// without end, honeypot links `robots.txt` disallows.
pub(crate) const TRAPS: Task = Task {
    name: "traps/1",
    host: "1.traps.test",
    wanted: 30,
    item_prefix: "I-",
    item_variants: true,
    flagging: Flagging::Honeypot,
    disallowed: Some("/hp/"),
    records_waste: true,
};

/// `limits/1`: 10 requests an address, then `429` with `Retry-After: 1`.
pub(crate) const LIMITS_RATE: Task = Task {
    name: "limits/1",
    host: "1.limits.test",
    wanted: 24,
    item_prefix: "L-",
    item_variants: false,
    flagging: Flagging::RetryAfterIgnored(3),
    disallowed: None,
    records_waste: false,
};

/// `limits/3`: at most 2 requests in flight on the host, one more answered
/// `503`.
pub(crate) const LIMITS_IN_FLIGHT: Task = Task {
    name: "limits/3",
    host: "3.limits.test",
    wanted: 24,
    item_prefix: "L-",
    item_variants: false,
    flagging: Flagging::Never,
    disallowed: None,
    records_waste: false,
};

/// The rules one tool broke on one task, and what it fetched there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Broken {
    pub(crate) task: String,
    /// Every request the task's host received from the tool.
    pub(crate) requests: u64,
    /// Findings the family recorded, none once it flagged the address.
    pub(crate) retry_after_ignored: u64,
    pub(crate) over_in_flight_cap: u64,
    /// Requests for a path `robots.txt` disallows, refused ones included.
    pub(crate) honeypot: u64,
    /// Whether the family's rule flagged the tool's address by its end.
    pub(crate) flagged: bool,
    /// Requests the host refused, having flagged the address.
    pub(crate) refused: u64,
    /// The task's items answered `200`, each once, and its items.
    pub(crate) wanted_fetched: u64,
    pub(crate) wanted: u64,
    /// Requests the family recorded as a page fetched again, or a trap
    /// request past its allowance, up to the flag; none where it records
    /// neither.
    pub(crate) duplicate: u64,
    pub(crate) trap: u64,
    /// Why the comparison stopped the tool before it ended its crawl.
    pub(crate) stopped: Option<String>,
}

/// The item number `target` names on `task`, by the family's own rule.
fn item(task: &Task, target: &str) -> Option<u64> {
    let path = target.split(['?', '#']).next().unwrap_or_default();
    let id = path.strip_prefix("/items/")?;
    let id = if task.item_variants {
        id.split(';')
            .next()
            .unwrap_or_default()
            .replace("%2D", "-")
            .replace("%2d", "-")
    } else {
        id.to_owned()
    };
    let digits = id.strip_prefix(task.item_prefix)?;
    let number: u64 = digits.parse().ok()?;
    (digits.len() == 4
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && (1..=task.wanted).contains(&number))
    .then_some(number)
}

/// An array of the observations that holds every one of its kind: none was
/// left unrecorded.
fn every<'a>(observations: &'a Value, name: &str) -> Result<&'a Vec<Value>> {
    let list = observations[name]
        .as_array()
        .with_context(|| format!("the observations hold no {name}"))?;
    let unrecorded = observations[format!("unrecorded_{name}")]
        .as_u64()
        .with_context(|| format!("the observations hold no count of unrecorded {name}"))?;
    if unrecorded > 0 {
        bail!("the Proving ground left {unrecorded} {name} unrecorded");
    }
    Ok(list)
}

/// The rules broken on `task`, read from the Proving ground's
/// `observations`; an error when they cannot be read whole.
pub(crate) fn broken(observations: &Value, task: &Task) -> Result<Broken> {
    let of_task = |entry: &&Value| entry["task"] == task.name;
    let requests: Vec<&Value> = every(observations, "requests")?
        .iter()
        .filter(of_task)
        .collect();
    let mut counts = Broken {
        task: task.name.to_owned(),
        requests: requests.len() as u64,
        retry_after_ignored: 0,
        over_in_flight_cap: 0,
        honeypot: 0,
        flagged: false,
        refused: 0,
        wanted_fetched: 0,
        wanted: task.wanted,
        duplicate: 0,
        trap: 0,
        stopped: None,
    };
    let mut honeypot_findings = 0;
    for finding in every(observations, "findings")?.iter().filter(of_task) {
        let count = match finding["kind"].as_str() {
            Some("retry_after_ignored") => &mut counts.retry_after_ignored,
            Some("over_in_flight_cap") => &mut counts.over_in_flight_cap,
            Some("honeypot") => &mut honeypot_findings,
            Some("duplicate") => &mut counts.duplicate,
            Some("trap") => &mut counts.trap,
            // A misstep these passes do not count: a link resolved against
            // the wrong base.
            Some("wrong_base") => continue,
            _ => bail!("a finding of an unknown kind: {finding}"),
        };
        *count += 1;
    }
    counts.flagged = match task.flagging {
        Flagging::Honeypot => honeypot_findings > 0,
        Flagging::RetryAfterIgnored(nth) => counts.retry_after_ignored >= nth,
        Flagging::Never => false,
    };
    counts.honeypot = task.disallowed.map_or(0, |disallowed| {
        requests
            .iter()
            .filter(|request| {
                request["target"]
                    .as_str()
                    .is_some_and(|target| target.starts_with(disallowed))
            })
            .count() as u64
    });
    // Both families refuse a flagged address with `403`, and nothing else.
    counts.refused = requests
        .iter()
        .filter(|request| request["status"] == 403)
        .count() as u64;
    let fetched: BTreeSet<u64> = requests
        .iter()
        .filter(|request| request["status"] == 200)
        .filter_map(|request| item(task, request["target"].as_str()?))
        .collect();
    counts.wanted_fetched = fetched.len() as u64;
    Ok(counts)
}

#[cfg(test)]
mod tests;
