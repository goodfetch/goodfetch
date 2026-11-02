//! B2 (ADR 0140): a Session against the scripted browser counts each CDP
//! command's work, and a plant of one extra unit shows in the count. Its
//! figures against the register's limits are `xtask counts`', and `--prove`
//! runs the plant through it.

#[path = "../../benches/session.rs"]
mod session;

use goodfetch_counted_work::Row;
use goodfetch_engine::metrics::Metrics;
use rstest::rstest;
use serde_json::{Map, Value};

use session::{Counted, counted, counting, page_check};

fn run(planted: Option<Row>) -> Counted {
    run_with(Map::new(), planted)
}

fn run_with(settings: Map<String, Value>, planted: Option<Row>) -> Counted {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let metrics = Metrics::none();
    let plant = planted.map(|row| counting(metrics.counted_work(), row));
    counted(home.path(), metrics, settings, plant)
}

/// The rows of a navigation's judgement (C4), each one's count.
fn judged(work: &goodfetch_counted_work::CountedWork) -> [u64; 4] {
    [
        Row::NavigationDetectionReads,
        Row::NavigationCompilations,
        Row::NavigationScriptCompilations,
        Row::NavigationProvenanceEntries,
    ]
    .map(|row| work.read(row))
}

/// Every command costs one encode, one send and one send to the command log's
/// writer, which keeps its file open once per batch at most.
#[test]
fn a_session_counts_each_commands_work() {
    let Counted { commands, work, .. } = run(None);

    assert!(commands > 0, "the Session sent no command");
    assert_eq!(work.read(Row::CdpEncodes), commands);
    assert_eq!(work.read(Row::CdpSends), commands);
    assert_eq!(work.read(Row::CdpLogSendsOff), commands);
    let opens = work.read(Row::CdpFileOpens);
    assert!(
        opens > 0 && opens <= 2 * commands,
        "{opens} opens over {commands} commands"
    );
}

/// A plant adds one unit of its row per command, which takes the row past
/// its limit: one over the exact rows, and past two for the writer's opens.
#[rstest]
#[case::encodes(Row::CdpEncodes, 2)]
#[case::sends(Row::CdpSends, 2)]
#[case::log_sends(Row::CdpLogSendsOff, 2)]
fn a_planted_unit_adds_one_a_command(#[case] row: Row, #[case] per_command: u64) {
    let Counted { commands, work, .. } = run(Some(row));

    assert_eq!(work.read(row), per_command * commands, "{}", row.name());
}

#[test]
fn a_planted_file_open_takes_the_row_past_two_a_command() {
    let Counted { commands, work, .. } = run(Some(Row::CdpFileOpens));

    assert!(
        work.read(Row::CdpFileOpens) > 2 * commands,
        "{} opens over {commands} commands",
        work.read(Row::CdpFileOpens)
    );
}

/// With no Page check, each navigation's document is read once for its Block
/// markers, once by the built-in detector for a password field (a Borrowed
/// browser's Session may hold a login) and once for the Consent prompt
/// runner's first pass, after the runner started in it: four detection
/// reads. It compiles nothing, and the built-in detector writes one
/// Provenance entry; the Consent page's prompt writes one more.
#[test]
fn each_navigation_counts_its_judgements_reads_and_entries() {
    let Counted {
        navigations, work, ..
    } = run(None);

    assert_eq!(navigations, 3);
    assert_eq!(judged(&work), [4 * navigations, 0, 0, navigations + 1]);
}

/// A Page check reads the document for its expected page's element in place
/// of the built-in detector's read, and compiles its login page's glob at each
/// judgement; the built-in detector writes no entry then, so only the
/// prompt's is left.
#[test]
fn a_page_check_reads_in_the_detectors_place_and_compiles_its_glob_at_each_navigation() {
    let settings = match page_check() {
        Value::Object(settings) => settings,
        other => panic!("settings are an object, not {other}"),
    };

    let Counted {
        navigations, work, ..
    } = run_with(settings, None);

    assert_eq!(judged(&work), [4 * navigations, navigations, 0, 1]);
}

/// A plant adds one unit of its navigation row per command the browser reads.
#[rstest]
#[case::detection_reads(Row::NavigationDetectionReads)]
#[case::compilations(Row::NavigationCompilations)]
#[case::script_compilations(Row::NavigationScriptCompilations)]
#[case::provenance_entries(Row::NavigationProvenanceEntries)]
fn a_planted_unit_adds_one_to_a_navigation_row_a_command(#[case] row: Row) {
    let unplanted = run(None).work.read(row);

    let Counted { commands, work, .. } = run(Some(row));

    assert_eq!(work.read(row), unplanted + commands, "{}", row.name());
}
