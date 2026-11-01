//! The governor's controller (ADR 0040): a pure function of its readings,
//! which judges how loaded the engine is and sizes each kind's concurrency
//! between its floor and its ceiling by additive increase and
//! multiplicative decrease. Memory past `governor.pressure` of the Memory
//! allowance, the host's own memory pressure, or CPU or Tokio workers busy
//! past the same share put the engine under pressure, and each target
//! halves, never below its floor; memory past `governor.critical`, or the
//! host's critical pressure, is critical. With every reading under it, a
//! kind whose admissions fill its target rises by a twentieth of its
//! ceiling, at least one, never past its ceiling; a target starts at its
//! ceiling, as ADR 0004's fixed cap did, so a floor equal to the ceiling
//! keeps it there. Under memory pressure, the allowance's or the host's,
//! nothing new is admitted past a kind's floor, whatever its target; busy
//! CPU or Tokio workers only halve each target, which a kind admits up to,
//! since a machine at full CPU is working, not in danger. Memory critical with every target at its
//! floor for `governor.last_resort_after` is the last resort.

use std::time::Duration;

use goodfetch_protocol::governor::{GovernorLevel, PressureSource, WorkKind};

/// A kind's floor and ceiling; a ceiling below the floor counts as the
/// floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bounds {
    pub(crate) floor: u32,
    pub(crate) ceiling: u32,
}

impl Bounds {
    fn clamp(self, target: u32) -> u32 {
        target.clamp(self.floor, self.ceiling.max(self.floor))
    }

    fn ceiling(self) -> u32 {
        self.ceiling.max(self.floor)
    }
}

/// What the controller works within, from the governor's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    /// Each kind's bounds, in [`WorkKind::ALL`]'s order.
    pub(crate) bounds: [Bounds; 3],
    /// `governor.pressure`, in percent.
    pub(crate) pressure: u64,
    /// `governor.critical`, in percent.
    pub(crate) critical: u64,
    /// `governor.last_resort_after`.
    pub(crate) last_resort_after: Duration,
}

/// What one sample read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Reading {
    /// When it was read, since the governor started.
    pub(crate) at: Duration,
    /// The Memory allowance, in bytes.
    pub(crate) allowance: u64,
    /// What the engine and its children use, in bytes.
    pub(crate) used: u64,
    /// The host's own memory pressure.
    pub(crate) host: GovernorLevel,
    /// The engine's CPU time since the last sample, in percent of its
    /// allowance.
    pub(crate) cpu_busy: u64,
    /// The Tokio workers' busy time since the last sample, in percent.
    pub(crate) tokio_busy: u64,
    /// Each kind's admissions now, in [`WorkKind::ALL`]'s order.
    pub(crate) in_use: [u32; 3],
}

/// What the controller decided at one sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Decision {
    pub(crate) level: GovernorLevel,
    pub(crate) source: Option<PressureSource>,
    /// Each kind's target, in [`WorkKind::ALL`]'s order.
    pub(crate) targets: [u32; 3],
    /// How many of each kind may be held while new ones are admitted: its
    /// target at normal and under busy CPU or Tokio workers; under memory
    /// pressure, the allowance's or the host's, its floor, since nothing
    /// new is admitted then (ADR 0040) but a kind never falls below its
    /// floor.
    pub(crate) admits: [u32; 3],
    pub(crate) last_resort: bool,
}

/// The controller's state between samples: each kind's target, and since
/// when memory is critical with every target at its floor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Controller {
    targets: Option<[u32; 3]>,
    critical_at_floor_since: Option<Duration>,
}

impl Controller {
    /// A controller that has read nothing yet.
    pub(crate) fn new() -> Self {
        Self {
            targets: None,
            critical_at_floor_since: None,
        }
    }

    /// Judges `reading` under `limits` and sizes each kind's target.
    pub(crate) fn step(&mut self, limits: &Limits, reading: &Reading) -> Decision {
        let (level, source) = judge(limits, reading);
        let previous = self.targets.unwrap_or(limits.bounds.map(Bounds::ceiling));
        let mut targets = [0; 3];
        for index in 0..WorkKind::ALL.len() {
            let bounds = limits.bounds[index];
            let target = bounds.clamp(previous[index]);
            targets[index] = match level {
                GovernorLevel::Normal if reading.in_use[index] >= target => {
                    bounds.clamp(target.saturating_add((bounds.ceiling() / 20).max(1)))
                }
                GovernorLevel::Normal => target,
                GovernorLevel::Pressure | GovernorLevel::Critical => bounds.clamp(target / 2),
            };
        }
        self.targets = Some(targets);
        let at_floor = targets
            .iter()
            .zip(limits.bounds)
            .all(|(target, bounds)| *target == bounds.floor);
        self.critical_at_floor_since = (level == GovernorLevel::Critical && at_floor)
            .then(|| self.critical_at_floor_since.unwrap_or(reading.at));
        let last_resort = self
            .critical_at_floor_since
            .is_some_and(|since| reading.at.saturating_sub(since) >= limits.last_resort_after);
        // Only memory, the allowance's or the host's, is in danger: it holds
        // each kind at its floor, at pressure as at critical, while busy CPU
        // or Tokio workers, which reach pressure at most, only halve each
        // target.
        let admits = match source {
            None | Some(PressureSource::Cpu | PressureSource::Tokio) => targets,
            Some(PressureSource::Memory | PressureSource::HostMemory) => {
                limits.bounds.map(|bounds| bounds.floor)
            }
        };
        Decision {
            level,
            source,
            targets,
            admits,
            last_resort,
        }
    }
}

/// How loaded `reading` says the engine is, and which reading holds it
/// there: memory before the host's, critical before pressure.
fn judge(limits: &Limits, reading: &Reading) -> (GovernorLevel, Option<PressureSource>) {
    let memory = share(reading.used, reading.allowance);
    let candidates = [
        (
            memory >= limits.critical,
            GovernorLevel::Critical,
            PressureSource::Memory,
        ),
        (
            reading.host == GovernorLevel::Critical,
            GovernorLevel::Critical,
            PressureSource::HostMemory,
        ),
        (
            memory >= limits.pressure,
            GovernorLevel::Pressure,
            PressureSource::Memory,
        ),
        (
            reading.host == GovernorLevel::Pressure,
            GovernorLevel::Pressure,
            PressureSource::HostMemory,
        ),
        (
            reading.cpu_busy >= limits.pressure,
            GovernorLevel::Pressure,
            PressureSource::Cpu,
        ),
        (
            reading.tokio_busy >= limits.pressure,
            GovernorLevel::Pressure,
            PressureSource::Tokio,
        ),
    ];
    candidates
        .into_iter()
        .find(|(holds, _, _)| *holds)
        .map_or((GovernorLevel::Normal, None), |(_, level, source)| {
            (level, Some(source))
        })
}

/// `used` in percent of `allowance`, rounded down; an allowance of nothing
/// is always full.
fn share(used: u64, allowance: u64) -> u64 {
    if allowance == 0 {
        return u64::MAX;
    }
    u64::try_from(u128::from(used) * 100 / u128::from(allowance)).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
