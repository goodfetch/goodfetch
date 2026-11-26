//! `init` through Collie's executable (story:11-275, story:11-278): the
//! `browser` starter it embeds, written in each language into a folder
//! whose name holds a space and a non-ASCII letter, its manifest naming the
//! folder and the executable's version; a path already there is left as it
//! is and listed in `skipped`; a starter or language left out, with no
//! person to ask, and Foxhound's `crawl` are usage errors, exit `2`, that
//! write nothing. Asking on a terminal is tested in process, in the `init`
//! module's own tests; building and running each written project, in
//! [`projects`].
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read and write the project folders themselves"
)]

mod projects;

use std::collections::BTreeMap;
use std::path::Path;

use goodfetch_test_support::{TestEnv, cli};
use serde_json::json;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The starters Collie's executable embeds.
const STARTERS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/starters");

/// The project folder each test writes into, under the test's folder.
const PROJECT: &str = "my starter é";

/// Every file under `folder`, by its `/`-separated path from it, with its
/// text.
fn tree(folder: &Path) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(next) = folders.pop() {
        for item in std::fs::read_dir(&next).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                folders.push(path);
                continue;
            }
            let relative = path.strip_prefix(folder).unwrap();
            let relative = relative.to_string_lossy().replace('\\', "/");
            files.insert(relative, std::fs::read_to_string(&path).unwrap());
        }
    }
    files
}

/// The starter's files as `init` writes them into [`PROJECT`]: a file
/// stored as `<path>.template` at `<path>`, with the project's name and the
/// executable's version in place of their placeholders.
fn filled(starter: BTreeMap<String, String>) -> BTreeMap<String, String> {
    starter
        .into_iter()
        .map(|(path, text)| {
            let path = path
                .strip_suffix(".template")
                .map(str::to_owned)
                .unwrap_or(path);
            let text = text
                .replace("{{name}}", "my-starter")
                .replace("{{version}}", env!("CARGO_PKG_VERSION"));
            (path, text)
        })
        .collect()
}

/// `init --starter browser` writes each language's starter, every file of
/// it listed in `written`, into a new folder named with a space and a
/// non-ASCII letter.
///
/// Covers: story:11-275, command:init
#[rstest::rstest]
#[case::typescript("typescript")]
#[case::python("python")]
#[case::rust("rust")]
fn init_writes_the_browser_starter(#[case] language: &str) {
    let env = TestEnv::new();

    let checked = cli(&env, COLLIE)
        .args([
            "init",
            PROJECT,
            "--starter",
            "browser",
            "--lang",
            language,
            "--json",
        ])
        .json("InitResult")
        .unwrap();

    let starter = filled(tree(&Path::new(STARTERS).join("browser").join(language)));
    let printed = &checked.documents[0];
    assert_eq!(
        (printed, tree(&env.folder().join(PROJECT))),
        (
            &json!({
                "starter": "browser",
                "language": language,
                "written": starter.keys().collect::<Vec<_>>(),
                "skipped": [],
            }),
            starter,
        ),
        "{}",
        checked.output
    );
}

/// A file already at one of the starter's paths is left as it is and
/// listed in `skipped`; the others are written.
///
/// Covers: story:11-275, command:init
#[test]
fn init_never_overwrites_a_path() {
    let env = TestEnv::new();
    let project = env.folder().join(PROJECT);
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("package.json"), "{}\n").unwrap();

    let checked = cli(&env, COLLIE)
        .args([
            "init",
            PROJECT,
            "--starter",
            "browser",
            "--lang",
            "typescript",
            "--json",
        ])
        .json("InitResult")
        .unwrap();

    let printed = &checked.documents[0];
    let written = tree(&project);
    assert_eq!(
        (
            &printed["skipped"],
            printed["written"]
                .as_array()
                .map(|written| written.iter().any(|path| path == "package.json")),
            written.get("package.json").map(String::as_str),
            written.len(),
        ),
        (
            &json!(["package.json"]),
            Some(false),
            Some("{}\n"),
            tree(&Path::new(STARTERS).join("browser/typescript")).len(),
        ),
        "{}",
        checked.output
    );
}

/// With no person to ask, a starter or language left out is a usage error
/// naming its flag, and nothing is written.
///
/// Covers: story:11-275, command:init, exit:usage
#[rstest::rstest]
#[case::starter(&["--lang", "python"], "--starter")]
#[case::language(&["--starter", "browser"], "--lang")]
fn init_without_a_person_refuses_what_is_left_out(#[case] given: &[&str], #[case] flag: &str) {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args(["init", PROJECT, "--json"])
        .args(given.iter().copied())
        .run()
        .unwrap();

    assert_eq!(
        (
            output.exit,
            output.stdout.as_str(),
            output.stderr.contains(flag),
            env.folder().join(PROJECT).exists(),
        ),
        (Some(2), "", true, false),
        "{output}"
    );
}

/// Collie's executable embeds no Foxhound starter: `--starter crawl` is a
/// usage error naming Foxhound's executable, and nothing is written.
///
/// Covers: story:11-278, command:init, exit:usage
#[test]
fn init_refuses_foxhounds_starter() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE)
        .args([
            "init",
            PROJECT,
            "--starter",
            "crawl",
            "--lang",
            "typescript",
        ])
        .run()
        .unwrap();

    assert_eq!(
        (
            output.exit,
            output
                .stderr
                .contains("goodfetch-foxhound init --starter crawl"),
            env.folder().join(PROJECT).exists(),
        ),
        (Some(2), true, false),
        "{output}"
    );
}
