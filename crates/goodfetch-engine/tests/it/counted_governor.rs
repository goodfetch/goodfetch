//! B3's governor scenario, which `examples/counts.rs` and `xtask counts` read:
//! each sample's work over the stand-in `/proc` is counted, and a plant adds
//! one unit of its row per unit (ADR 0140's E4).

use goodfetch_counted_work::{CountedWork, Row};
use rstest::rstest;

use crate::governor_scenario::{self as scenario, PROCESSES, SAMPLES, Sampled};

/// The E4 rows the governor's sample counts, each by its unit's count.
const ROWS: [Row; 5] = [
    Row::GovernorConfigurationReads,
    Row::GovernorMeminfoOpens,
    Row::GovernorStaticOpens,
    Row::GovernorCpuStatOpens,
    Row::GovernorProcessReads,
];

fn run(planted: Option<Row>) -> (Sampled, [u64; 5]) {
    let folder = tempfile::tempdir().unwrap();
    let work = CountedWork::default();
    let sampled = scenario::run(folder.path(), &work, planted);
    (sampled, ROWS.map(|row| work.read(row)))
}

/// Each sample reads no settings file, its settings coming from the
/// engine's snapshot of them (#1885), opens `meminfo` once, opens neither
/// `self/cgroup` nor `self/mountinfo`, which the governor read as it
/// started, opens no `cpu.stat`, and reads each process's memory once, the
/// browser's in the engine's own walk (#1889).
#[test]
fn each_sample_reads_meminfo_once_its_cgroup_never_and_each_process_once() {
    let (sampled, counts) = run(None);

    assert_eq!(
        (sampled.samples, sampled.process_samples, counts),
        (
            SAMPLES,
            SAMPLES * PROCESSES,
            [0, SAMPLES, 0, 0, SAMPLES * PROCESSES]
        )
    );
}

/// A plant adds one unit of its row per unit: per sample, and per process
/// for the memory reads.
#[rstest]
#[case::configuration_reads(Row::GovernorConfigurationReads, 0, SAMPLES)]
#[case::meminfo_opens(Row::GovernorMeminfoOpens, 1, 2 * SAMPLES)]
#[case::static_opens(Row::GovernorStaticOpens, 2, SAMPLES)]
#[case::process_reads(Row::GovernorProcessReads, 4, 2 * SAMPLES * PROCESSES)]
fn a_planted_unit_adds_one_per_unit(#[case] row: Row, #[case] at: usize, #[case] count: u64) {
    let (_, counts) = run(Some(row));

    assert_eq!(counts[at], count, "{}", row.name());
}
