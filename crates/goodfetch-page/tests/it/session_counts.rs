//! B2 (ADR 0140): a Session against the scripted browser counts each CDP
//! command's work, each intercepted request's and each guarded action's, and
//! a plant of one extra unit shows in the count. Its figures against the
//! register's limits are `xtask counts`', and `--prove` runs the plant
//! through it.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

#[path = "../../benches/session.rs"]
mod session;

use std::collections::BTreeMap;
use std::path::Path;

use goodfetch_counted_work::Row;
use goodfetch_engine::metrics::Metrics;
use goodfetch_protocol::home::RUN_TRACE_CDP_COMMANDS;
use goodfetch_protocol::page::BROWSER_CDP_COMMAND_LOG;
use goodfetch_test_client::HomeLayout;
use rstest::rstest;
use serde_json::{Map, Value, json};

use session::{Browsers, Counted, READ_ROWS, RUN, counted, counting, identified, page_check};

fn run(planted: Option<Row>) -> Counted {
    run_with(Map::new(), planted)
}

fn run_with(settings: Map<String, Value>, planted: Option<Row>) -> Counted {
    run_in(Browsers::Borrowed, settings, planted)
}

fn run_in(browsers: Browsers, settings: Map<String, Value>, planted: Option<Row>) -> Counted {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    run_at(home.path(), browsers, settings, planted)
}

fn run_at(
    home: &Path,
    browsers: Browsers,
    settings: Map<String, Value>,
    planted: Option<Row>,
) -> Counted {
    let metrics = Metrics::none();
    let plant = planted.map(|row| counting(metrics.counted_work(), row));
    counted(browsers, home, metrics, settings, plant)
}

/// The methods the Run trace's CDP command log of the Run in `home` holds,
/// in order; `None` when its file was never written.
#[expect(
    clippy::disallowed_methods,
    reason = "a Run trace file read once its engine ended"
)]
fn logged_methods(home: &Path) -> Option<Vec<String>> {
    let file = HomeLayout::of(home).run_trace(RUN, RUN_TRACE_CDP_COMMANDS);
    let text = std::fs::read_to_string(file).ok()?;
    Some(
        text.lines()
            .map(|line| {
                let line: Value = serde_json::from_str(line).expect("a line is JSON");
                line["method"]
                    .as_str()
                    .expect("a line names its method")
                    .to_owned()
            })
            .collect(),
    )
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

/// Every command costs the encode of its frame and one send, and with the CDP
/// command log off, its default, nothing goes to the Run trace's writer; a
/// typed command's params serialized first are an encode of their own (X21);
/// the page a Borrowed browser opens walks no memory, as the pool never
/// measures it.
#[test]
fn a_session_counts_each_commands_work() {
    let Counted {
        commands,
        pages_opened,
        work,
        ..
    } = run(None);

    assert!(commands > 0, "the Session sent no command");
    let encodes = work.read(Row::CdpEncodes);
    assert!(
        encodes > commands && encodes <= 2 * commands,
        "{encodes} encodes over {commands} commands"
    );
    assert_eq!(work.read(Row::CdpSends), commands);
    assert_eq!(work.read(Row::CdpLogSendsOff), 0);
    assert_eq!((pages_opened, work.read(Row::PoolWalks)), (1, 0));
    assert_eq!(work.read(Row::CdpFileOpens), 0);
}

/// On each of the two documents with actions, a click, a fill and a press
/// each wait for one check, which sends one command and `check.js`'s source
/// with it (X11). Each kind's commands: the click checks, moves, presses and
/// releases the pointer and disarms; the fill checks and inserts its text;
/// the press checks, sends its key down and up, and waits for the renderer.
/// Each navigation's charset read makes the guard's world in its document
/// before them, the earlier document's being gone, so no action makes it
/// and no check finds it gone. The program's reads are waits whose checks
/// count on these rows too, and are C2's: the figures leave them out.
#[test]
fn a_sessions_actions_count_each_check_and_their_commands_by_kind() {
    let counted = run(None);
    let checks = counted.checks;

    assert_eq!(
        counted.attempts,
        BTreeMap::from([("click", 2), ("fill", 2), ("press", 2)])
    );
    assert_eq!(checks, 6);
    assert_eq!(counted.count(Row::ActionScriptSends), checks);
    assert_eq!(counted.count(Row::ActionCheckCommands), checks);
    assert_eq!(
        [
            Row::ActionClickCommands,
            Row::ActionFillCommands,
            Row::ActionPressCommands,
        ]
        .map(|row| counted.count(row)),
        [10, 4, 8]
    );
}

/// A plant adds one unit of its row per command, which takes the row past
/// its limit.
#[rstest]
#[case::sends(Row::CdpSends, 2)]
#[case::log_sends(Row::CdpLogSendsOff, 1)]
#[case::file_opens(Row::CdpFileOpens, 1)]
/// A page's walk is planted per command, at least one per page opened.
#[case::pool_walks(Row::PoolWalks, 1)]
fn a_planted_unit_adds_one_a_command(#[case] row: Row, #[case] per_command: u64) {
    let Counted { commands, work, .. } = run(Some(row));

    assert_eq!(work.read(row), per_command * commands, "{}", row.name());
}

/// With `browser.cdp_command_log` off, its default, no command's method goes
/// to the Run trace's writer and the log's file is never written, however
/// many browsers the Run launches and closes.
/// Covers: setting:browser.cdp_command_log
#[test]
fn the_command_log_off_writes_no_file() {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");

    let Counted { commands, work, .. } = run_at(home.path(), Browsers::Pool, Map::new(), None);

    assert!(commands > 0, "the Session sent no command");
    assert_eq!(
        (work.read(Row::CdpLogSendsOff), logged_methods(home.path())),
        (0, None)
    );
}

/// With `browser.cdp_command_log` on, every command the Run's browsers read
/// is on file by its method, each launched browser's `Browser.close`
/// included, as each is sent to its writer once.
/// Covers: setting:browser.cdp_command_log
#[test]
fn the_command_log_on_holds_every_commands_method() {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let settings = Map::from_iter([(BROWSER_CDP_COMMAND_LOG.name.to_owned(), json!(true))]);

    let Counted { commands, work, .. } = run_at(home.path(), Browsers::Pool, settings, None);

    let methods = logged_methods(home.path()).expect("the command log is written");
    assert_eq!(
        (
            work.read(Row::CdpLogSendsOff),
            u64::try_from(methods.len()).expect("invariant: a few lines"),
        ),
        (commands, commands)
    );
    assert!(
        methods.iter().any(|method| method == "Browser.close"),
        "{methods:?}"
    );
}

/// A planted encode adds one a command to the encodes the Session counts.
#[test]
fn a_planted_encode_adds_one_a_command() {
    let unplanted = run(None);
    let planted = run(Some(Row::CdpEncodes));

    assert_eq!(planted.commands, unplanted.commands);
    assert_eq!(
        planted.work.read(Row::CdpEncodes) - unplanted.work.read(Row::CdpEncodes),
        planted.commands
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

/// A formats read serializes the page with two commands, `DOM.getDocument`
/// and `DOM.getOuterHTML`, each counted on its row (ADR 0140): every command
/// sent while the program read formats is the serialization's, one in each of
/// the three documents.
#[test]
fn each_serialization_counts_its_two_commands() {
    let counted = run(None);

    assert_eq!(counted.serializations, 3);
    assert_eq!(
        (
            counted.count(Row::PageSerializationCommands),
            counted.serialization_commands
        ),
        (2 * counted.serializations, 2 * counted.serializations)
    );
}

/// A `page.read` sends two commands, its isolated world's creation and its
/// one call, whatever number of fields it holds (ADR 0147): the program
/// reads one field and twenty in each of the three documents, and C2's
/// `read_commands` counts every command the browser read meanwhile.
#[test]
fn a_fields_read_sends_the_same_two_commands_at_one_field_and_at_twenty() {
    let counted = run(None);

    assert_eq!(counted.field_reads, 6);
    assert_eq!(counted.field_rows, BTreeMap::from([(1, 6), (20, 6)]));
    assert_eq!(counted.field_read_commands, counted.field_rows);
    assert_eq!(
        counted.count(Row::PageFieldReadCommands),
        2 * counted.field_reads
    );
}

/// A plant runs once per command the browser reads, two per `page.read`.
#[test]
fn a_planted_unit_adds_two_a_fields_read() {
    let counted = run(Some(Row::PageFieldReadCommands));

    assert_eq!(
        counted.count(Row::PageFieldReadCommands),
        4 * counted.field_reads
    );
}

/// A plant runs once per command the browser reads, two per serialization.
#[test]
fn a_planted_unit_adds_two_a_serialization() {
    let counted = run(Some(Row::PageSerializationCommands));

    assert_eq!(
        counted.count(Row::PageSerializationCommands),
        4 * counted.serializations
    );
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
/// Run trace's writer, which opens no file for it; nothing is compiled.
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
        [2 * paused + 1, paused, requests, refused, refused, 0, 0]
    );
}

/// An `identified` Session signs each request it continues (#1551): its key
/// was resolved as it opened, so signing compiles nothing and each pause is
/// still answered once (C5). A Session that is not identified signs none.
/// Covers: setting:identified.signing_key
#[test]
fn an_identified_session_signs_each_request_with_one_answer_and_nothing_compiled() {
    let standard = run_in(Browsers::Pool, Map::new(), None);

    let Counted {
        requests,
        paused,
        signed,
        work,
        ..
    } = run_in(Browsers::Pool, identified(), None);

    assert!(requests > 0, "the Session paused no request");
    assert_eq!(
        (
            standard.signed,
            signed,
            work.read(Row::InterceptedAnswers),
            work.read(Row::InterceptedCompilations),
        ),
        (0, requests, paused, 0)
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

/// In the browser pool (C7), the three pages open one after another in one
/// Session of the pool, in the one browser it launched: the pool walks that
/// browser's processes for their memory before each page but the first,
/// which it has nothing to judge by yet.
#[test]
fn a_pool_page_walks_its_browsers_memory_but_before_its_first() {
    let Counted {
        pages_opened, work, ..
    } = run_in(Browsers::Pool, Map::new(), None);

    assert_eq!((pages_opened, work.read(Row::PoolWalks)), (3, 2));
}

/// A page's opening awaits each of its commands in turn (C7): its target's
/// creation and attachment, then its own set-up (its focus, `Page.enable`,
/// its lifecycle events, `Network.enable` and its new documents' script),
/// then the Session's (the interception's `Fetch.enable`, `DOM.enable` and
/// `CSS.enable` for live hiding, the Consent prompt runner's binding and its
/// children's auto-attach). A Borrowed browser's page shows its own
/// identity, and the Session's browser context was found as it opened.
#[test]
fn a_borrowed_pages_opening_counts_each_command_it_awaits() {
    let Counted {
        pages_opened, work, ..
    } = run(None);

    assert_eq!((pages_opened, work.read(Row::PageOpeningCommands)), (1, 12));
}

/// A pool page of the headless browser takes its window's identity too
/// (`Emulation.setUserAgentOverride`), one command more; the Session's
/// browser context was made as the Session opened, before its first page.
#[test]
fn a_pool_pages_opening_counts_its_windows_identity_too() {
    let Counted {
        pages_opened, work, ..
    } = run_in(Browsers::Pool, Map::new(), None);

    assert_eq!(
        (pages_opened, work.read(Row::PageOpeningCommands)),
        (3, 3 * 13)
    );
}

/// A Session of the browser pool makes its browser context once, as it
/// opens, whatever the pages it opens; a Borrowed browser's Session finds
/// the browser's default one and makes none (C7).
#[rstest]
#[case::pool(Browsers::Pool, 1)]
#[case::borrowed(Browsers::Borrowed, 0)]
fn a_session_makes_its_browser_context_once(#[case] browsers: Browsers, #[case] made: u64) {
    let Counted {
        sessions_opened,
        work,
        ..
    } = run_in(browsers, Map::new(), None);

    assert_eq!(
        (sessions_opened, work.read(Row::ContextsCreated)),
        (1, made)
    );
}

/// A plant adds one browser context per command the browser reads.
#[test]
fn a_planted_unit_adds_one_context_a_command() {
    let unplanted = run_in(Browsers::Pool, Map::new(), None);

    let planted = run_in(Browsers::Pool, Map::new(), Some(Row::ContextsCreated));

    assert_eq!(
        planted.work.read(Row::ContextsCreated),
        unplanted.work.read(Row::ContextsCreated) + planted.commands
    );
}

/// A plant adds one unit of C7's row per command the browser reads.
#[test]
fn a_planted_unit_adds_one_an_opening_command_a_command() {
    let unplanted = run_in(Browsers::Pool, Map::new(), None);

    let planted = run_in(Browsers::Pool, Map::new(), Some(Row::PageOpeningCommands));

    assert_eq!(
        planted.work.read(Row::PageOpeningCommands),
        unplanted.work.read(Row::PageOpeningCommands) + planted.commands
    );
}
