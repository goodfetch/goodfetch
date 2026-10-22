use std::fmt::Write as _;

use goodfetch_cli::output::ColourChoice;
use goodfetch_protocol::catalog::Product;
use rstest::rstest;

use super::{COLOR, command, command_line};

/// Every command path below `command`, each group before its commands,
/// hidden ones included.
fn paths(command: &clap::Command) -> Vec<Vec<String>> {
    command
        .get_subcommands()
        .flat_map(|sub| {
            std::iter::once(vec![sub.get_name().to_owned()]).chain(paths(sub).into_iter().map(
                |mut path| {
                    path.insert(0, sub.get_name().to_owned());
                    path
                },
            ))
        })
        .collect()
}

/// The help the executable prints for `path`, rendered by its own command
/// line.
fn help(command: &clap::Command, path: &[String]) -> String {
    let arguments = std::iter::once(command.get_name().to_owned())
        .chain(path.iter().cloned())
        .chain(std::iter::once("--help".to_owned()));
    command
        .clone()
        .try_get_matches_from(arguments)
        .err()
        .unwrap()
        .to_string()
}

#[rstest]
#[case::collie(Product::Collie)]
#[case::foxhound(Product::Foxhound)]
fn the_command_line_holds_no_clap_misconfiguration(#[case] product: Product) {
    command(product).debug_assert();
}

#[test]
fn the_help_of_every_command_is_rendered_from_the_command_line() {
    let command = command(Product::Collie);

    let tree =
        std::iter::once(Vec::new())
            .chain(paths(&command))
            .fold(String::new(), |mut tree, path| {
                let words: Vec<&str> = std::iter::once("goodfetch-collie")
                    .chain(path.iter().map(String::as_str))
                    .chain(std::iter::once("--help"))
                    .collect();
                let _ = writeln!(tree, "$ {}\n{}", words.join(" "), help(&command, &path));
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
    let (command_line, _) = command_line(product);

    let matches = command_line
        .try_get_matches_from(std::iter::once("goodfetch").chain(arguments.iter().copied()))
        .unwrap();

    assert_eq!(
        matches.get_one::<ColourChoice>(COLOR).copied(),
        Some(expected)
    );
}
