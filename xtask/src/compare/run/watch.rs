//! The watch over a trial: a trial whose crawl stalls, or runs past its
//! bound, is stopped and its processes killed, and a timed trial stopped
//! once runs once more.

use std::process::Child;
use std::time::Duration;

use anyhow::{Context as _, Result};

use super::super::trial::{self, Outcome};
use super::super::trials::SITE_PAGES;

/// How long a watched trial may go with the Proving ground receiving no
/// request before it is stopped as stalled.
pub(crate) const STALL_SECONDS: u64 = 120;
/// The longest a counted run may take.
pub(crate) const COUNTED_BOUND_SECONDS: u64 = 1800;
/// The longest a timed trial of the [`SITE_PAGES`]-page site may take: a
/// browser trial took at most about 260 s on a loaded machine (#1903).
pub(crate) const TIMED_BOUND_SECONDS: u64 = 600;

/// Which trial the comparison watches, stopping it once its crawl stalls
/// or reaches its bound.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Watch {
    /// Not watched: a Callgrind run, whose slowdown would need another
    /// bound, and whose scenario 1 crawls finish.
    None,
    /// The `n`-th run of a cell under strace.
    Counted(u32),
    /// The `n`-th timed trial of a cell, of a site of so many pages.
    Timed(u32, u64),
}

/// Why the watched trial, `seconds` into its crawl and `quiet` seconds
/// since the Proving ground's count of requests last grew, at `requests`,
/// is stopped; none while it goes on.
pub(crate) fn stopped(watch: Watch, seconds: u64, quiet: u64, requests: u64) -> Option<String> {
    let (named, what, bound) = match watch {
        Watch::None => return None,
        Watch::Counted(number) => (
            format!("under `strace -f`, run {number}"),
            "a counted run",
            COUNTED_BOUND_SECONDS,
        ),
        Watch::Timed(number, pages) => (
            format!("timed trial {number}"),
            "a timed trial",
            TIMED_BOUND_SECONDS * pages.div_ceil(SITE_PAGES).max(1),
        ),
    };
    if quiet >= STALL_SECONDS {
        return Some(format!(
            "{named} stopped after {seconds} s: the Proving ground received no request for {quiet} s after {requests} requests"
        ));
    }
    (seconds >= bound).then(|| {
        format!(
            "{named} stopped after {seconds} s, the longest {what} may take, at {requests} requests"
        )
    })
}

/// A timed trial once run again when stopped: [`retried`].
pub(crate) enum Retried<T> {
    /// Its figures, and why its first attempt stopped when they are its
    /// retry's.
    Done {
        measured: T,
        retried: Option<String>,
    },
    /// Stopped twice: both reasons.
    Stopped(String),
}

/// Runs `attempt`, and once more when the comparison stopped it.
pub(crate) fn retried<T>(mut attempt: impl FnMut() -> Result<Outcome<T>>) -> Result<Retried<T>> {
    let first = match attempt()? {
        Outcome::Done(measured) => {
            return Ok(Retried::Done {
                measured,
                retried: None,
            });
        }
        Outcome::Stopped(reason) => reason,
    };
    Ok(match attempt()? {
        Outcome::Done(measured) => Retried::Done {
            measured,
            retried: Some(first),
        },
        Outcome::Stopped(second) => {
            Retried::Stopped(format!("{second}; its first attempt: {first}"))
        }
    })
}

/// Waits for the trial `child` on the Proving ground at `address` to exit;
/// when it is watched, reads its count of requests each second, and once
/// [`stopped`] says why, kills the trial's processes, each by its own ID,
/// and returns the reason.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a watched trial is watched by sleeping between reads of its Proving ground's request count (STYLE.md)"
)]
pub(super) fn watched(child: &mut Child, address: &str, watch: Watch) -> Result<Option<String>> {
    if matches!(watch, Watch::None) {
        return Ok(None);
    }
    let started = trial::now();
    let (mut requests, mut grew) = (0, started);
    loop {
        if child
            .try_wait()
            .context("waiting for xtask compare trial")?
            .is_some()
        {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_secs(1));
        let count = trial::request_count(address)?;
        if count != requests {
            (requests, grew) = (count, trial::now());
        }
        let reason = stopped(
            watch,
            started.elapsed().as_secs(),
            grew.elapsed().as_secs(),
            requests,
        );
        if let Some(reason) = reason {
            trial::kill_tree(child.id());
            child.wait().context("waiting for the stopped trial")?;
            return Ok(Some(reason));
        }
    }
}
