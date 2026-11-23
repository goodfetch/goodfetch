//! The determinism check (ADR 0060): a few seeds each run twice, and the
//! two runs' Observation logs and `tracing` output must match line for
//! line. A mismatch is a leak of real time, of randomness drawn outside
//! `Entropy`, or of hash order, and the check names the first line that
//! differs so it is fixed at its source. Security values never reach the
//! comparison: the Observation log holds them redacted, and `STYLE.md`
//! keeps them out of `tracing` output.

use std::fmt;

use serde::Serialize;

use crate::observation::ObservationLog;

/// What one run of a seed produced, as the lines the check compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replay {
    observations: Vec<String>,
    tracing: Vec<String>,
}

impl Replay {
    /// A run's Observation log, one NDJSON line a record, and the bytes its
    /// test `tracing` subscriber wrote, whose timer reads simulated time.
    ///
    /// # Panics
    ///
    /// If a record cannot be written as JSON, which a plain record always
    /// can.
    pub fn new<R: Serialize>(log: &ObservationLog<R>, tracing: &[u8]) -> Self {
        let mut written = Vec::new();
        log.write_ndjson(&mut written)
            .expect("invariant: an Observation log's records are plain JSON values");
        Self {
            observations: lines(&written),
            tracing: lines(tracing),
        }
    }
}

fn lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Which output of a run differed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayOutput {
    /// The Observation log.
    ObservationLog,
    /// The `tracing` output.
    Tracing,
}

impl fmt::Display for ReplayOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ObservationLog => "Observation log",
            Self::Tracing => "tracing output",
        })
    }
}

/// The first line on which two runs of one seed differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// The seed that does not replay.
    pub seed: u64,
    /// The output it differs in.
    pub output: ReplayOutput,
    /// The line, counted from 1.
    pub line: usize,
    /// The first run's line, `None` when it ended before it.
    pub first: Option<String>,
    /// The second run's line, `None` when it ended before it.
    pub second: Option<String>,
}

impl fmt::Display for Divergence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shown = |line: &Option<String>| line.clone().unwrap_or_else(|| "(nothing)".to_owned());
        write!(
            formatter,
            "seed {} does not replay: line {} of its {} differs between two runs\n  first run:  {}\n  second run: {}",
            self.seed,
            self.line,
            self.output,
            shown(&self.first),
            shown(&self.second)
        )
    }
}

impl std::error::Error for Divergence {}

/// Runs each seed twice and compares the two runs' Observation logs, then
/// their `tracing` output.
///
/// # Errors
///
/// The first seed whose two runs differ, at the first line they differ on.
pub fn check_determinism(
    seeds: impl IntoIterator<Item = u64>,
    mut run: impl FnMut(u64) -> Replay,
) -> Result<(), Divergence> {
    for seed in seeds {
        let first = run(seed);
        let second = run(seed);
        let outputs = [
            (
                ReplayOutput::ObservationLog,
                &first.observations,
                &second.observations,
            ),
            (ReplayOutput::Tracing, &first.tracing, &second.tracing),
        ];
        for (output, first, second) in outputs {
            if let Some(divergence) = first_difference(first, second) {
                let (line, first, second) = divergence;
                return Err(Divergence {
                    seed,
                    output,
                    line,
                    first,
                    second,
                });
            }
        }
    }
    Ok(())
}

/// The first line, counted from 1, on which two runs' lines differ, with
/// each run's line there.
fn first_difference(
    first: &[String],
    second: &[String],
) -> Option<(usize, Option<String>, Option<String>)> {
    (0..first.len().max(second.len())).find_map(|index| {
        let (one, other) = (first.get(index), second.get(index));
        (one != other).then(|| (index + 1, one.cloned(), other.cloned()))
    })
}

#[cfg(test)]
mod tests;
