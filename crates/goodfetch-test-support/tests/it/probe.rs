//! Tests that other tests run as child processes, to watch what a test
//! using `TestEnv` shows when it hangs or fails, and to stand in for an
//! engine speaking its protocol. They are ignored, so they run only as those
//! children.

use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use goodfetch_test_support::{
    JourneyRun, ObservationLog, Operation, Record, Redacted, TestEnv, Tool, tools,
};

#[test]
#[ignore = "run as a child by seed::seed_line_is_printed_before_the_test_body_runs"]
fn hangs_after_creating_a_test_env() {
    let _env = TestEnv::new();

    // The parent kills it long before this ends.
    std::thread::sleep(Duration::from_secs(600));
}

#[test]
#[ignore = "run as a child by teardown::a_kept_folder_is_listed_on_failure"]
fn fails_with_a_test_env() {
    let _env = TestEnv::new();

    panic!("the probe fails on purpose");
}

#[test]
#[ignore = "run as a child by teardown::an_in_memory_folder_of_a_failed_test_is_kept_on_disk"]
fn fails_with_an_in_memory_test_env() {
    let env = TestEnv::in_memory();
    std::fs::write(env.folder().join("evidence.txt"), "kept").unwrap();

    panic!("the probe fails on purpose");
}

#[test]
#[ignore = "run as a child by seeded_kill's tests"]
fn sends_protocol_events_then_hangs() {
    for n in 1..=3 {
        for method in ["delivery.begin", "delivery.commit"] {
            let message =
                serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": { "n": n } });
            #[expect(
                clippy::print_stdout,
                reason = "the probe speaks newline-delimited JSON-RPC on its stdout, as an engine does"
            )]
            {
                println!("{message}");
            }
        }
    }

    // The parent kills it long before this ends.
    std::thread::sleep(Duration::from_secs(600));
}

/// The probe's record of each event it sent, before it sent it, in the
/// folder its parent gives it as its temporary folder.
pub(crate) const SENT: &str = "sent.log";

#[test]
#[ignore = "run as a child by seeded_kill::no_event_after_the_moment_is_sent"]
#[expect(
    clippy::disallowed_methods,
    reason = "the probe paces its events and records them, as a slow engine would"
)]
fn sends_protocol_events_slowly_then_hangs() {
    let sent = std::env::temp_dir().join(SENT);
    for n in 1..=3 {
        for method in ["delivery.begin", "delivery.commit"] {
            let mut log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&sent)
                .unwrap();
            writeln!(log, "{method} {n}").unwrap();
            let message =
                serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": { "n": n } });
            #[expect(
                clippy::print_stdout,
                reason = "the probe speaks newline-delimited JSON-RPC on its stdout, as an engine does"
            )]
            {
                println!("{message}");
            }
            // Time for a kill at this event to land before the next.
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    // The parent kills it long before this ends.
    std::thread::sleep(Duration::from_secs(600));
}

#[test]
#[ignore = "run as a child by exercise_record::the_test_subscriber_writes_the_dispatch_and_outcome_records_into_the_exercise_record"]
fn dispatches_under_the_test_subscriber() {
    let start = TestEnv::new().start_time();
    let subscriber = goodfetch_test_support::subscriber(move || start);

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "goodfetch::engine", "engine started");
        tracing::trace!(target: "goodfetch::rpc::dispatch", method = "engine.info", "request answered");
        tracing::trace!(target: "goodfetch::run::outcome", error_code = "handler_lost", "the run recorded an error code");
    });
}

/// A test's own operation, carrying a security value.
#[derive(serde::Serialize)]
struct SignIn {
    token: Redacted<String>,
}

#[test]
#[ignore = "run as a child by observation_log::a_failed_test_writes_its_observation_log_in_its_kept_folder"]
fn fails_with_an_observation_log() {
    let env = TestEnv::new();
    let mut log = ObservationLog::for_test(&env);
    log.push(Record::new("engine", Operation::Fence { epoch: 1 }));
    log.push(Record::new(
        "client",
        Operation::Other(SignIn {
            token: Redacted::new("gf_token_5f1e".to_owned()),
        }),
    ));

    panic!("the probe fails on purpose");
}

#[test]
#[ignore = "run as a child by journey_run::the_test_seed_variable_does_not_change_the_journeys_seed"]
fn runs_journey_902() {
    let journey =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/fixtures/journey-run/J-902.toml");
    let foxhound = tools().unwrap().path(Tool::Foxhound).unwrap().to_owned();
    let run = JourneyRun::start_file(&journey, "cli", &foxhound, |facts: serde_json::Value| facts)
        .unwrap();

    let _finished = run.finish().unwrap();
}

#[test]
#[ignore = "run as a child by cli::an_escape_byte_in_piped_output_fails_the_run"]
fn prints_an_escape_byte_to_stdout() {
    #[expect(
        clippy::print_stdout,
        reason = "the probe stands in for a command that leaks colour into a pipe"
    )]
    {
        println!("\u{1b}[31mred\u{1b}[0m");
    }
}

#[test]
#[ignore = "run as a child by cli::an_escape_byte_in_piped_output_fails_the_run"]
fn prints_an_escape_byte_to_stderr() {
    #[expect(
        clippy::print_stderr,
        reason = "the probe stands in for a command that leaks colour into a pipe"
    )]
    {
        eprintln!("\u{1b}[31mred\u{1b}[0m");
    }
}

#[test]
#[ignore = "run as a child by cli::a_clean_run_snapshots_its_exit_code_stdout_and_stderr_as_one_value"]
fn prints_to_both_streams_then_exits_with_3() {
    #[expect(
        clippy::print_stdout,
        clippy::print_stderr,
        reason = "the probe stands in for a command writing to both of its streams"
    )]
    {
        println!("out");
        eprintln!("err");
    }
    std::io::stdout().flush().unwrap();
    #[expect(
        clippy::exit,
        reason = "the probe stands in for a command that ends with its own exit code"
    )]
    std::process::exit(3);
}

#[test]
#[ignore = "run as a child by cli::teardown_copies_the_command_record_into_the_tests_exercise_record"]
fn records_a_command_then_drops_its_test_env() {
    let env = TestEnv::new();

    goodfetch_test_support::record_command(&env, &["browser", "install"], Some(0)).unwrap();
}

#[test]
#[ignore = "run as a child by journey_run::a_skew_run_drives_the_engine_variable_in_place_of_the_executable_passed"]
fn finishes_journey_901_on_collie() {
    let journey =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/fixtures/journey-run/J-901.toml");
    let collie = tools().unwrap().path(Tool::Collie).unwrap().to_owned();
    let run = JourneyRun::start_file(&journey, "cli", &collie, |mut facts: serde_json::Value| {
        facts["outcome"] = serde_json::json!("failed");
        facts
    })
    .unwrap();

    let _finished = run.finish().unwrap();
}

#[test]
#[ignore = "run as a child by journey_run::a_runs_document_stays_its_own_when_another_run_of_its_variant_finishes_after_it"]
fn runs_journey_901_twice() {
    let journey =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/fixtures/journey-run/J-901.toml");
    let collie = tools().unwrap().path(Tool::Collie).unwrap().to_owned();
    let start = |outcome: &'static str| {
        JourneyRun::start_file(
            &journey,
            "cli",
            &collie,
            move |mut facts: serde_json::Value| {
                facts["outcome"] = serde_json::json!(outcome);
                facts
            },
        )
        .unwrap()
    };
    let (first, second) = (start("proved"), start("failed"));

    let first = first.finish().unwrap();
    let _second = second.finish().unwrap();

    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(first.path()).unwrap()).unwrap();
    assert_eq!(written["outcome"], serde_json::json!("proved"));
}

/// What a fixture command's colour depends on: `--color always` among its
/// arguments, its `NO_COLOR` and whether its stdout is a terminal.
struct ColourInputs {
    always: bool,
    no_color: Option<std::ffi::OsString>,
    terminal: bool,
}

fn colour_inputs() -> ColourInputs {
    use std::io::IsTerminal as _;

    let arguments: Vec<String> = std::env::args().collect();
    ColourInputs {
        always: arguments
            .windows(2)
            .any(|pair| pair[0] == "--color" && pair[1] == "always"),
        no_color: std::env::var_os("NO_COLOR"),
        terminal: std::io::stdout().is_terminal(),
    }
}

/// Prints `line`, flushes, and exits before libtest prints its own
/// verdict, which it colours on a terminal.
#[expect(
    clippy::exit,
    reason = "the probe stands in for a command and ends before libtest's coloured verdict"
)]
fn print_and_exit(line: &str, code: i32) -> ! {
    #[expect(
        clippy::print_stdout,
        reason = "the probe stands in for a command writing its output"
    )]
    {
        println!("{line}");
    }
    std::io::stdout().flush().unwrap();
    std::process::exit(code);
}

#[test]
#[ignore = "run as a child by pty's tests"]
fn keeps_the_colour_properties() {
    let inputs = colour_inputs();
    let no_color = inputs.no_color.is_some_and(|value| !value.is_empty());
    let colour = inputs.always || (!no_color && inputs.terminal);

    print_and_exit(
        if colour {
            "status: \u{1b}[32mready\u{1b}[0m"
        } else {
            "status: ready"
        },
        0,
    );
}

#[test]
#[ignore = "run as a child by pty::a_command_that_breaks_a_colour_property_fails_it"]
fn says_failed_only_in_colour() {
    let inputs = colour_inputs();
    let no_color = inputs.no_color.is_some_and(|value| !value.is_empty());
    let colour = inputs.always || (!no_color && inputs.terminal);

    print_and_exit(
        if colour {
            "status: \u{1b}[31mready\u{1b}[0m"
        } else {
            "status: ready (failed)"
        },
        0,
    );
}

#[test]
#[ignore = "run as a child by pty::a_command_that_breaks_a_colour_property_fails_it"]
fn turns_colour_off_under_an_empty_no_color() {
    let inputs = colour_inputs();
    let colour = inputs.always || (inputs.no_color.is_none() && inputs.terminal);

    print_and_exit(
        if colour {
            "status: \u{1b}[32mready\u{1b}[0m"
        } else {
            "status: ready"
        },
        0,
    );
}

#[test]
#[ignore = "run as a child by pty::a_command_that_breaks_a_colour_property_fails_it"]
fn lets_no_color_beat_color_always() {
    let inputs = colour_inputs();
    let no_color = inputs.no_color.is_some_and(|value| !value.is_empty());
    let colour = !no_color && (inputs.always || inputs.terminal);

    print_and_exit(
        if colour {
            "status: \u{1b}[32mready\u{1b}[0m"
        } else {
            "status: ready"
        },
        0,
    );
}

#[test]
#[ignore = "run as a child by pty::a_command_that_never_colours_fails_the_stripped_colour_property"]
fn never_colours() {
    print_and_exit("status: ready", 0);
}

#[test]
#[ignore = "run as a child by pty::a_prompt_is_answered_through_a_pseudo_terminal"]
fn prompts_for_an_answer() {
    use std::io::{BufRead as _, IsTerminal as _};

    if !std::io::stdin().is_terminal() {
        print_and_exit("refused: stdin is not a terminal", 1);
    }
    #[expect(
        clippy::print_stdout,
        reason = "the probe stands in for a command asking a person"
    )]
    {
        print!("Accept the certificate? [y/N] ");
    }
    std::io::stdout().flush().unwrap();
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer).unwrap();

    if answer.trim() == "y" {
        print_and_exit("accepted", 0);
    }
    print_and_exit("declined", 1);
}

#[test]
#[ignore = "run as a child by pty::a_prompt_that_never_shows_fails_the_wait"]
fn ends_without_a_prompt() {
    print_and_exit("nothing to ask", 0);
}

/// The variable an executable built for coverage sets in its own
/// environment as its profile runtime starts (`just coverage`, ADR 0072): no
/// parent passes it, so the probe leaves it out of the environment it shows.
const PROFILE_RUNTIME_VARIABLE: &str = "__LLVM_PROFILE_RT_INIT_ONCE";

#[test]
#[ignore = "run as a child by cli::a_locale_case_sets_lang_and_tz_only_for_the_child_it_runs"]
fn prints_its_environment() {
    let lines: Vec<String> = std::env::vars()
        .filter(|(name, _)| name != PROFILE_RUNTIME_VARIABLE)
        .map(|(name, value)| format!("env:{name}={value}"))
        .collect();

    print_and_exit(&lines.join("\n"), 0);
}
