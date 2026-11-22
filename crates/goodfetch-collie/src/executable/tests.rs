#![expect(
    clippy::disallowed_types,
    reason = "the log writer test shares what the subscriber wrote with the test through a Mutex"
)]

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use super::output::ColourChoice;
use clap::FromArgMatches as _;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::daemon::service::LaunchedCommand;
use goodfetch_engine::mask::{Masks, Reach, Redaction};
use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::{
    certificate_verification_switches, command_help, command_paths, long_flags,
};
use rstest::rstest;
use secrecy::SecretString;
use tracing_subscriber::layer::SubscriberExt as _;

use super::catalog_commands::members;
use super::daemon_command::DaemonCommand;
use super::log_file::{LogFile, Rotation};
use super::logs::Masked;
use super::{COLOR, Command, CommandLine, OwnCommands, attached_engine, command, command_line};

/// Foxhound's catalog's generated commands, none written by hand: Foxhound's
/// executable's command line but for its own commands, which Foxhound's
/// crate holds and tests.
const FOXHOUNDS_CATALOG: OwnCommands = OwnCommands {
    commands: &[],
    members: foxhounds_catalog_members,
};

pub(super) fn foxhounds_catalog_members(group: clap::Command) -> clap::Command {
    members(group, Product::Foxhound, &[])
}

/// The commands `product`'s executable writes by hand, here.
fn own(product: Product) -> OwnCommands {
    match product {
        Product::Collie => OwnCommands::COLLIE,
        Product::Foxhound => FOXHOUNDS_CATALOG,
    }
}

/// A start builds no group's commands, hand-written or generated: clap
/// builds them only for the group it parses or shows help for, so
/// `--version` and every command skip building the rest of the tree (ADR
/// 0071's startup figure).
#[test]
fn a_start_builds_no_command_of_any_group() {
    let (command_line, _) = command_line(Product::Collie, OwnCommands::COLLIE);

    let built: Vec<(&str, Vec<&str>)> = command_line
        .get_subcommands()
        .map(|group| {
            let commands: Vec<&str> = group
                .get_subcommands()
                .map(clap::Command::get_name)
                .collect();
            (group.get_name(), commands)
        })
        .filter(|(_, commands)| !commands.is_empty())
        .collect();

    assert_eq!(built, Vec::<(&str, Vec<&str>)>::new());
}

/// Covers: story:11-198, story:11-218
#[test]
fn the_help_of_every_command_is_rendered_from_the_command_line() {
    let command = command(Product::Collie, OwnCommands::COLLIE);

    let tree = std::iter::once(Vec::new())
        .chain(command_paths(&command))
        .fold(String::new(), |mut tree, path| {
            let words: Vec<&str> = std::iter::once("goodfetch-collie")
                .chain(path.iter().map(String::as_str))
                .chain(std::iter::once("--help"))
                .collect();
            let _ = writeln!(
                tree,
                "$ {}\n{}",
                words.join(" "),
                command_help(&command, &path)
            );
            tree
        });

    insta::assert_snapshot!("help_tree", tree);
}

/// `--color` is taken before the command, after a group and after a
/// command, of the hand-written commands and the generated ones alike; given
/// twice, the later one wins.
#[rstest]
#[case::none(&["daemon", "status"], ColourChoice::Auto)]
#[case::before_a_hand_written_command(&["--color", "never", "daemon", "status"], ColourChoice::Never)]
#[case::after_a_hand_written_group(&["daemon", "--color", "never", "status"], ColourChoice::Never)]
#[case::after_a_hand_written_command(&["daemon", "status", "--color", "never"], ColourChoice::Never)]
#[case::after_a_command_without_a_group(&["upgrade-files", "--color", "never"], ColourChoice::Never)]
#[case::before_a_generated_command(&["--color", "never", "engine", "info"], ColourChoice::Never)]
#[case::after_a_generated_group(&["engine", "--color", "never", "info"], ColourChoice::Never)]
#[case::after_a_generated_command(&["engine", "info", "--color", "never"], ColourChoice::Never)]
#[case::the_later_of_two(&["--color", "always", "engine", "info", "--color", "never"], ColourChoice::Never)]
fn color_is_taken_wherever_it_is_given(
    #[values(Product::Collie, Product::Foxhound)] product: Product,
    #[case] arguments: &[&str],
    #[case] expected: ColourChoice,
) {
    let (command_line, _) = command_line(product, own(product));

    let matches = command_line
        .try_get_matches_from(std::iter::once("goodfetch").chain(arguments.iter().copied()))
        .unwrap();

    assert_eq!(
        matches.get_one::<ColourChoice>(COLOR).copied(),
        Some(expected)
    );
}

/// An Attached engine started as an SDK starts it is recognised before any
/// command line is built (#2204), as the command line parses it, its colour
/// included; anything else it is given is left to the command line.
#[rstest]
#[case::as_an_sdk_starts_it(&["attached-engine"], Some(ColourChoice::Auto))]
#[case::coloured(&["attached-engine", "--color", "always"], Some(ColourChoice::Always))]
#[case::coloured_in_one_argument(&["attached-engine", "--color=never"], Some(ColourChoice::Never))]
#[case::coloured_before_it(&["--color", "never", "attached-engine"], None)]
#[case::its_help(&["attached-engine", "--help"], None)]
#[case::a_flag_it_lacks(&["attached-engine", "--json"], None)]
#[case::a_colour_none_names(&["attached-engine", "--color", "pink"], None)]
#[case::another_command(&["daemon", "status"], None)]
fn an_attached_engine_is_recognised_before_the_command_line_is_built(
    #[values(Product::Collie, Product::Foxhound)] product: Product,
    #[case] arguments: &[&str],
    #[case] expected: Option<ColourChoice>,
) {
    let arguments: Vec<std::ffi::OsString> = std::iter::once("goodfetch")
        .chain(arguments.iter().copied())
        .map(std::ffi::OsString::from)
        .collect();

    let recognised = attached_engine(&arguments);

    let (command_line, _) = command_line(product, own(product));
    let parsed = command_line
        .try_get_matches_from(&arguments)
        .ok()
        .and_then(|matches| {
            let colour = matches.get_one::<ColourChoice>(COLOR).copied()?;
            match CommandLine::from_arg_matches(&matches).ok()?.command {
                Command::AttachedEngine { .. } => Some(colour),
                _ => None,
            }
        });
    assert_eq!((recognised, expected.and(parsed)), (expected, expected));
}

/// `daemon launched`'s arguments as `product`'s command line reads them:
/// the build of its copy, whether it keeps alive, whether the service
/// starts it and the variables it is passed.
fn launched(
    product: Product,
    launched: &LaunchedCommand<'_>,
) -> (Option<String>, bool, bool, Vec<String>) {
    let (command_line, _) = command_line(product, own(product));
    let matches = command_line
        .try_get_matches_from(std::iter::once("goodfetch".to_owned()).chain(launched.arguments()))
        .unwrap();
    match CommandLine::from_arg_matches(&matches).unwrap().command {
        Command::Daemon {
            command:
                DaemonCommand::Launched {
                    copy,
                    keep_alive,
                    service,
                    environment,
                },
            ..
        } => (copy, keep_alive, service, environment),
        _ => panic!("the arguments name `daemon launched`"),
    }
}

/// An installed service's definition holds these arguments, so a flag the
/// command line renames would stop the service at its next start.
#[rstest]
#[case::collie(Product::Collie)]
#[case::foxhound(Product::Foxhound)]
fn the_daemon_launched_command_is_one_the_command_line_reads(#[case] product: Product) {
    let copy = Some("0.1.0-abc".to_owned());
    let passed = "GOODFETCH_HOME=/home/someone/gf".to_owned();

    assert_eq!(
        launched(
            product,
            &LaunchedCommand::of_service("0.1.0-abc", vec![passed.clone()]),
        ),
        (copy.clone(), false, true, vec![passed]),
    );
    assert_eq!(
        launched(product, &LaunchedCommand::started("0.1.0-abc", true)),
        (copy, true, false, Vec::new()),
    );
}

/// Story 4-31: no flag of Collie's executable turns certificate verification
/// off, as Foxhound's crate tests of Foxhound's. No flag reads a variable
/// either: clap's `env` feature is off.
/// Covers: story:4-31
#[test]
fn no_flag_switches_certificate_verification_off() {
    let flags = long_flags(&command(Product::Collie, OwnCommands::COLLIE));

    assert!(
        flags.iter().any(|flag| flag == COLOR),
        "the enumeration reaches the executable's flags"
    );
    assert_eq!(
        certificate_verification_switches(&flags),
        Vec::<String>::new()
    );
}

/// What a log writer was given, shared with the test.
#[derive(Clone, Default)]
struct Written(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Written {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Every log line a writer of the process's subscriber writes passes
/// through the engine's masks as a kept output (spec #564): the value a
/// Run resolved is written as `%name%`, raw and encoded, and a Run whose
/// `security.redaction` is off keeps its values in the logs.
/// Covers: story:4-56, story:4-76
#[rstest]
#[case::redaction_on(Redaction::On, "the page said %proxy.password% and %proxy.password%")]
#[case::redaction_off(
    Redaction::Off,
    "the page said pa ss\"w/rd>?9~ and 706120737322772f72643e3f397e"
)]
fn a_log_line_holds_a_resolved_value_only_when_its_runs_redaction_is_off(
    #[case] redaction: Redaction,
    #[case] logged: &str,
) {
    let masks = Masks::new();
    let mask = masks.for_run(redaction);
    mask.join(
        "proxy.password",
        &SecretString::from("pa ss\"w/rd>?9~"),
        Reach::Value,
    );
    let written = Written::default();
    let writer = written.clone();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .without_time()
            .with_ansi(false)
            .with_target(false)
            .with_level(false)
            .with_writer(Masked::new(masks, move || writer.clone())),
    );

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(
            "the page said {} and {}",
            "pa ss\"w/rd>?9~",
            "706120737322772f72643e3f397e"
        );
    });

    assert_eq!(
        String::from_utf8(written.0.lock().unwrap().clone()).unwrap(),
        format!("{logged}\n")
    );
}

/// Each write a log writer was given, apart, shared with the test.
#[derive(Clone, Default)]
struct Writes(Arc<Mutex<Vec<Vec<u8>>>>);

impl std::io::Write for Writes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().push(bytes.to_vec());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Writes {
    /// The writes the writer was handed, in order, each as text.
    fn handed(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|write| String::from_utf8(write.clone()).unwrap())
            .collect()
    }
}

/// The log formats' redaction with values hard to redact (TESTING.md,
/// Secrets): a value and a longer one holding it, each logged in the
/// message and as a field, Display and Debug, through the text and the
/// JSON format, leave no form a Secret canary search finds.
///
/// Covers: story:4-91
#[rstest]
#[case::quotes_backslashes_and_percent(r#"a"b\c%d%2"#)]
#[case::a_newline("first\nsecond")]
#[case::unicode("pässwört-日本語-🔑")]
#[case::a_common_word("password")]
#[case::very_long(&"x\"y\\z%".repeat(40))]
fn no_form_of_a_hard_value_passes_either_log_format(
    #[case] value: &str,
    #[values(false, true)] json: bool,
) {
    let outer = format!("<{value}>");
    let masks = Masks::new();
    let mask = masks.for_run(Redaction::On);
    mask.join("inner", &SecretString::from(value), Reach::Value);
    mask.join("outer", &SecretString::from(outer.as_str()), Reach::Value);
    let written = Written::default();
    let writer = written.clone();
    let layer = tracing_subscriber::fmt::layer()
        .without_time()
        .with_ansi(false)
        .with_writer(Masked::new(masks, move || writer.clone()));
    let log = || {
        tracing::info!(inner = value, outer = %outer, "said {value} and {outer:?}");
    };
    if json {
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer.json()), log);
    } else {
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), log);
    }

    let mut canaries = goodfetch_test_support::Canaries::new(0);
    canaries.add(goodfetch_test_support::Canary::known(
        "inner",
        goodfetch_test_support::SecretKind::TypedPassword,
        value,
    ));
    canaries.add(goodfetch_test_support::Canary::known(
        "outer",
        goodfetch_test_support::SecretKind::TypedPassword,
        &outer,
    ));
    let written = written.0.lock().unwrap().clone();
    let leaks = canaries
        .leaks(&[goodfetch_test_support::Output::Bytes {
            name: "log",
            bytes: &written,
        }])
        .unwrap();
    assert_eq!(leaks, Vec::new(), "{}", String::from_utf8_lossy(&written));
}

/// A log line the Mask rewrites reaches the writer below as one write, the
/// whole masked line: the log file takes each write as a line of its own,
/// so a line split into several writes would tear.
///
/// Covers: story:4-56, row:RT-17
#[test]
fn a_log_line_the_mask_rewrites_reaches_its_writer_in_one_write() {
    let masks = Masks::new();
    let mask = masks.for_run(Redaction::On);
    mask.join(
        "proxy.password",
        &SecretString::from("pa ss\"w/rd>?9~"),
        Reach::Value,
    );
    let given = Writes::default();
    let writer = given.clone();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .json()
            .without_time()
            .with_target(false)
            .with_level(false)
            .with_writer(Masked::new(masks, move || writer.clone())),
    );

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(proxy = "pa ss\"w/rd>?9~", "the proxy answered");
    });

    assert_eq!(
        given.handed(),
        vec![
            "{\"fields\":{\"message\":\"the proxy answered\",\"proxy\":\"%proxy.password%\"}}\n"
                .to_owned()
        ]
    );
}

/// Lines a thread logs one after another, masked or not, longer or
/// shorter than the one before, each reach the writer below as that line
/// alone: nothing of an earlier line is left in a later one.
///
/// Covers: story:4-56, row:RT-17
#[test]
fn lines_logged_one_after_another_each_reach_their_writer_as_that_line_alone() {
    let masks = Masks::new();
    let mask = masks.for_run(Redaction::On);
    mask.join(
        "proxy.password",
        &SecretString::from("pa ss\"w/rd>?9~"),
        Reach::Value,
    );
    let given = Writes::default();
    let writer = given.clone();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .without_time()
            .with_ansi(false)
            .with_target(false)
            .with_level(false)
            .with_writer(Masked::new(masks, move || writer.clone())),
    );

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!("the proxy answered for {} at last", "pa ss\"w/rd>?9~");
        tracing::info!("short");
        tracing::info!("{} again", "pa ss\"w/rd>?9~");
    });

    assert_eq!(
        given.handed(),
        vec![
            "the proxy answered for %proxy.password% at last\n".to_owned(),
            "short\n".to_owned(),
            "%proxy.password% again\n".to_owned(),
        ]
    );
}

/// Lines that threads log at once into the log file, each holding a value
/// the Mask rewrites, are each one whole JSON line there, masked: none is
/// cut or interleaved with another.
///
/// Covers: story:4-56, row:RT-17
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test logs from OS threads of its own and reads the file the writer wrote, outside any async worker"
)]
fn masked_lines_logged_at_once_from_many_threads_are_each_one_json_line_in_the_log_file() {
    const THREADS: usize = 8;
    const LINES: usize = 200;
    let env = goodfetch_test_support::TestEnv::new();
    let folder = env.folder().join("logs");
    let file = LogFile::open(
        &folder,
        "collie-1",
        Rotation {
            max_size: 1 << 30,
            max_files: 1,
        },
        Clock::system(),
    )
    .unwrap();
    let masks = Masks::new();
    let mask = masks.for_run(Redaction::On);
    mask.join(
        "proxy.password",
        &SecretString::from("pa ss\"w/rd>?9~"),
        Reach::Value,
    );
    let dispatch = tracing::Dispatch::new(
        tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(Masked::new(masks, file.clone())),
        ),
    );

    std::thread::scope(|scope| {
        for thread in 0..THREADS {
            let dispatch = dispatch.clone();
            scope.spawn(move || {
                tracing::dispatcher::with_default(&dispatch, || {
                    for line in 0..LINES {
                        tracing::info!(
                            thread,
                            line,
                            proxy = "pa ss\"w/rd>?9~",
                            "the proxy answered"
                        );
                    }
                });
            });
        }
    });
    file.flush();

    let text = std::fs::read_to_string(folder.join("collie-1.jsonl")).unwrap();
    let unreadable: Vec<&str> = text
        .lines()
        .filter(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .map_or(true, |value| value["fields"]["proxy"] != "%proxy.password%")
        })
        .collect();
    assert_eq!(
        (text.lines().count(), unreadable),
        (THREADS * LINES, Vec::<&str>::new())
    );
}

/// A WebSocket crate's `trace` records, which hold each frame's payload and
/// so an Exposed Daemon's token, never reach the executable's logs, though
/// `log.level` asks for `trace` and names those targets (ADR 0067); its
/// `debug` records and every other target's `trace` records still do. A
/// `log` record a bridge hands over is filtered under the same target, as
/// the Rust SDK's `frame_logs` test shows over a live WebSocket. The test
/// installs the process's global subscriber, so it runs in a process of
/// its own, as `just test` runs every test.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the file the subscriber wrote, outside any async worker"
)]
fn the_executables_logs_never_hold_a_websocket_frames_payload_at_trace() {
    const TOKEN: &str = "endpoint-token-canary-3209c0ffee51";
    let env = goodfetch_test_support::TestEnv::new();
    let folder = env.folder().join("logs");
    let file = LogFile::open(
        &folder,
        "collie-1",
        Rotation {
            max_size: 1 << 30,
            max_files: 1,
        },
        Clock::system(),
    )
    .unwrap();
    let installed = super::logs::install(super::logs::Logs {
        filter: "trace,tungstenite=trace,tokio_tungstenite=trace".to_owned(),
        file: Some(file),
        to: super::logs::LogsTo::Stderr,
        settings: None,
        events: &goodfetch_engine::log::LogEvents::new(),
        masks: &Masks::new(),
        product: Product::Collie,
        handles: super::logs::Handles {
            clock: Clock::system(),
            trace_ids: None,
        },
    });

    tracing::trace!(target: "tungstenite::protocol::frame", "writing frame {TOKEN}");
    tracing::trace!(target: "tokio_tungstenite::compat", "Write.write {TOKEN}");
    tracing::debug!(target: "tungstenite::handshake::server", "Server handshake done.");
    tracing::trace!(target: "goodfetch_engine::rpc", "a trace record of the engine's own");
    drop(installed);

    let text = std::fs::read_to_string(folder.join("collie-1.jsonl")).unwrap();
    assert_eq!(
        [
            text.contains(TOKEN),
            text.contains("Server handshake done."),
            text.contains("a trace record of the engine's own"),
        ],
        [false, true, true],
        "{text}"
    );
}
