use super::{of, words};
use crate::compare::recipe_arguments;

/// The command that reruns a run started with `arguments` after the
/// recipe's own.
fn rerun_of(arguments: &[&str]) -> String {
    of(&recipe_arguments(arguments))
}

/// A run given only the recipe's arguments reruns with the recipe alone.
#[test]
fn a_run_of_the_recipes_arguments_reruns_with_the_recipe() {
    assert_eq!(rerun_of(&[]), "just compare release");
}

/// Every argument the run was given after the recipe's is given again,
/// each quoted for a shell where it needs it, and a default left out.
#[test]
fn the_rerun_command_gives_each_argument_again_quoted() {
    assert_eq!(
        rerun_of(&[
            "--scenario",
            "raw",
            "--scenario",
            "browser",
            "--scenario",
            "collie",
            "--trials",
            "3",
            "--growth",
            "--machine",
            "Hetzner Cloud CCX43, 16 dedicated vCPUs",
            "--browser",
            "/opt/chrome's build",
            "--collie-driver",
            "elsewhere/act.ts",
            "--out",
            "target/compare/trials.json",
        ]),
        "just compare release --scenario raw --scenario browser --scenario collie --trials 3 \
         --growth --machine 'Hetzner Cloud CCX43, 16 dedicated vCPUs' --browser \
         '/opt/chrome'\\''s build' --collie-driver elsewhere/act.ts"
    );
}

/// The rerun command reads back, word by word, as a shell reads it: a
/// redo takes the run's own arguments from it (#2521).
#[test]
fn the_rerun_command_reads_back_as_the_words_it_gives() {
    let given = [
        "--scenario",
        "browser",
        "--spider-pages",
        "500",
        "--machine",
        "Hetzner Cloud CCX43, 16 dedicated vCPUs",
        "--foxhound-setting",
        "governor.worker_threads = \"per_core\"",
        "--cell",
        "browser/foxhound",
        "--browser",
        "/opt/chrome's build",
    ];

    let read = words(&rerun_of(&given)).expect("the command reads back");

    assert_eq!(
        read,
        ["just", "compare", "release"]
            .iter()
            .chain(&given)
            .map(|word| (*word).to_owned())
            .collect::<Vec<_>>()
    );
}

/// A command whose quote never closes is no command [`of`] wrote.
#[test]
fn an_unclosed_quote_reads_as_no_command() {
    let error = words("just compare release --machine 'Hetzner").expect_err("no command");
    assert_eq!(
        error.to_string(),
        "the rerun command `just compare release --machine 'Hetzner` leaves a quote open"
    );
}
