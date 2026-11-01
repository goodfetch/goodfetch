//! The controller through its interface (spec #213's Seam 4): sequences of
//! readings in, levels and targets out, with no clock.

use std::time::Duration;

use goodfetch_protocol::governor::{GovernorLevel, PressureSource};
use proptest::prelude::*;
use rstest::rstest;

use super::{Bounds, Controller, Decision, Limits, Reading};

const GIB: u64 = 1 << 30;

/// ADR 0040's defaults: `http` 1–200, `browser_page` 1–32, `model_call`
/// 1–16, pressure 85 %, critical 95 %, last resort after 60 s.
fn defaults() -> Limits {
    Limits {
        bounds: [
            Bounds {
                floor: 1,
                ceiling: 200,
            },
            Bounds {
                floor: 1,
                ceiling: 32,
            },
            Bounds {
                floor: 1,
                ceiling: 16,
            },
        ],
        pressure: 85,
        critical: 95,
        last_resort_after: Duration::from_secs(60),
    }
}

/// A reading at `second` with `used` of a 1 GiB allowance and nothing else
/// loaded.
fn memory(second: u64, used: u64) -> Reading {
    Reading {
        at: Duration::from_secs(second),
        allowance: GIB,
        used,
        host: GovernorLevel::Normal,
        cpu_busy: 0,
        tokio_busy: 0,
        in_use: [0; 3],
    }
}

fn percent(share: u64) -> u64 {
    GIB / 100 * share
}

#[rstest]
#[case::under_pressure_is_normal(memory(0, percent(84)), GovernorLevel::Normal, None)]
#[case::at_pressure_from_memory(
    memory(0, percent(85) + 1024),
    GovernorLevel::Pressure,
    Some(PressureSource::Memory)
)]
#[case::at_critical_from_memory(
    memory(0, percent(95) + 1024),
    GovernorLevel::Critical,
    Some(PressureSource::Memory)
)]
#[case::host_pressure_counts_as_its_own(
    Reading { host: GovernorLevel::Pressure, ..memory(0, 0) },
    GovernorLevel::Pressure,
    Some(PressureSource::HostMemory)
)]
#[case::host_critical_counts_as_its_own(
    Reading { host: GovernorLevel::Critical, ..memory(0, 0) },
    GovernorLevel::Critical,
    Some(PressureSource::HostMemory)
)]
#[case::busy_cpu(Reading { cpu_busy: 90, ..memory(0, 0) }, GovernorLevel::Pressure, Some(PressureSource::Cpu))]
#[case::busy_tokio_workers(
    Reading { tokio_busy: 85, ..memory(0, 0) },
    GovernorLevel::Pressure,
    Some(PressureSource::Tokio)
)]
#[case::memory_holds_before_cpu(
    Reading { cpu_busy: 100, ..memory(0, percent(96)) },
    GovernorLevel::Critical,
    Some(PressureSource::Memory)
)]
fn a_reading_past_a_threshold_sets_the_level_and_its_source(
    #[case] reading: Reading,
    #[case] level: GovernorLevel,
    #[case] source: Option<PressureSource>,
) {
    let decision = Controller::new().step(&defaults(), &reading);

    assert_eq!((decision.level, decision.source), (level, source));
}

#[test]
fn thresholds_are_settings() {
    let limits = Limits {
        pressure: 50,
        critical: 60,
        ..defaults()
    };

    let decision = Controller::new().step(&limits, &memory(0, percent(61)));

    assert_eq!(decision.level, GovernorLevel::Critical);
}

#[test]
fn under_pressure_each_target_halves_down_to_its_floor() {
    let mut controller = Controller::new();
    let pressed = memory(0, percent(90));

    let targets: Vec<[u32; 3]> = (0..9)
        .map(|_| controller.step(&defaults(), &pressed).targets)
        .collect();

    assert_eq!(
        targets,
        [
            [100, 16, 8],
            [50, 8, 4],
            [25, 4, 2],
            [12, 2, 1],
            [6, 1, 1],
            [3, 1, 1],
            [1, 1, 1],
            [1, 1, 1],
            [1, 1, 1],
        ]
    );
}

/// At memory pressure, the host's included, nothing new is admitted past
/// each kind's floor (ADR 0040's first step), while each target halves as
/// before; at normal a kind admits up to its target. CPU or Tokio workers
/// busy past `governor.pressure` only halve each target, which a kind
/// admits up to: a machine at full CPU is working, not in danger.
#[rstest]
#[case::at_normal(memory(0, percent(10)), [200, 32, 16])]
#[case::at_pressure(memory(0, percent(90)), [1, 1, 1])]
#[case::at_critical(memory(0, percent(99)), [1, 1, 1])]
#[case::at_host_pressure(Reading { host: GovernorLevel::Pressure, ..memory(0, 0) }, [1, 1, 1])]
#[case::busy_cpu_halves_each_target(Reading { cpu_busy: 90, ..memory(0, 0) }, [100, 16, 8])]
#[case::busy_tokio_workers_halve_each_target(
    Reading { tokio_busy: 90, ..memory(0, 0) },
    [100, 16, 8]
)]
#[case::memory_pressure_with_busy_cpu_holds_each_floor(
    Reading { cpu_busy: 100, ..memory(0, percent(90)) },
    [1, 1, 1]
)]
fn above_normal_what_each_kind_admits_follows_the_source(
    #[case] reading: Reading,
    #[case] admits: [u32; 3],
) {
    let decision = Controller::new().step(&defaults(), &reading);

    assert_eq!(decision.admits, admits);
}

#[test]
fn with_headroom_a_full_target_rises_by_a_twentieth_of_its_ceiling() {
    let mut controller = Controller::new();
    let _ = controller.step(&defaults(), &memory(0, percent(90)));
    let full = Reading {
        in_use: [100, 16, 0],
        ..memory(1, percent(10))
    };

    let decision = controller.step(&defaults(), &full);

    // `http` rises by 10, `browser_page` by 1 (32 / 20, at least one), and
    // `model_call`, with nothing in use, stays.
    assert_eq!(decision.targets, [110, 17, 8]);
}

#[test]
fn memory_critical_at_the_floor_past_the_set_time_is_the_last_resort() {
    let limits = Limits {
        bounds: [Bounds {
            floor: 1,
            ceiling: 1,
        }; 3],
        ..defaults()
    };
    let mut controller = Controller::new();
    let critical = |second| memory(second, percent(99));

    let resorts: Vec<bool> = [0, 30, 59, 60, 61]
        .into_iter()
        .map(|second| controller.step(&limits, &critical(second)).last_resort)
        .collect();

    assert_eq!(resorts, [false, false, false, true, true]);
}

#[test]
fn a_sample_out_of_critical_starts_the_last_resorts_time_again() {
    let limits = Limits {
        bounds: [Bounds {
            floor: 1,
            ceiling: 1,
        }; 3],
        ..defaults()
    };
    let mut controller = Controller::new();

    let resorts: Vec<bool> = [(0, 99), (50, 80), (51, 99), (110, 99), (111, 99)]
        .into_iter()
        .map(|(second, share)| {
            controller
                .step(&limits, &memory(second, percent(share)))
                .last_resort
        })
        .collect();

    assert_eq!(resorts, [false, false, false, false, true]);
}

fn level() -> impl Strategy<Value = GovernorLevel> {
    prop_oneof![
        Just(GovernorLevel::Normal),
        Just(GovernorLevel::Pressure),
        Just(GovernorLevel::Critical),
    ]
}

fn readings() -> impl Strategy<Value = Vec<Reading>> {
    proptest::collection::vec(
        (
            0..2 * GIB,
            level(),
            0..120_u64,
            0..120_u64,
            proptest::array::uniform3(0..300_u32),
        ),
        1..40,
    )
    .prop_map(|samples| {
        samples
            .into_iter()
            .zip(0_u64..)
            .map(
                |((used, host, cpu_busy, tokio_busy, in_use), second)| Reading {
                    at: Duration::from_secs(second),
                    allowance: GIB,
                    used,
                    host,
                    cpu_busy,
                    tokio_busy,
                    in_use,
                },
            )
            .collect()
    })
}

fn bounds() -> impl Strategy<Value = [Bounds; 3]> {
    proptest::array::uniform3(
        (0..50_u32, 0..300_u32).prop_map(|(floor, ceiling)| Bounds { floor, ceiling }),
    )
}

fn decisions(limits: &Limits, readings: &[Reading]) -> Vec<Decision> {
    let mut controller = Controller::new();
    readings
        .iter()
        .map(|reading| controller.step(limits, reading))
        .collect()
}

proptest! {
    /// Whatever it reads, each target stays between its floor and its
    /// ceiling, and a floor equal to the ceiling fixes it there.
    #[test]
    fn every_target_stays_within_its_floor_and_ceiling(bounds in bounds(), readings in readings()) {
        let limits = Limits { bounds, ..defaults() };

        let outside: Vec<(usize, u32)> = decisions(&limits, &readings)
            .iter()
            .flat_map(|decision| decision.targets.into_iter().enumerate())
            .filter(|(index, target)| {
                let Bounds { floor, ceiling } = bounds[*index];
                *target < floor || *target > ceiling.max(floor)
            })
            .collect();

        prop_assert_eq!(outside, Vec::<(usize, u32)>::new());
    }

    /// A target never rises above normal and never falls at normal: it
    /// decreases only under pressure and increases only with headroom.
    #[test]
    fn a_target_rises_only_at_normal_and_falls_only_above_it(readings in readings()) {
        let limits = defaults();

        let decisions = decisions(&limits, &readings);
        let wrong: Vec<usize> = decisions
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| {
                let normal = pair[1].level == GovernorLevel::Normal;
                pair[0].targets.iter().zip(pair[1].targets).any(|(before, after)| {
                    if normal { after < *before } else { after > *before }
                })
            })
            .map(|(index, _)| index + 1)
            .collect();

        prop_assert_eq!(wrong, Vec::<usize>::new());
    }
}
