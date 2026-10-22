//! The CLI helper's pseudo-terminal runs (ADRs 0069 and 0073): `pty(&env,
//! executable)` shows a fixture command's colour where a pipe does not,
//! answers a fixture prompt, records its command, and runs the colour
//! property cases, which pass on a fixture command that keeps each property
//! and fail on one that breaks it.

use goodfetch_test_support::{
    COMMAND_RECORD, ColourError, ColourProperty, PtyError, Tool, assert_cli_snapshot, cli, pty,
    tools,
};
use rstest::rstest;
use serde_json::json;

use crate::cli::{json_lines, probe_arguments, unrecorded};

fn this_binary() -> std::path::PathBuf {
    std::env::current_exe().unwrap()
}

#[test]
fn a_fixture_commands_colour_appears_on_a_pseudo_terminal() {
    let env = unrecorded();

    let output = pty(&env, this_binary())
        .args(probe_arguments("probe::keeps_the_colour_properties"))
        .run()
        .unwrap();

    assert_eq!(
        (output.exit, output.has_colour()),
        (Some(0), true),
        "{output}"
    );
}

#[test]
fn a_fixture_commands_colour_stays_out_of_a_pipe() {
    let env = unrecorded();

    // An empty NO_COLOR leaves colour on, so only the pipe turns it off.
    let output = cli(&env, this_binary())
        .args(probe_arguments("probe::keeps_the_colour_properties"))
        .env("NO_COLOR", "")
        .run()
        .unwrap();

    assert_cli_snapshot!(output, @r"
    exit: 0
    ----- stdout -----

    running 1 test
    test probe::keeps_the_colour_properties ... status: ready
    ----- stderr -----
    ");
}

#[test]
fn a_prompt_is_answered_through_a_pseudo_terminal() {
    let env = unrecorded();
    let mut run = pty(&env, this_binary())
        .args(probe_arguments("probe::prompts_for_an_answer"))
        .spawn()
        .unwrap();
    run.expect("[y/N] ").unwrap();

    run.send_line("y").unwrap();

    let output = run.finish().unwrap();
    assert_eq!(
        // A console on Windows echoes the answer; a Unix one here does not.
        (output.exit, output.stripped().ends_with("accepted\n")),
        (Some(0), true),
        "{output}"
    );
}

#[test]
fn a_prompt_that_never_shows_fails_the_wait() {
    let env = unrecorded();
    let mut run = pty(&env, this_binary())
        .args(probe_arguments("probe::ends_without_a_prompt"))
        .spawn()
        .unwrap();

    let waited = run.expect("[y/N] ");

    let error = waited.unwrap_err();
    assert!(matches!(error, PtyError::NotShown { .. }), "{error}");
}

#[test]
fn a_pseudo_terminal_run_appends_its_command_to_the_record() {
    let env = unrecorded();
    let collie = tools().unwrap().path(Tool::Collie).unwrap().to_owned();

    let output = pty(&env, collie)
        .args(["config", "explain", "timeout"])
        .run()
        .unwrap();

    assert_eq!(
        (output.exit, json_lines(&env.folder().join(COMMAND_RECORD))),
        (
            Some(0),
            vec![json!({
                "target": "goodfetch::test::command",
                "fields": { "command": "config.explain", "exit_code": 0 },
            })]
        )
    );
}

#[rstest]
#[case::stripped_colour_equals_no_color(ColourProperty::StrippedColourEqualsNoColor)]
#[case::empty_no_color_keeps_colour(ColourProperty::EmptyNoColorKeepsColour)]
#[case::color_always_overrides_no_color(ColourProperty::ColorAlwaysOverridesNoColor)]
fn a_command_that_keeps_a_colour_property_passes_it(#[case] property: ColourProperty) {
    let env = unrecorded();

    let checked = property.check(
        &env,
        this_binary(),
        &probe_arguments("probe::keeps_the_colour_properties"),
    );

    checked.unwrap();
}

/// Which colour property failed, by its error's variant.
fn broken(error: &ColourError) -> Option<ColourProperty> {
    match error {
        ColourError::ColourCarriesInformation { .. } => {
            Some(ColourProperty::StrippedColourEqualsNoColor)
        }
        ColourError::EmptyNoColorTurnsColourOff { .. } => {
            Some(ColourProperty::EmptyNoColorKeepsColour)
        }
        ColourError::ColorAlwaysIgnored { .. } => Some(ColourProperty::ColorAlwaysOverridesNoColor),
        _ => None,
    }
}

#[test]
fn a_command_that_never_colours_fails_the_stripped_colour_property() {
    let env = unrecorded();

    let checked = ColourProperty::StrippedColourEqualsNoColor.check(
        &env,
        this_binary(),
        &probe_arguments("probe::never_colours"),
    );

    let error = checked.unwrap_err();
    assert!(
        matches!(error, ColourError::NoColourShown { .. }),
        "{error}"
    );
}

#[rstest]
#[case::stripped_colour_equals_no_color(
    ColourProperty::StrippedColourEqualsNoColor,
    "probe::says_failed_only_in_colour"
)]
#[case::empty_no_color_keeps_colour(
    ColourProperty::EmptyNoColorKeepsColour,
    "probe::turns_colour_off_under_an_empty_no_color"
)]
#[case::color_always_overrides_no_color(
    ColourProperty::ColorAlwaysOverridesNoColor,
    "probe::lets_no_color_beat_color_always"
)]
fn a_command_that_breaks_a_colour_property_fails_it(
    #[case] property: ColourProperty,
    #[case] probe: &str,
) {
    let env = unrecorded();

    let checked = property.check(&env, this_binary(), &probe_arguments(probe));

    let error = checked.unwrap_err();
    assert_eq!(broken(&error), Some(property), "{error}");
}
