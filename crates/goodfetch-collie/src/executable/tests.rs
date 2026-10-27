#![expect(
    clippy::disallowed_types,
    reason = "the log writer test shares what the subscriber wrote with the test through a Mutex"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use goodfetch_cli::output::ColourChoice;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::home::Home;
use goodfetch_engine::mask::{Masks, Reach, Redaction};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::log::{LOG_FILE_MAX_FILES, LOG_FILE_MAX_SIZE, LOG_OUTPUT};
use goodfetch_protocol::versions::VERSION_AGE_NOTICE;
use rstest::rstest;
use secrecy::SecretString;
use tracing_subscriber::layer::SubscriberExt as _;

use super::log_file::{LogFile, Rotation};
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

/// The containment check (ADRs 0090 and 0094): Foxhound's executable
/// carries every command of Collie's unchanged, so each command path of
/// Collie's renders the same help in Foxhound's, its executable's name
/// aside, a group's listing Foxhound's own commands beside Collie's (`run
/// outcomes` in `run`); the snapshot is Foxhound's whole help tree.
///
/// Covers: story:11-1130
#[test]
fn foxhounds_help_shows_each_command_collies_carries_unchanged() {
    let (collie, foxhound) = (command(Product::Collie), command(Product::Foxhound));

    let changed: Vec<String> = paths(&collie)
        .into_iter()
        .filter(|path| {
            // A group's list aligns its commands' help to its longest name,
            // so a line is compared with its runs of spaces as one.
            let words = |line: &str| line.split_whitespace().collect::<Vec<_>>().join(" ");
            let foxhounds = help(&foxhound, path).replace("goodfetch-foxhound", "goodfetch-collie");
            let mut lines = foxhounds.lines().map(words);
            // Each line of Collie's, in its order, with only Foxhound's own
            // commands' lines between them.
            !help(&collie, path)
                .lines()
                .map(words)
                .all(|line| lines.any(|theirs| theirs == line))
        })
        .map(|path| path.join(" "))
        .collect();

    assert_eq!(changed, Vec::<String>::new());
    let tree = std::iter::once(Vec::new()).chain(paths(&foxhound)).fold(
        String::new(),
        |mut tree, path| {
            let words: Vec<&str> = std::iter::once("goodfetch-foxhound")
                .chain(path.iter().map(String::as_str))
                .chain(std::iter::once("--help"))
                .collect();
            let _ = writeln!(tree, "$ {}\n{}", words.join(" "), help(&foxhound, &path));
            tree
        },
    );
    insta::assert_snapshot!("help_tree_foxhound", tree);
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

    let handed: Vec<String> = given
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|write| String::from_utf8(write.clone()).unwrap())
        .collect();
    assert_eq!(
        handed,
        vec![
            "{\"fields\":{\"message\":\"the proxy answered\",\"proxy\":\"%proxy.password%\"}}\n"
                .to_owned()
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

/// The Operations guide's page (spec #583), which an operator reads to
/// find and bound the Daemon's logs.
const OPERATIONS_GUIDE: &str = include_str!("../../../../docs/operations.md");

/// The rows of the log folder table in the guide's section "Where the logs
/// are": each row's place and the folders its second cell names in code
/// spans, a span holding a path separator being a folder.
fn log_folder_rows(guide: &str) -> Vec<(String, Vec<String>)> {
    let section = guide
        .split("\n## ")
        .find(|section| section.starts_with("Where the logs are"))
        .expect("the guide has a section \"Where the logs are\"");
    section
        .lines()
        .skip_while(|line| !line.starts_with("| Where"))
        .skip(2)
        .take_while(|line| line.starts_with('|'))
        .map(|line| {
            let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
            let folders = cells[1]
                .split('`')
                .skip(1)
                .step_by(2)
                .filter(|span| span.contains(['/', '\\']))
                .map(str::to_owned)
                .collect();
            (cells[0].to_owned(), folders)
        })
        .collect()
}

/// `folder` as a path, its `~` and variables (`$NAME`, `%NAME%`) expanded
/// by `variables`; `None` when it names one `variables` leaves unset.
fn expand(folder: &str, variables: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let mut path = PathBuf::new();
    for part in folder.split(['/', '\\']) {
        let value = match part {
            "~" => dirs::home_dir()?.display().to_string(),
            variable if variable.starts_with('$') => variables(&variable[1..])?,
            variable
                if variable.len() > 1 && variable.starts_with('%') && variable.ends_with('%') =>
            {
                variables(variable.trim_matches('%'))?
            }
            plain => plain.to_owned(),
        };
        path.push(value);
    }
    Some(path)
}

/// The log folder the guide names for each OS is the one the executable's
/// Goodfetch home lays out: on this OS the very folder it resolves, with
/// `GOODFETCH_HOME` unset and set, and on every other OS the same folders
/// below its data folder. The guide names each OS once, and no other place.
///
/// Covers: story:13-47
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the guide's folder for this OS depends on the variables the OS's data folder reads, as `main` passes none of them"
)]
fn the_operations_guide_names_the_log_folder_the_goodfetch_home_lays_out_on_each_os() {
    let rows = log_folder_rows(OPERATIONS_GUIDE);
    let places: Vec<&str> = rows.iter().map(|(place, _)| place.as_str()).collect();
    assert_eq!(
        places,
        ["Linux", "macOS", "Windows", "`GOODFETCH_HOME` is set"]
    );
    let below_home = Path::new(super::HOME_FOLDER).join(
        Home::new("root".into())
            .logs()
            .strip_prefix("root")
            .expect("invariant: the log folder is in the home"),
    );
    for (place, folders) in &rows[..3] {
        assert_ne!(
            folders.as_slice(),
            [] as [String; 0],
            "{place} names no log folder"
        );
        for folder in folders {
            let path = expand(folder, &|_| Some("data".to_owned())).unwrap();
            assert!(
                path.ends_with(&below_home),
                "{place}'s `{folder}` is not the Goodfetch home's log folder"
            );
        }
    }

    let this_os = match std::env::consts::OS {
        "linux" => "Linux",
        "macos" => "macOS",
        "windows" => "Windows",
        other => panic!("the guide names no log folder for {other}"),
    };
    let (_, folders) = rows
        .iter()
        .find(|(place, _)| place == this_os)
        .expect("the guide names this OS");
    let environment = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    // A folder naming a variable that is set is the one the OS uses then.
    let named = folders
        .iter()
        .filter(|folder| folder.contains(['$', '%']))
        .find_map(|folder| expand(folder, &environment));
    let plain = folders
        .iter()
        .filter(|folder| !folder.contains(['$', '%']))
        .find_map(|folder| expand(folder, &environment));
    let resolved = super::home(&BTreeMap::new())
        .expect("the OS has a data folder")
        .logs();
    assert_eq!(named.or(plain), Some(resolved));

    let (_, folders) = &rows[3];
    let root = std::env::temp_dir().join("goodfetch-home");
    let environment = BTreeMap::from([(
        OsString::from(super::HOME_VARIABLE),
        root.clone().into_os_string(),
    )]);
    let home = super::home(&environment).expect("GOODFETCH_HOME is set");
    assert_ne!(folders.as_slice(), [] as [String; 0]);
    for folder in folders {
        let named = expand(folder, &|name| {
            (name == super::HOME_VARIABLE).then(|| root.display().to_string())
        });
        assert_eq!(named, Some(home.logs()), "`{folder}`");
    }
}

/// Every `log.*` and `version.*` setting the guide names is one of Collie's
/// settings, spelled as the generated reference spells it, and the guide
/// names the age notice's setting and the settings that bound the log
/// files.
///
/// Covers: story:13-47
#[test]
fn the_operations_guide_names_the_log_and_age_notice_settings_of_the_generated_reference() {
    let guide = OPERATIONS_GUIDE;
    let catalog: serde_json::Value = serde_json::from_str(include_str!(
        "../../../goodfetch-protocol/generated/collie.json"
    ))
    .unwrap();
    let reference: Vec<&str> = catalog["settings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|setting| setting["name"].as_str().unwrap())
        .collect();
    let named: BTreeSet<&str> = guide
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| span.starts_with("log.") || span.starts_with("version."))
        .collect();
    for setting in &named {
        assert!(
            reference.contains(setting),
            "the guide names `{setting}`, which is not in Collie's settings reference"
        );
    }
    for setting in [
        VERSION_AGE_NOTICE.name,
        LOG_OUTPUT.name,
        LOG_FILE_MAX_SIZE.name,
        LOG_FILE_MAX_FILES.name,
    ] {
        assert!(
            named.contains(setting),
            "the guide does not name `{setting}`"
        );
    }
}
