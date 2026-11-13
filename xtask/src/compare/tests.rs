use clap::Parser;

use super::{RunArguments, rerun};

/// `xtask compare run`'s arguments, as the recipe and its caller give them.
#[derive(Parser)]
struct Run {
    #[command(flatten)]
    arguments: RunArguments,
}

/// The command that reruns a run started with `arguments` after the
/// recipe's own.
fn rerun_of(arguments: &[&str]) -> String {
    let recipe = [
        "xtask",
        "--tools",
        "target/compare/test-tools.json",
        "--spider-builds",
        "target/compare-spider",
        "--spider-profile",
        "release",
    ];
    let run = Run::try_parse_from(recipe.iter().chain(arguments))
        .expect("invariant: the test's arguments parse");
    rerun(&run.arguments)
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
            "--trials",
            "3",
            "--growth",
            "--machine",
            "Hetzner Cloud CCX43, 16 dedicated vCPUs",
            "--browser",
            "/opt/chrome's build",
            "--out",
            "target/compare/trials.json",
        ]),
        "just compare release --scenario raw --scenario browser --trials 3 --growth --machine \
         'Hetzner Cloud CCX43, 16 dedicated vCPUs' --browser '/opt/chrome'\\''s build'"
    );
}
