//! B2 (ADR 0140): a Session against the scripted browser counts each CDP
//! command's work, and a plant of one extra unit shows in the count. Its
//! figures against the register's limits are `xtask counts`', and `--prove`
//! runs the plant through it.

#[path = "../../benches/session.rs"]
mod session;

use goodfetch_counted_work::Row;
use goodfetch_engine::metrics::Metrics;
use rstest::rstest;
use serde_json::Map;

use session::{Counted, counted, counting};

fn run(planted: Option<Row>) -> Counted {
    let home = tempfile::tempdir().unwrap();
    let metrics = Metrics::none();
    let plant = planted.map(|row| counting(metrics.counted_work(), row));
    counted(home.path(), metrics, Map::new(), plant)
}

/// Every command costs one encode, one send and one send to the command log's
/// writer, which keeps its file open once per batch at most.
#[test]
fn a_session_counts_each_commands_work() {
    let Counted { commands, work } = run(None);

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
    let Counted { commands, work } = run(Some(row));

    assert_eq!(work.read(row), per_command * commands, "{}", row.name());
}

#[test]
fn a_planted_file_open_takes_the_row_past_two_a_command() {
    let Counted { commands, work } = run(Some(Row::CdpFileOpens));

    assert!(
        work.read(Row::CdpFileOpens) > 2 * commands,
        "{} opens over {commands} commands",
        work.read(Row::CdpFileOpens)
    );
}
