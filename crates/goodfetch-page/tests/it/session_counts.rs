//! B2 (ADR 0140): a Session against the scripted browser counts each CDP
//! command's work and each intercepted request's, and a plant of one extra
//! unit shows in the count. Its
//! figures against the register's limits are `xtask counts`', and `--prove`
//! runs the plant through it.

#[path = "../../benches/session.rs"]
mod session;

use goodfetch_counted_work::Row;
use goodfetch_engine::metrics::Metrics;
use rstest::rstest;
use serde_json::Map;

use session::{Counted, READ_ROWS, counted, counting};

fn run(planted: Option<Row>) -> Counted {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let metrics = Metrics::none();
    let plant = planted.map(|row| counting(metrics.counted_work(), row));
    counted(home.path(), metrics, Map::new(), plant)
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
/// and two reads ran in each of the two documents.
#[test]
fn each_read_counts_its_commands_and_its_world() {
    let counted = run(None);

    assert_eq!((counted.reads, counted.documents), (4, 2));
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
