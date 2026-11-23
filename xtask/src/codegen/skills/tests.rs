#![expect(
    clippy::disallowed_methods,
    reason = "a test of files it writes; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use serde_json::{Value, json};

use super::*;

/// A workspace holding the wording list and `skills`, each a file by path
/// from the root and its text.
fn workspace(skills: &[(&str, &str)]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("wording.toml"),
        "[[term]]\nwords = \"bypass\"\nevery_form = true\n",
    )
    .unwrap();
    for (path, text) in skills {
        let file = root.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    root
}

fn operation(name: &str, product: &str, reach: &str) -> Value {
    json!({ "name": name, "product": product, "reach": reach })
}

fn catalog() -> Value {
    json!({ "operations": [
        operation("run.start", "collie", "all_surfaces"),
        operation("usage.raise", "collie", "person_only"),
        operation("session.cookies", "collie", "sdk_only"),
        operation("crawl.start", "foxhound", "all_surfaces"),
    ]})
}

fn command(name: &str, commands: &[Value]) -> Value {
    json!({ "name": name, "commands": commands })
}

fn command_lines() -> CommandLines {
    let collies = || {
        vec![
            command("run", &[command("start", &[])]),
            command("usage", &[command("raise", &[])]),
        ]
    };
    let mut foxhounds = collies();
    foxhounds.push(command("crawl", &[command("start", &[])]));
    serde_json::from_value(json!({
        "collie": command("goodfetch-collie", &collies()),
        "foxhound": command("goodfetch-foxhound", &foxhounds),
    }))
    .unwrap()
}

/// A `SKILL.md` named `name`, described by `description`, whose body is
/// `body`, starting on line 5.
fn skill(name: &str, description: &str, body: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n{body}\n")
}

fn render_at(root: &Path) -> (BTreeMap<String, String>, Vec<(&'static str, String)>) {
    let (files, findings) = rendered(root, &catalog(), Some(&command_lines())).unwrap();
    (
        files
            .into_iter()
            .map(|(path, text)| (path, String::from_utf8(text).unwrap()))
            .collect(),
        findings
            .into_iter()
            .map(|finding| (finding.kind, finding.subject))
            .collect(),
    )
}

/// Covers: story:11-240
#[test]
fn a_skill_renders_for_each_product_that_carries_it_with_its_names() {
    let overview = skill(
        "{{executable}}",
        "\"Drives a browser with {{executable}}: a Run, its pages and their reads.\"",
        "# {{executable}}\n\nStart a Run with `{{executable}} run start`, which calls `run.start`.\n\nTo raise a Limit, ask the person to run `{{executable}} usage raise` at a terminal.",
    );
    let crawl = skill(
        "{{executable}}-crawl",
        "Crawls a site with {{executable}}.",
        "Start one with `{{executable}} crawl start`, which calls `crawl.start`.",
    );
    let root = workspace(&[
        ("skills/goodfetch-collie/SKILL.md", &overview),
        (
            "skills/goodfetch-collie/references/runs.md",
            "Runs of `{{executable}}`.\n",
        ),
        ("skills/foxhound/goodfetch-foxhound-crawl/SKILL.md", &crawl),
    ]);

    let (files, findings) = render_at(root.path());

    assert_eq!(findings, Vec::new());
    assert_eq!(
        files,
        BTreeMap::from([
            (
                format!("{COLLIE_FOLDER}/goodfetch-collie/SKILL.md"),
                "---\nname: goodfetch-collie\ndescription: \"Drives a browser with goodfetch-collie: a Run, its pages and their reads.\"\n---\n# goodfetch-collie\n\nStart a Run with `goodfetch-collie run start`, which calls `run.start`.\n\nTo raise a Limit, ask the person to run `goodfetch-collie usage raise` at a terminal.\n".to_owned(),
            ),
            (
                format!("{COLLIE_FOLDER}/goodfetch-collie/references/runs.md"),
                "Runs of `goodfetch-collie`.\n".to_owned(),
            ),
            (
                format!("{FOXHOUND_FOLDER}/goodfetch-foxhound/SKILL.md"),
                "---\nname: goodfetch-foxhound\ndescription: \"Drives a browser with goodfetch-foxhound: a Run, its pages and their reads.\"\n---\n# goodfetch-foxhound\n\nStart a Run with `goodfetch-foxhound run start`, which calls `run.start`.\n\nTo raise a Limit, ask the person to run `goodfetch-foxhound usage raise` at a terminal.\n".to_owned(),
            ),
            (
                format!("{FOXHOUND_FOLDER}/goodfetch-foxhound/references/runs.md"),
                "Runs of `goodfetch-foxhound`.\n".to_owned(),
            ),
            (
                format!("{FOXHOUND_FOLDER}/goodfetch-foxhound-crawl/SKILL.md"),
                "---\nname: goodfetch-foxhound-crawl\ndescription: \"Crawls a site with goodfetch-foxhound.\"\n---\nStart one with `goodfetch-foxhound crawl start`, which calls `crawl.start`.\n".to_owned(),
            ),
        ])
    );
}

/// Covers: story:11-240
#[rstest::rstest]
#[case::name_over_64_characters(
    &format!("{{{{executable}}}}-{}", "a".repeat(48)),
    "Drives a browser.",
    "Text.",
    ("skill-name", 2)
)]
#[case::name_not_its_folders("{{executable}}-cli", "Drives a browser.", "Text.", ("skill-name", 2))]
#[case::description_over_1024_characters(
    "{{executable}}",
    &"a".repeat(1_025),
    "Text.",
    ("skill-description", 3)
)]
#[case::operation_not_in_the_catalog(
    "{{executable}}",
    "Drives a browser.",
    "Call `run.begin`.",
    ("skill-operation", 5)
)]
#[case::operation_for_the_sdks_only(
    "{{executable}}",
    "Drives a browser.",
    "Read them with `session.cookies`.",
    ("skill-operation", 5)
)]
#[case::operation_of_another_product(
    "{{executable}}",
    "Drives a browser.",
    "Crawl with `crawl.start`.",
    ("skill-operation", 5)
)]
#[case::command_that_does_not_exist(
    "{{executable}}",
    "Drives a browser.",
    "Run `{{executable}} run begin`.",
    ("skill-command", 5)
)]
#[case::example_command_that_does_not_exist(
    "{{executable}}",
    "Drives a browser.",
    "```console\n$ {{executable}} run begin --json\n```",
    ("skill-command", 6)
)]
#[case::persons_answer_as_an_example(
    "{{executable}}",
    "Drives a browser.",
    "```console\n$ {{executable}} usage raise --amount 10\n```",
    ("skill-person-only", 6)
)]
#[case::persons_answer_as_the_agents_action(
    "{{executable}}",
    "Drives a browser.",
    "Raise the Limit with `{{executable}} usage raise`.",
    ("skill-person-only", 5)
)]
#[case::persons_operation_as_the_agents_action(
    "{{executable}}",
    "Drives a browser.",
    "Call `usage.raise`.",
    ("skill-person-only", 5)
)]
#[case::wording_policy_word(
    "{{executable}}",
    "Drives a browser.",
    "It bypasses nothing.",
    ("wording", 5)
)]
fn the_generator_fails_a_skill_that_breaks_a_rule(
    #[case] name: &str,
    #[case] description: &str,
    #[case] body: &str,
    #[case] (kind, line): (&str, usize),
) {
    let root = workspace(&[(
        "skills/goodfetch-collie/SKILL.md",
        &skill(name, description, body),
    )]);

    let (_, findings) = render_at(root.path());

    assert_eq!(
        findings,
        vec![(kind, format!("skills/goodfetch-collie/SKILL.md:{line}"))]
    );
}

/// Covers: story:11-240
#[test]
fn a_skill_without_a_front_matter_does_not_render() {
    let root = workspace(&[("skills/goodfetch-collie/SKILL.md", "# Collie\n")]);

    let (files, findings) = render_at(root.path());

    assert_eq!(
        (files, findings),
        (
            BTreeMap::new(),
            vec![(
                "skill-front-matter",
                "skills/goodfetch-collie/SKILL.md:1".to_owned()
            )]
        )
    );
}
