use clap::{Arg, ArgAction, Command};

use super::unknown_options;

/// `tool [--json] run start [--run-id <id>]`, `--json` global.
fn tool() -> Command {
    Command::new("tool")
        .arg(
            Arg::new("json")
                .long("json")
                .global(true)
                .action(ArgAction::SetTrue),
        )
        .subcommand(
            Command::new("run")
                .subcommand(Command::new("start").arg(Arg::new("run-id").long("run-id"))),
        )
}

#[test]
fn an_example_s_option_its_command_lacks_is_named_with_the_command() {
    let markdown = "```console\n$ tool run start --run-id first --resume --json\n```\n";

    assert_eq!(unknown_options(&tool(), markdown), ["run start --resume"]);
}

#[test]
fn a_code_span_s_option_its_command_lacks_is_named_with_the_command() {
    let markdown =
        "Resume it with `tool run start --run-id <run-id>`, never `tool run start --again`.\n";

    assert_eq!(unknown_options(&tool(), markdown), ["run start --again"]);
}

#[test]
fn a_global_option_and_an_option_given_its_value_after_an_equals_sign_are_known() {
    let markdown = "```console\n$ tool run start --json --run-id=first\n```\n";

    assert_eq!(unknown_options(&tool(), markdown), Vec::<String>::new());
}

#[test]
fn another_program_s_commands_and_other_blocks_are_not_read() {
    let markdown = "`other run start --nope`\n\n```text\n$ tool run start --nope\n```\n\n```console\n$ other --nope\n```\n";

    assert_eq!(unknown_options(&tool(), markdown), Vec::<String>::new());
}
