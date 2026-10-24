//! Human output through Collie's executable (ADR 0073): English with `.`
//! decimals and ISO 8601, reading no locale; times in the local zone with
//! their offset, while `--json` stays UTC; `--color auto|always|never`,
//! colour that never carries meaning alone and never reaches a pipe, and
//! progress on stderr only on a terminal.

use goodfetch_test_support::{ColourProperty, TestEnv, cli, pty};
use rstest::rstest;
use serde_json::Value;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Starts the Run `run_id` with `run start`, on an Attached engine that
/// stops it as the command ends, so `run finish` reads its report.
fn start(env: &TestEnv, run_id: &str) {
    let output = cli(env, COLLIE)
        .args(["run", "start", "--run-id", run_id, "--attached"])
        .run()
        .unwrap();
    assert_eq!(output.exit, Some(0), "{output}");
}

/// The value of the human output's line `name: value`.
fn field<'a>(stdout: &'a str, name: &str) -> &'a str {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{name}: ")))
        .unwrap_or_else(|| panic!("no `{name}:` line in:\n{stdout}"))
}

/// Covers: story:11-207, command:run.finish
#[test]
fn two_time_zones_print_one_instant_with_their_offsets_and_json_prints_it_in_utc() {
    let env = TestEnv::new();
    start(&env, "first");
    let report = cli(&env, COLLIE)
        .args(["run", "finish", "first", "--json", "--attached"])
        .json("RunReport")
        .unwrap();
    let Value::String(utc) = &report.documents[0]["started_at"] else {
        panic!("no started_at in {}", report.output);
    };

    let kathmandu = cli(&env, COLLIE)
        .args(["run", "finish", "first", "--attached"])
        .with_time_zone("Asia/Kathmandu")
        .run()
        .unwrap();
    let in_utc = cli(&env, COLLIE)
        .args(["run", "finish", "first", "--attached"])
        .with_time_zone("UTC")
        .run()
        .unwrap();

    let kathmandu = field(&kathmandu.stdout, "started_at");
    let in_utc = field(&in_utc.stdout, "started_at");
    let instant = |text: &str| text.parse::<jiff::Timestamp>().unwrap();
    assert_eq!(
        (
            utc.ends_with('Z'),
            &kathmandu[kathmandu.len() - 6..],
            &in_utc[in_utc.len() - 6..],
            instant(kathmandu),
            instant(in_utc),
        ),
        (true, "+05:45", "+00:00", instant(utc), instant(utc)),
    );
}

/// Covers: story:11-207, command:run.finish
#[rstest]
#[case::de_de("de-DE")]
#[case::fr_fr("fr-FR")]
#[case::tr_tr("tr-TR")]
#[case::ar_eg("ar-EG")]
#[case::hi_in("hi-IN")]
#[case::ja_jp("ja-JP")]
fn human_output_reads_no_locale(#[case] locale: &str) {
    let env = TestEnv::new();
    start(&env, "first");
    let in_english = cli(&env, COLLIE)
        .args(["run", "finish", "first", "--attached"])
        .with_locale("en-US")
        .run()
        .unwrap();

    let elsewhere = cli(&env, COLLIE)
        .args(["run", "finish", "first", "--attached"])
        .with_locale(locale)
        .run()
        .unwrap();

    assert_eq!(
        (elsewhere.exit, elsewhere.stdout),
        (in_english.exit, in_english.stdout)
    );
}

/// Covers: story:11-208, command:engine.info
#[rstest]
#[case::no_color_empty(&[])]
#[case::color_auto(&["--color", "auto"])]
#[case::color_never(&["--color", "never"])]
fn a_command_piped_holds_no_escape_byte(#[case] flags: &[&str]) {
    let env = TestEnv::new();

    // `cli` fails with `CliError::EscapeByte` on an escape byte in either
    // pipe; an empty `NO_COLOR` leaves colour to `--color`.
    let output = cli(&env, COLLIE)
        .args(["engine", "info"])
        .args(flags)
        .env("NO_COLOR", "")
        .run()
        .unwrap();

    assert_eq!(output.exit, Some(0), "{output}");
}

/// A test's setup whose engines log no trace lines: they carry their
/// instant, which two runs on a pseudo-terminal compared with each other
/// must not hold.
fn without_trace_lines() -> TestEnv {
    let env = TestEnv::new();
    env.log_level("info");
    env
}

/// Covers: story:11-208, command:engine.info
#[rstest]
#[case::stripped_colour_equals_no_color(ColourProperty::StrippedColourEqualsNoColor)]
#[case::empty_no_color_keeps_colour(ColourProperty::EmptyNoColorKeepsColour)]
#[case::color_always_overrides_no_color(ColourProperty::ColorAlwaysOverridesNoColor)]
fn a_results_colour_never_carries_meaning_alone(#[case] property: ColourProperty) {
    let env = without_trace_lines();

    let checked = property.check(&env, COLLIE, &["engine", "info"]);

    assert!(checked.is_ok(), "{checked:?}");
}

/// Covers: story:11-208, command:run.finish
#[rstest]
#[case::stripped_colour_equals_no_color(ColourProperty::StrippedColourEqualsNoColor)]
#[case::empty_no_color_keeps_colour(ColourProperty::EmptyNoColorKeepsColour)]
#[case::color_always_overrides_no_color(ColourProperty::ColorAlwaysOverridesNoColor)]
fn a_failures_colour_never_carries_meaning_alone(#[case] property: ColourProperty) {
    let env = without_trace_lines();

    let checked = property.check(&env, COLLIE, &["run", "finish", "missing", "--attached"]);

    assert!(checked.is_ok(), "{checked:?}");
}

/// Covers: story:11-208, command:engine.info
#[test]
fn progress_shows_on_a_terminal_and_is_erased_before_the_result() {
    let env = without_trace_lines();

    let output = pty(&env, COLLIE).args(["engine", "info"]).run().unwrap();

    assert_eq!(
        (output.exit, output.text.contains("engine info…\r\u{1b}[2K")),
        (Some(0), true),
        "{output}"
    );
}

/// Covers: story:11-208, command:engine.info
#[test]
fn progress_never_reaches_a_pipe() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE).args(["engine", "info"]).run().unwrap();

    assert_eq!(
        (
            output.exit,
            output.stdout.contains('…'),
            output.stderr.contains('…')
        ),
        (Some(0), false, false),
        "{output}"
    );
}
