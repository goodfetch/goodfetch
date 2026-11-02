//! B2 (ADR 0140): a Session against the scripted browser counts each CDP
//! command's work and each guarded action's, and a plant of one extra unit
//! shows in the count. Its figures against the register's limits are `xtask
//! counts`', and `--prove` runs the plant through it.

#[path = "../../benches/session.rs"]
mod session;

use std::collections::BTreeMap;

use goodfetch_counted_work::Row;
use goodfetch_engine::metrics::Metrics;
use rstest::rstest;
use serde_json::Map;

use session::{Counted, counted, counting};

fn run(planted: Option<Row>) -> Counted {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let metrics = Metrics::none();
    let plant = planted.map(|row| counting(metrics.counted_work(), row));
    counted(home.path(), metrics, Map::new(), plant)
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
