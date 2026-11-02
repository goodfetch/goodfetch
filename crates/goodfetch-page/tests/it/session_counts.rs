//! B2 (ADR 0140): a Session against the scripted browser counts each CDP
//! command's work, each intercepted request's and each guarded action's, and
//! a plant of one extra unit shows in the count. Its figures against the
//! register's limits are `xtask counts`', and `--prove` runs the plant
//! through it.

#[path = "../../benches/session.rs"]
mod session;

use std::collections::BTreeMap;

use goodfetch_counted_work::Row;
use goodfetch_engine::metrics::Metrics;
use rstest::rstest;
use serde_json::{Map, Value};

use session::{Counted, READ_ROWS, counted, counting, page_check};

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
/// writer, which keeps its file open once per batch at most; the page a
/// Borrowed browser opens walks no memory, as the pool never measures it.
#[test]
fn a_session_counts_each_commands_work() {
    let Counted {
        commands,
        pages_opened,
        work,
        ..
    } = run(None);

    assert!(commands > 0, "the Session sent no command");
    assert_eq!(work.read(Row::CdpEncodes), commands);
    assert_eq!(work.read(Row::CdpSends), commands);
    assert_eq!(work.read(Row::CdpLogSendsOff), commands);
    assert_eq!((pages_opened, work.read(Row::PoolWalks)), (1, 0));
    let opens = work.read(Row::CdpFileOpens);
    assert!(
        opens > 0 && opens <= 2 * commands,
        "{opens} opens over {commands} commands"
    );
}

/// On each of the two documents, a click, a fill and a press each wait for
/// one check, which sends one command and `check.js`'s source with it (X11).
/// Each kind's commands: the click makes the guard's world in its document,
/// checks, moves, presses and releases the pointer and disarms; the fill
/// checks and inserts its text; the press checks, sends its key down and up,
/// and waits for the renderer. On the second document the click's first
/// check finds the first document's world gone, a check of its own, and the
/// world is made again exactly once.
#[test]
fn a_sessions_actions_count_each_check_and_their_commands_by_kind() {
    let Counted {
        checks,
        attempts,
        work,
        ..
    } = run(None);

    assert_eq!(
        attempts,
        BTreeMap::from([("click", 2), ("fill", 2), ("press", 2)])
    );
    assert_eq!(checks, 7);
    assert_eq!(work.read(Row::ActionScriptSends), checks);
    assert_eq!(work.read(Row::ActionCheckCommands), checks);
    assert_eq!(
        [
            Row::ActionClickCommands,
            Row::ActionFillCommands,
            Row::ActionPressCommands,
        ]
        .map(|row| work.read(row)),
        [13, 4, 8]
    );
}

/// A plant adds one unit of its row per command, which takes the row past
/// its limit: one over the exact rows, and past two for the writer's opens.
#[rstest]
#[case::encodes(Row::CdpEncodes, 2)]
#[case::sends(Row::CdpSends, 2)]
#[case::log_sends(Row::CdpLogSendsOff, 2)]
/// A page's walk is planted per command, at least one per page opened.
#[case::pool_walks(Row::PoolWalks, 1)]
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

/// Each engine read in a frame sends two commands, its isolated world's
/// creation and its call, and makes its world again even in a document it
/// read before (X10): every command sent while the program read is a read's,
/// and two reads ran in each of the three documents.
#[test]
fn each_read_counts_its_commands_and_its_world() {
    let counted = run(None);

    assert_eq!((counted.reads, counted.documents), (6, 3));
    assert_eq!(
        READ_ROWS.map(|row| counted.count(row)),
        [2 * counted.reads, counted.reads, counted.reads]
    );
    assert_eq!(counted.read_commands, counted.count(Row::PageReadCommands));
}

/// A plant runs once per command the browser reads, two per read, so each of
/// C2's rows gains two units a read.
#[rstest]
#[case::commands(Row::PageReadCommands, 2)]
#[case::world_creations(Row::PageReadWorldCreations, 1)]
#[case::document_worlds(Row::PageReadDocumentWorlds, 1)]
fn a_planted_unit_adds_two_a_read(#[case] row: Row, #[case] per_read: u64) {
    let counted = run(Some(row));

    assert_eq!(
        counted.count(row),
        (per_read + 2) * counted.reads,
        "{}",
        row.name()
    );
}

/// An action's row counts per check or per attempt, each of which sends at
/// least one command, so a plant of one unit per command adds at least one
/// per check and per attempt, past each row's limit.
#[rstest]
#[case::script_sends(Row::ActionScriptSends)]
#[case::check_commands(Row::ActionCheckCommands)]
#[case::click_commands(Row::ActionClickCommands)]
#[case::fill_commands(Row::ActionFillCommands)]
#[case::press_commands(Row::ActionPressCommands)]
fn a_planted_unit_adds_one_a_command_to_an_actions_row(#[case] row: Row) {
    let unplanted = run(None);

    let planted = run(Some(row));

    assert_eq!(planted.commands, unplanted.commands);
    assert_eq!(
        planted.work.read(row),
        unplanted.work.read(row) + planted.commands,
        "{}",
        row.name()
    );
}

/// Every request the page sends passes the interception (C5): each pause and
/// each answer to it is a `Fetch` message, as is the page's `Fetch.enable`;
/// each request leaving is matched once against the Filter lists, which are
/// on by default; each refusal spawns one task, which sends one entry to the
/// request log, whose folder and file it opens; nothing is compiled.
#[test]
fn a_session_counts_each_intercepted_requests_work() {
    let Counted {
        requests,
        paused,
        refused,
        work,
        ..
    } = run(None);

    assert!(
        refused > 0 && requests > refused && paused > requests,
        "{requests} requests, {paused} pauses, {refused} refused"
    );
    assert_eq!(
        [
            Row::InterceptedFetchMessages,
            Row::InterceptedAnswers,
            Row::InterceptedMatches,
            Row::InterceptedSpawns,
            Row::InterceptedEntries,
            Row::InterceptedFileOpens,
            Row::InterceptedCompilations,
        ]
        .map(|row| work.read(row)),
        [
            2 * paused + 1,
            paused,
            requests,
            refused,
            refused,
            2 * refused,
            0
        ]
    );
}

/// A plant adds one unit of its row per command, and every intercepted
/// request's unit is answered by a command, so the row passes its limit.
#[rstest]
#[case::fetch_messages(Row::InterceptedFetchMessages)]
#[case::spawns(Row::InterceptedSpawns)]
#[case::file_opens(Row::InterceptedFileOpens)]
#[case::matches(Row::InterceptedMatches)]
#[case::answers(Row::InterceptedAnswers)]
#[case::compilations(Row::InterceptedCompilations)]
#[case::entries(Row::InterceptedEntries)]
fn a_planted_unit_adds_one_to_an_intercepted_requests_row_a_command(#[case] row: Row) {
    let unplanted = run(None).work.read(row);

    let Counted { commands, work, .. } = run(Some(row));

    assert_eq!(work.read(row), unplanted + commands, "{}", row.name());
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
