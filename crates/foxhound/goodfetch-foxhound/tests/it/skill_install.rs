//! `skill install` through Foxhound's executable (story:11-237): it writes
//! the skills Foxhound embeds, its renderings, which name Foxhound. How the
//! command picks a host's folder, keeps what it wrote and refuses what it
//! did not is tested on Collie's executable, which shares its code.
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads the skills folder the command writes"
)]

use std::collections::BTreeMap;
use std::path::Path;

use goodfetch_test_support::{TestEnv, cli};

use crate::FOXHOUND;

/// The renderings Foxhound's executable embeds.
const RENDERINGS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/generated/skills");

/// Every file under `folder` but the install's markers, by its
/// `/`-separated path from it, with its bytes.
fn files(folder: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(next) = folders.pop() {
        for item in std::fs::read_dir(&next).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else if !path.ends_with(".goodfetch-skill") {
                let relative = path.strip_prefix(folder).unwrap();
                let relative = relative.to_string_lossy().replace('\\', "/");
                files.insert(relative, std::fs::read(&path).unwrap());
            }
        }
    }
    files
}

/// Foxhound's executable installs Foxhound's skills, not Collie's.
///
/// Covers: story:11-237, command:skill.install
#[test]
fn skill_install_writes_foxhounds_skills() {
    let env = TestEnv::new();
    let folder = env.folder().join(".agents/skills");

    let checked = cli(&env, FOXHOUND)
        .args(["skill", "install", "--json"])
        .json("SkillsInstalled")
        .unwrap();

    let names: Vec<&str> = checked.documents[0]["installed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skill| skill["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        (names, files(&folder)),
        (
            vec!["goodfetch-foxhound", "goodfetch-foxhound-cli"],
            files(Path::new(RENDERINGS)),
        ),
        "{}",
        checked.output
    );
}
