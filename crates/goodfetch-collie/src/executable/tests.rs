use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use goodfetch_cli::output::ColourChoice;
use goodfetch_engine::mask::{Masks, Reach, Redaction};
use goodfetch_protocol::catalog::Product;
use rstest::rstest;
use secrecy::SecretString;
use tracing_subscriber::layer::SubscriberExt as _;

use super::logs::Masked;
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

/// A start builds no group's commands, hand-written or generated: clap
/// builds them only for the group it parses or shows help for, so
/// `--version` and every command skip building the rest of the tree (ADR
/// 0071's startup figure).
#[rstest]
#[case::collie(Product::Collie)]
#[case::foxhound(Product::Foxhound)]
fn a_start_builds_no_command_of_any_group(#[case] product: Product) {
    let (command_line, _) = command_line(product);

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

/// Every argument of `command` and of each command below it.
fn arguments(command: &clap::Command) -> Vec<&clap::Arg> {
    command
        .get_arguments()
        .chain(command.get_subcommands().flat_map(arguments))
        .collect()
}

/// Story 4-31: no flag of either executable turns certificate verification
/// off. No flag reads a variable either: clap's `env` feature is off.
/// Covers: story:4-31
#[rstest]
fn no_flag_switches_certificate_verification_off(
    #[values(Product::Collie, Product::Foxhound)] product: Product,
) {
    // Words naming certificates or turning their checks off; not `verif`
    // alone, which `--require-verified` (a Verdict's) holds.
    const WORDS: [&str; 7] = [
        "insecure",
        "no-verify",
        "skip-verif",
        "cert",
        "tls",
        "ssl",
        "danger",
    ];
    let command = command(product);
    let every = arguments(&command);

    let switches: Vec<String> = every
        .iter()
        .flat_map(|argument| {
            argument
                .get_long_and_visible_aliases()
                .into_iter()
                .flatten()
                .map(str::to_owned)
        })
        .filter(|name| {
            let name = name.to_ascii_lowercase();
            WORDS.iter().any(|word| name.contains(word))
        })
        .collect();

    assert!(
        every.iter().any(|argument| argument.get_id() == COLOR),
        "the enumeration reaches the executable's flags"
    );
    assert_eq!(switches, Vec::<String>::new());
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
