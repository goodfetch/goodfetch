#![expect(
    clippy::disallowed_methods,
    reason = "a test of the starters folder and of the files `init` writes"
)]

use std::collections::BTreeSet;
use std::path::Path;

use super::*;

/// The starters folder Collie's executable embeds.
const STARTERS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/starters");

/// Collie's docs page that starts a project.
const START_A_PROJECT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/start-a-project.md");

/// The text of each fenced code block of `page` whose info string is
/// `language`, every line ending in a line break.
fn blocks(page: &str, language: &str) -> Vec<String> {
    let opening = format!("```{language}");
    let mut found = Vec::new();
    let mut open: Option<String> = None;
    for line in page.lines() {
        match open.as_mut() {
            Some(_) if line == "```" => found.extend(open.take()),
            Some(block) => {
                block.push_str(line);
                block.push('\n');
            }
            None if line == opening => open = Some(String::new()),
            None => {}
        }
    }
    found
}

/// Every file under `folder`, by its `/`-separated path from it.
fn paths(folder: &Path) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(next) = folders.pop() {
        for item in std::fs::read_dir(&next).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                folders.push(path);
                continue;
            }
            let relative = path.strip_prefix(folder).unwrap();
            paths.insert(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    paths
}

/// A request for the `browser` starter into `folder` on Collie's
/// executable, with what the command line names.
fn request(folder: &Path, starter: Option<&str>, language: Option<StarterLanguage>) -> Request {
    Request {
        folder: folder.to_path_buf(),
        starter: starter.map(str::to_owned),
        language,
        product: Product::Collie,
        version: "1.2.3".to_owned(),
    }
}

/// Collie's executable embeds every file of its `starters/` folder, each
/// as the folder holds it, and no starter of Foxhound's.
///
/// Covers: story:11-278
#[test]
fn collie_embeds_its_starters_folder_and_no_foxhound_starter() {
    let embedded: BTreeSet<String> = Starters::COLLIE
        .files()
        .iter()
        .map(|file| file.path.to_owned())
        .collect();
    let unchanged = Starters::COLLIE
        .files()
        .iter()
        .all(|file| std::fs::read(Path::new(STARTERS).join(file.path)).unwrap() == file.contents);
    let foxhounds: Vec<&str> = Starters::COLLIE
        .names()
        .into_iter()
        .filter(|name| FOXHOUND_STARTERS.contains(name))
        .collect();

    assert_eq!(
        (embedded, unchanged, foxhounds),
        (paths(Path::new(STARTERS)), true, Vec::<&str>::new())
    );
}

/// Collie's page that starts a project shows each language's `browser`
/// program as its one sample in that language, so that the program is a
/// docs sample on Collie's executable (#433), which the docs examples run.
///
/// Covers: story:11-275
#[test]
fn the_start_page_shows_each_starter_program_as_its_sample() {
    let page = std::fs::read_to_string(START_A_PROJECT).unwrap();
    let programs = [
        ("ts", "typescript/src/main.ts"),
        ("python", "python/main.py"),
        ("rust", "rust/src/main.rs"),
    ];

    let shown: Vec<Vec<String>> = programs
        .iter()
        .map(|(language, _)| blocks(&page, language))
        .collect();

    let starters: Vec<Vec<String>> = programs
        .iter()
        .map(|(_, program)| {
            vec![
                std::fs::read_to_string(Path::new(STARTERS).join("browser").join(program)).unwrap(),
            ]
        })
        .collect();
    assert_eq!(
        (page.lines().any(|line| line == "products: [collie]"), shown),
        (true, starters)
    );
}

/// On a terminal, a starter and language left out are asked, and the
/// answers chosen.
///
/// Covers: story:11-275
#[test]
fn a_starter_and_language_left_out_are_asked() {
    let folder = tempfile::tempdir().unwrap();
    let mut answers = vec!["rust", "browser"];
    let mut questions = Vec::new();

    let result = write(
        Starters::COLLIE,
        request(folder.path(), None, None),
        &mut |question| {
            questions.push(question.to_owned());
            answers.pop().map(str::to_owned)
        },
    )
    .unwrap();

    assert_eq!(
        (
            questions,
            result.starter,
            result.language,
            paths(folder.path())
        ),
        (
            vec![
                "Which starter (browser)?".to_owned(),
                "Which language (typescript, python, rust)?".to_owned()
            ],
            "browser".to_owned(),
            StarterLanguage::Rust,
            BTreeSet::from(
                [".gitignore", "Cargo.toml", "src/main.rs", "src/tests.rs"].map(str::to_owned)
            ),
        )
    );
}

/// An answer that names no language is a usage error, and nothing is
/// written.
///
/// Covers: story:11-275
#[test]
fn an_answer_naming_no_language_is_refused() {
    let folder = tempfile::tempdir().unwrap();

    let refused = write(
        Starters::COLLIE,
        request(folder.path(), Some("browser"), None),
        &mut |_| Some("cobol".to_owned()),
    );

    assert!(
        matches!(&refused, Err(Refused::Usage(message)) if message.contains("`cobol`"))
            && paths(folder.path()).is_empty(),
        "{refused:?}"
    );
}

/// The project's name is its folder's in lowercase ASCII letters and
/// digits joined by `-`, a default when it holds none.
#[test]
fn the_project_name_comes_from_the_folder() {
    let names: Vec<String> = ["my starter é", "Été 2026", "--", "app_v2"]
        .into_iter()
        .map(|folder| project_name(&Path::new("/projects").join(folder)))
        .collect();

    assert_eq!(names, ["my-starter", "t-2026", DEFAULT_NAME, "app-v2"]);
}
