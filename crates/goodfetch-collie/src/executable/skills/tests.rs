#![expect(
    clippy::disallowed_methods,
    reason = "a test of the files the install writes"
)]

use std::path::Path;

use super::*;

/// The skills of one release: `goodfetch-a`, with a reference, and
/// `goodfetch-b`.
const FIRST: &[SkillFile] = &[
    SkillFile {
        path: "goodfetch-a/SKILL.md",
        contents: b"a, first\n",
    },
    SkillFile {
        path: "goodfetch-a/references/old.md",
        contents: b"dropped next\n",
    },
    SkillFile {
        path: "goodfetch-b/SKILL.md",
        contents: b"b, first\n",
    },
];

/// The same skills in the next release.
const NEXT: &[SkillFile] = &[
    SkillFile {
        path: "goodfetch-a/SKILL.md",
        contents: b"a, next\n",
    },
    SkillFile {
        path: "goodfetch-b/SKILL.md",
        contents: b"b, next\n",
    },
];

/// Every file of the skill folder `folder` but its marker, by path.
fn files(folder: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(next) = folders.pop() {
        for item in std::fs::read_dir(&next).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else if !path.ends_with(MARKER) {
                let relative = path.strip_prefix(folder).unwrap().to_string_lossy();
                files.push((
                    relative.replace('\\', "/"),
                    std::fs::read_to_string(&path).unwrap(),
                ));
            }
        }
    }
    files.sort();
    files
}

/// A skill it installed and nobody changed is replaced by the next
/// release's, whose files take the place of all of its own.
///
/// Covers: story:11-237
#[test]
fn an_install_replaces_the_skills_it_installed() {
    let folder = tempfile::tempdir().unwrap();
    install_into(Skills::new(FIRST), folder.path()).unwrap();

    let installed = install_into(Skills::new(NEXT), folder.path()).unwrap();

    let written: Vec<(&str, bool)> = installed
        .iter()
        .map(|skill| (skill.skill.name.as_str(), skill.written))
        .collect();
    assert_eq!(
        (
            written,
            files(&folder.path().join("goodfetch-a")),
            files(&folder.path().join("goodfetch-b")),
        ),
        (
            vec![("goodfetch-a", true), ("goodfetch-b", true)],
            vec![("SKILL.md".to_owned(), "a, next\n".to_owned())],
            vec![("SKILL.md".to_owned(), "b, next\n".to_owned())],
        )
    );
}

/// A skill it installed that the person changed since is theirs: the
/// install is refused, naming its folder, and writes no skill.
///
/// Covers: story:11-237
#[test]
fn an_install_refuses_a_skill_changed_since_it_installed_it() {
    let folder = tempfile::tempdir().unwrap();
    install_into(Skills::new(FIRST), folder.path()).unwrap();
    let edited = folder.path().join("goodfetch-b/SKILL.md");
    std::fs::write(&edited, "b, my way\n").unwrap();

    let refused = install_into(Skills::new(NEXT), folder.path());

    assert_eq!(
        (
            refused.err().map(|refused| match refused {
                Refused::Taken(path) => path,
                Refused::Io { path, .. } => path.join("io"),
            }),
            files(&folder.path().join("goodfetch-a")),
            std::fs::read_to_string(&edited).unwrap(),
        ),
        (
            Some(folder.path().join("goodfetch-b")),
            vec![
                ("SKILL.md".to_owned(), "a, first\n".to_owned()),
                ("references/old.md".to_owned(), "dropped next\n".to_owned()),
            ],
            "b, my way\n".to_owned(),
        )
    );
}
