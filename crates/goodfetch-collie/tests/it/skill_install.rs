//! `skill install` through Collie's executable (story:11-237): the agent
//! skills it embeds, written into each host's skills folder at project
//! scope, the test's folder, or at user scope, its `HOME`; a second run
//! writes nothing, and a skill folder it did not install is refused before
//! anything is written. Replacing a skill it installed is tested in
//! process, from the `skills` module's own skills.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read and write the skills folders themselves"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

use goodfetch_test_support::{TestEnv, cli};
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The renderings Collie's executable embeds.
const RENDERINGS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/generated/skills");

/// Every file under `folder`, by its `/`-separated path from it, with its
/// bytes and when it was last written.
fn tree(folder: &Path) -> BTreeMap<String, (Vec<u8>, SystemTime)> {
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
            let written = std::fs::metadata(&path).unwrap().modified().unwrap();
            files.insert(relative, (std::fs::read(&path).unwrap(), written));
        }
    }
    files
}

fn contents(tree: BTreeMap<String, (Vec<u8>, SystemTime)>) -> BTreeMap<String, Vec<u8>> {
    tree.into_iter()
        .map(|(path, (bytes, _))| (path, bytes))
        .collect()
}

/// Each host's skills folder, at each scope, receives the skills Collie
/// embeds, each in a folder of its name, and the output names each with
/// its folder and digest.
///
/// Covers: story:11-237, command:skill.install
#[rstest::rstest]
#[case::claude_code_project("claude-code", "--project", "claude_code", ".claude/skills")]
#[case::claude_code_user("claude-code", "--user", "claude_code", "user/.claude/skills")]
#[case::codex_project("codex", "--project", "codex", ".agents/skills")]
#[case::codex_user("codex", "--user", "codex", "user/.agents/skills")]
#[case::cursor_project("cursor", "--project", "cursor", ".cursor/skills")]
#[case::cursor_user("cursor", "--user", "cursor", "user/.cursor/skills")]
#[case::agents_project("agents", "--project", "agents", ".agents/skills")]
#[case::agents_user("agents", "--user", "agents", "user/.agents/skills")]
fn skill_install_writes_the_hosts_skills_folder(
    #[case] host: &str,
    #[case] scope: &str,
    #[case] target: &str,
    #[case] folder: &str,
) {
    let env = TestEnv::new();
    let folder = env.folder().join(folder);

    let checked = cli(&env, COLLIE)
        .args(["skill", "install", "--for", host, scope, "--json"])
        .json("SkillsInstalled")
        .unwrap();

    let printed = &checked.documents[0];
    let installed: Vec<(&str, String)> = printed["installed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skill| {
            let digest = skill["digest"].as_str().unwrap();
            assert!(
                digest.starts_with("sha256:") && digest.len() == 71,
                "{digest}"
            );
            (
                skill["name"].as_str().unwrap(),
                skill["path"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let written = contents(tree(&folder));
    let ours: BTreeMap<String, Vec<u8>> = written
        .into_iter()
        .filter(|(path, _)| !path.ends_with("/.goodfetch-skill"))
        .collect();
    assert_eq!(
        (&printed["target"], &printed["scope"], installed, ours),
        (
            &json!(target),
            &json!(scope.trim_start_matches("--")),
            vec![
                (
                    "goodfetch-collie",
                    folder.join("goodfetch-collie").display().to_string()
                ),
                (
                    "goodfetch-collie-cli",
                    folder.join("goodfetch-collie-cli").display().to_string()
                ),
            ],
            contents(tree(Path::new(RENDERINGS))),
        ),
        "{}",
        checked.output
    );
}

/// A second run finds every skill as the first wrote it: it prints the
/// same and writes nothing. With no flags, it installs for `agents` at
/// project scope.
///
/// Covers: story:11-237, command:skill.install
#[test]
fn skill_install_twice_writes_nothing_the_second_time() {
    let env = TestEnv::new();
    let folder = env.folder().join(".agents/skills");
    let first = cli(&env, COLLIE)
        .args(["skill", "install", "--json"])
        .json("SkillsInstalled")
        .unwrap();
    let before = tree(&folder);

    let second = cli(&env, COLLIE)
        .args(["skill", "install", "--json"])
        .json("SkillsInstalled")
        .unwrap();

    assert_eq!(
        (&second.documents, tree(&folder)),
        (&first.documents, before),
        "{}",
        second.output
    );
}

/// A folder of a skill's name that `skill install` did not write is the
/// person's: the run is refused with `skill_install_refused`, exit `4`, and
/// writes no skill, that folder's or any other.
///
/// Covers: story:11-237, command:skill.install, error:skill_install_refused, exit:refused
#[test]
fn skill_install_refuses_a_skill_folder_it_did_not_install() {
    let env = TestEnv::new();
    let folder = env.folder().join(".claude/skills");
    let theirs = folder.join("goodfetch-collie");
    std::fs::create_dir_all(&theirs).unwrap();
    std::fs::write(theirs.join("SKILL.md"), "my own notes\n").unwrap();
    let before = contents(tree(&folder));

    let refused = cli(&env, COLLIE)
        .args(["skill", "install", "--for", "claude-code", "--json"])
        .run()
        .unwrap();

    let printed: Value = serde_json::from_str(&refused.stdout).unwrap_or(Value::Null);
    assert_eq!(
        (
            refused.exit,
            printed["error"]["code"].as_str(),
            contents(tree(&folder))
        ),
        (Some(4), Some("skill_install_refused"), before),
        "{refused}"
    );
}
