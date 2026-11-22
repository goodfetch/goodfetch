//! The pre-flight: before a run's first timed trial, one short run of each
//! cell it measures under each tool the run will run it under, plain, under
//! strace and under Callgrind, then one of each hostile pages' and auto
//! cell, so a broken invocation fails the run in minutes rather than hours
//! into it (#2521: a Callgrind run's relative output path failed after 220
//! timed trials and 207 strace runs).

use anyhow::{Context as _, Result};

use super::super::matrix::{Cell, Config};

/// What a probe runs its cell under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Under {
    /// A timed trial's own run.
    Plain,
    Strace,
    Callgrind,
    /// A hostile pages' pass (#2439).
    Rules,
    /// An auto cell's run (#2411).
    Auto,
}

impl Under {
    /// The tool or pass, as the pre-flight's failure names it.
    fn named(self) -> &'static str {
        match self {
            Self::Plain => "a timed trial",
            Self::Strace => "strace",
            Self::Callgrind => "Callgrind",
            Self::Rules => "the hostile pages' pass",
            Self::Auto => "the auto cell's run",
        }
    }
}

/// One run of the pre-flight: `cell`'s tool under `under`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Probe {
    pub(super) cell: &'static Cell,
    pub(super) config: Config,
    pub(super) under: Under,
}

/// The pre-flight's runs: each of `measured` plain, then under strace when
/// it is one of `strace` and under Callgrind when it is one of `callgrind`,
/// then each other of `callgrind`, a default-cost cell that Callgrind alone
/// runs (#2438), under Callgrind, then each of `ruled` and `auto` in its
/// own pass. A caller whose host
/// lacks a counting tool hands it no cell, as the run will run none.
pub(super) fn probes(
    measured: &[(&'static Cell, Config)],
    strace: &[(&'static Cell, Config)],
    callgrind: &[(&'static Cell, Config)],
    ruled: &[(&'static Cell, Config)],
    auto: &[(&'static Cell, Config)],
) -> Vec<Probe> {
    let among = |cells: &[(&'static Cell, Config)], cell: &Cell| {
        cells.iter().any(|(other, _)| std::ptr::eq(*other, cell))
    };
    let probe = |under| {
        move |&(cell, config): &(&'static Cell, Config)| Probe {
            cell,
            config,
            under,
        }
    };
    measured
        .iter()
        .flat_map(|&(cell, config)| {
            [
                Some(Under::Plain),
                among(strace, cell).then_some(Under::Strace),
                among(callgrind, cell).then_some(Under::Callgrind),
            ]
            .into_iter()
            .flatten()
            .map(move |under| Probe {
                cell,
                config,
                under,
            })
        })
        .chain(
            callgrind
                .iter()
                .filter(|(cell, _)| !among(measured, cell))
                .map(probe(Under::Callgrind)),
        )
        .chain(ruled.iter().map(probe(Under::Rules)))
        .chain(auto.iter().map(probe(Under::Auto)))
        .collect()
}

/// Flies each of `probes` through `fly`, in order, and stops at the first
/// that fails, with its error under the cell and the tool it ran under.
pub(super) fn flown(probes: &[Probe], mut fly: impl FnMut(&Probe) -> Result<()>) -> Result<()> {
    for probe in probes {
        fly(probe).with_context(|| {
            format!(
                "the pre-flight's run of {} {} under {} failed, before the first timed trial",
                probe.cell.scenario.name(),
                probe.cell.name,
                probe.under.named()
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
