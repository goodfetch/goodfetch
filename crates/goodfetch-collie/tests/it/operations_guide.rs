//! The Operations guide (spec #583), `docs/operations.md`, which an operator
//! reads to find and bound the Daemon's logs and to turn off the age notice:
//! each log folder it names is the one the Goodfetch home lays out, each
//! setting it names is in Collie's generated settings reference, and each
//! settings file it shows reads as one.

use std::path::{Component, Path, PathBuf};

use goodfetch_engine::home::Home;
use goodfetch_engine::settings::{SettingsFiles, operator_policy, user_configuration};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::log::{LOG_FILE_MAX_FILES, LOG_FILE_MAX_SIZE, LOG_OUTPUT};
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_protocol::versions::VERSION_AGE_NOTICE;
use rstest::rstest;

const GUIDE: &str = include_str!("../../../../docs/operations.md");

/// The guide's spans of code, in order.
fn code_spans(guide: &str) -> impl Iterator<Item = &str> {
    guide.split('`').skip(1).step_by(2)
}

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
            let folders = code_spans(cells[1])
                .filter(|span| span.contains(['/', '\\']))
                .map(str::to_owned)
                .collect();
            (cells[0].to_owned(), folders)
        })
        .collect()
}

/// A folder's parts, whichever separator it is written with.
fn parts(folder: &str) -> Vec<String> {
    folder.split(['/', '\\']).map(str::to_owned).collect()
}

/// The log folder's parts below the Goodfetch home, as the engine lays it
/// out.
fn logs_below_home() -> Vec<String> {
    let root = Path::new("home");
    Home::new(root.to_owned())
        .logs()
        .strip_prefix(root)
        .expect("invariant: the log folder is in the home")
        .components()
        .map(|part| match part {
            Component::Normal(part) => part.to_string_lossy().into_owned(),
            other => panic!("the log folder's part {other:?} is no folder name"),
        })
        .collect()
}

/// The guide names a log folder for each OS once, and for no other place.
///
/// Covers: story:13-47
#[test]
fn the_operations_guide_names_the_log_folder_of_each_os_and_of_a_set_goodfetch_home() {
    let rows = log_folder_rows(GUIDE);

    let places: Vec<&str> = rows.iter().map(|(place, _)| place.as_str()).collect();
    assert_eq!(
        places,
        ["Linux", "macOS", "Windows", "`GOODFETCH_HOME` is set"]
    );
}

/// Each OS's row names the log folder of the Goodfetch home in that OS's
/// data folder, as `dirs` documents it (`$XDG_DATA_HOME` or
/// `~/.local/share`, `~/Library/Application Support`,
/// `{FOLDERID_RoamingAppData}`), on every host; a set `GOODFETCH_HOME` is
/// the home itself.
///
/// Covers: story:13-47
#[rstest]
#[case::linux("Linux", &["~/.local/share/goodfetch", "$XDG_DATA_HOME/goodfetch"])]
#[case::macos("macOS", &["~/Library/Application Support/goodfetch"])]
#[case::windows("Windows", &["%APPDATA%/goodfetch"])]
#[case::goodfetch_home("`GOODFETCH_HOME` is set", &["$GOODFETCH_HOME"])]
fn the_operations_guide_names_the_log_folder_of_the_goodfetch_home_in_each_places_data_folder(
    #[case] place: &str,
    #[case] homes: &[&str],
) {
    let rows = log_folder_rows(GUIDE);
    let below_home = logs_below_home();

    let (_, folders) = rows
        .iter()
        .find(|(row, _)| row == place)
        .unwrap_or_else(|| panic!("the guide names no log folder for {place}"));

    let named: Vec<Vec<String>> = folders.iter().map(|folder| parts(folder)).collect();
    let expected: Vec<Vec<String>> = homes
        .iter()
        .map(|home| [parts(home), below_home.clone()].concat())
        .collect();
    assert_eq!(named, expected);
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

/// On this host, the guide's row for its OS names the very folder the
/// Goodfetch home in the OS's data folder logs to: a folder naming a
/// variable that is set is the one the OS uses then.
///
/// Covers: story:13-47
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the guide's folder for this OS depends on the variables the OS's data folder reads"
)]
fn the_operations_guide_names_this_hosts_log_folder_as_its_data_folder_resolves() {
    let this_os = match std::env::consts::OS {
        "linux" => "Linux",
        "macos" => "macOS",
        "windows" => "Windows",
        other => panic!("the guide names no log folder for {other}"),
    };
    let rows = log_folder_rows(GUIDE);
    let (_, folders) = rows
        .iter()
        .find(|(place, _)| place == this_os)
        .expect("the guide names this OS");
    let environment = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());

    let named = folders
        .iter()
        .filter(|folder| folder.contains(['$', '%']))
        .find_map(|folder| expand(folder, &environment));
    let plain = folders
        .iter()
        .filter(|folder| !folder.contains(['$', '%']))
        .find_map(|folder| expand(folder, &environment));

    let home = Home::new(
        dirs::data_dir()
            .expect("the OS has a data folder")
            .join("goodfetch"),
    );
    assert_eq!(named.or(plain), Some(home.logs()));
}

/// Every `log.*` and `version.*` setting the guide names is one of Collie's
/// settings, spelled as the generated reference spells it, and the guide
/// names the age notice's setting and the settings that bound the log
/// files.
///
/// Covers: story:13-47
#[test]
fn the_operations_guide_names_the_log_and_age_notice_settings_of_the_generated_reference() {
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

    let named: Vec<&str> = code_spans(GUIDE)
        .filter(|span| span.starts_with("log.") || span.starts_with("version."))
        .collect();

    let unknown: Vec<&&str> = named
        .iter()
        .filter(|setting| !reference.contains(setting))
        .collect();
    assert_eq!(unknown, Vec::<&&str>::new(), "not in Collie's reference");
    let missing: Vec<&str> = [
        VERSION_AGE_NOTICE.name,
        LOG_OUTPUT.name,
        LOG_FILE_MAX_SIZE.name,
        LOG_FILE_MAX_FILES.name,
    ]
    .into_iter()
    .filter(|setting| !named.contains(setting))
    .collect();
    assert_eq!(missing, Vec::<&str>::new(), "not named by the guide");
}

/// The guide's fenced `toml` blocks, each the whole text of a file.
fn toml_blocks(guide: &str) -> Vec<&str> {
    guide
        .split("```toml\n")
        .skip(1)
        .map(|block| {
            block
                .split_once("```")
                .expect("a `toml` block ends with its fence")
                .0
        })
        .collect()
}

/// Each settings file the guide shows, written as it shows it as a new user
/// configuration or Operator policy, reads as one in Collie's engine, so the
/// Daemon that loads it starts.
///
/// Covers: story:13-47
#[test]
fn each_settings_file_the_operations_guide_shows_reads_as_user_configuration_and_operator_policy() {
    let version = ProductVersion::parse(env!("CARGO_PKG_VERSION")).unwrap();
    let blocks = toml_blocks(GUIDE);
    assert_ne!(
        blocks,
        Vec::<&str>::new(),
        "the guide shows a settings file"
    );

    let refused: Vec<String> = blocks
        .iter()
        .flat_map(|block| {
            [
                user_configuration(
                    SettingsLayer::UserConfiguration,
                    block,
                    Product::Collie,
                    &version,
                )
                .err()
                .map(|error| error.to_string()),
                operator_policy(block, Product::Collie, &version)
                    .err()
                    .map(|error| error.to_string()),
            ]
        })
        .flatten()
        .collect();

    assert_eq!(refused, Vec::<String>::new());
}

/// The guide names where the user configuration and the Operator policy
/// are, by the paths the engine reads them from: in the Goodfetch home of
/// its log folder table, below the major's folder, such as major 1's on
/// Linux.
///
/// Covers: story:13-47
#[test]
fn the_operations_guide_names_the_settings_files_where_the_engine_reads_them() {
    let home = Home::new(PathBuf::from("~/.local/share/goodfetch"));
    let files = SettingsFiles::of(&home, &ProductVersion::parse("1.0.0").unwrap());
    let spans: Vec<Vec<String>> = code_spans(GUIDE).map(parts).collect();

    let named: Vec<bool> = [files.user_configuration(), files.operator_policy()]
        .iter()
        .map(|path| {
            let path: Vec<String> = path
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            spans.contains(&path)
        })
        .collect();

    assert_eq!(named, [true, true]);
}
