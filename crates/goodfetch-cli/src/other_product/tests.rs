use std::ffi::OsString;

use clap::Command;
use rstest::rstest;

use super::{Named, foxhound_only};

/// A command line that has a `run` group, as Collie's executable does.
fn collie() -> Command {
    Command::new("goodfetch-collie").subcommand(Command::new("run"))
}

fn named(name: &str, operation: bool) -> Named {
    Named {
        name: name.to_owned(),
        operation,
    }
}

#[rstest]
#[case::an_operations_command(&["crawl", "start"], Some(named("crawl.start", true)))]
#[case::in_a_namespace_both_carry(&["run", "add-seed", "--json"], Some(named("run.add_seed", true)))]
#[case::after_the_colour_flag(&["--color", "never", "crawl-request", "push"], Some(named("crawl_request.push", true)))]
#[case::a_namespace_alone(&["crawl"], Some(named("crawl", false)))]
#[case::a_namespace_with_a_member_none_has(&["crawl-request", "nothing"], Some(named("crawl_request", false)))]
#[case::a_member_no_product_has(&["run", "nothing"], None)]
#[case::a_word_no_product_has(&["nothing"], None)]
fn a_command_line_names_what_only_foxhound_carries(
    #[case] words: &[&str],
    #[case] expected: Option<Named>,
) {
    let arguments: Vec<OsString> = std::iter::once("goodfetch-collie")
        .chain(words.iter().copied())
        .map(OsString::from)
        .collect();

    assert_eq!(foxhound_only(&collie(), &arguments), expected);
}
