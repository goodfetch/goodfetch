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

/// Each sample reads both settings files, opens `meminfo` three times and
/// `self/cgroup` and `self/mountinfo` once each, opens no `cpu.stat`, and
/// reads the browser's memory twice beside the engine's once: the gaps X17
/// waives.
#[test]
fn each_sample_counts_its_settings_reads_its_opens_and_its_memory_reads() {
    let (sampled, counts) = run(None);

    assert_eq!(
        (sampled.samples, sampled.process_samples, counts),
        (
            SAMPLES,
            SAMPLES * PROCESSES,
            [2 * SAMPLES, 3 * SAMPLES, 2 * SAMPLES, 0, 3 * SAMPLES]
        )
    );
}

/// A plant adds one unit of its row per unit: per sample, and per process
/// for the memory reads.
#[rstest]
#[case::configuration_reads(Row::GovernorConfigurationReads, 0, 2 * SAMPLES + SAMPLES)]
#[case::meminfo_opens(Row::GovernorMeminfoOpens, 1, 3 * SAMPLES + SAMPLES)]
#[case::static_opens(Row::GovernorStaticOpens, 2, 2 * SAMPLES + SAMPLES)]
#[case::process_reads(Row::GovernorProcessReads, 4, 3 * SAMPLES + SAMPLES * PROCESSES)]
fn a_planted_unit_adds_one_per_unit(#[case] row: Row, #[case] at: usize, #[case] count: u64) {
    let (_, counts) = run(Some(row));

    assert_eq!(counts[at], count, "{}", row.name());
}
