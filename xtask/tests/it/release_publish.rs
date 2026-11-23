//! `xtask release plan`: both Products' publish as one step per registry and
//! package, in the order ADR 0094 sets, each read from its registry's public
//! interface. What is already out is skipped, so a re-run finishes a partial
//! publish; a name not on its registry yet is flagged, since trusted
//! publishing needs a first publish by hand (spec #130).

use std::collections::BTreeMap;
use std::path::Path;

use crate::github::{GitHub, Request};
use crate::workspace::{Outcome, Workspace, finding};

/// The Node platforms each Product's npm platform packages are named
/// after (ADR 0090).
const PLATFORMS: [&str; 6] = [
    "darwin-arm64",
    "darwin-x64",
    "linux-arm64",
    "linux-x64",
    "win32-arm64",
    "win32-x64",
];

/// The fixture's crates a release publishes: the three released crates and
/// the internal crate they build on.
const CRATES: [&str; 4] = [
    "goodfetch-collie",
    "goodfetch-collie-client",
    "goodfetch-engine",
    "goodfetch-foxhound",
];

/// Every npm package of both Products.
fn npm_packages() -> Vec<String> {
    let mut names = vec!["@goodfetch/collie-client".to_owned()];
    for product in ["collie", "foxhound"] {
        names.extend(
            PLATFORMS
                .iter()
                .map(|platform| format!("@goodfetch/{product}-{platform}")),
        );
        names.push(format!("@goodfetch/{product}"));
    }
    names
}

/// What the registries hold before the plan reads them: each name's
/// published versions, a name left out being not on its registry yet.
struct Published {
    crates: BTreeMap<String, Vec<String>>,
    npm: BTreeMap<String, Vec<String>>,
    /// `Some(true)` for a draft GitHub Release of the version, `Some(false)`
    /// for a published one.
    release: Option<bool>,
    tag: bool,
}

impl Published {
    /// Every name out at `1.3.0`, the internal crate at `0.0.0`, and
    /// nothing of `1.4.0`.
    fn previous_release() -> Self {
        let crates = CRATES
            .iter()
            .map(|name| {
                let version = if *name == "goodfetch-engine" {
                    "0.0.0"
                } else {
                    "1.3.0"
                };
                ((*name).to_owned(), vec![version.to_owned()])
            })
            .collect();
        let npm = npm_packages()
            .into_iter()
            .map(|name| (name, vec!["1.3.0".to_owned()]))
            .collect();
        Self {
            crates,
            npm,
            release: None,
            tag: false,
        }
    }

    /// Adds `version` to the crate's published versions.
    fn crate_out(&mut self, name: &str, version: &str) {
        self.crates
            .entry(name.to_owned())
            .or_default()
            .push(version.to_owned());
    }

    fn npm_out(&mut self, name: &str, version: &str) {
        self.npm
            .entry(name.to_owned())
            .or_default()
            .push(version.to_owned());
    }
}

/// The registries' stand-ins, serving what [`Published`] holds: a static
/// sparse Cargo index served from a temporary folder, npm's packuments, and
/// GitHub.
struct Registries {
    _index: tempfile::TempDir,
    cargo: GitHub,
    npm: GitHub,
    github: GitHub,
}

impl Registries {
    fn serving(published: &Published) -> Self {
        let index = tempfile::tempdir().unwrap();
        for (name, versions) in &published.crates {
            // Cargo's index keeps a name of four characters or more at its
            // first two, then its next two: every fixture crate's is
            // `goodfetch-…`.
            let path = index.path().join("go/od").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let lines: Vec<String> = versions
                .iter()
                .map(|version| {
                    format!(r#"{{"name":"{name}","vers":"{version}","deps":[],"cksum":"0","features":{{}},"yanked":false}}"#)
                })
                .collect();
            std::fs::write(path, lines.join("\n") + "\n").unwrap();
        }
        let cargo = serve_folder(index.path());

        let npm: Vec<(String, String)> = published
            .npm
            .iter()
            .map(|(name, versions)| {
                let listed: Vec<String> = versions
                    .iter()
                    .map(|version| {
                        format!(r#""{version}":{{"name":"{name}","version":"{version}"}}"#)
                    })
                    .collect();
                (
                    format!("/{}", name.replace('/', "%2f")),
                    format!(r#"{{"name":"{name}","versions":{{{}}}}}"#, listed.join(",")),
                )
            })
            .collect();

        let mut github = vec![(
            "/repos/goodfetch/goodfetch/releases?per_page=100&page=1".to_owned(),
            match published.release {
                None => "[]".to_owned(),
                Some(draft) => format!(r#"[{{"tag_name":"v1.4.0","draft":{draft}}}]"#),
            },
        )];
        if published.tag {
            github.push((
                "/repos/goodfetch/goodfetch/git/ref/tags/v1.4.0".to_owned(),
                r#"{"ref":"refs/tags/v1.4.0"}"#.to_owned(),
            ));
        }

        Self {
            _index: index,
            cargo,
            npm: serve(&npm),
            github: serve(&github),
        }
    }
}

fn serve(routes: &[(String, String)]) -> GitHub {
    let routes: Vec<(&str, &str)> = routes
        .iter()
        .map(|(path, body)| (path.as_str(), body.as_str()))
        .collect();
    GitHub::serving(&routes)
}

/// Serves each file under `folder` at its path from it, as a static web
/// server does.
fn serve_folder(folder: &Path) -> GitHub {
    let mut routes = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(folder).unwrap().to_str().unwrap();
                routes.push((
                    format!("/{relative}"),
                    std::fs::read_to_string(&path).unwrap(),
                ));
            }
        }
    }
    serve(&routes)
}

/// The fixture with `version` prepared in its released manifests and
/// committed, as the release job runs the plan.
fn prepared(version: &str) -> Workspace {
    let workspace = Workspace::from_fixture("release");
    let outcome = workspace.xtask(&["release", "prepare", "--version", version]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    workspace.commit_all();
    workspace
}

fn plan(workspace: &Workspace, version: &str, registries: &Registries) -> Outcome {
    workspace.xtask_with_env(
        &[
            "release",
            "plan",
            "--version",
            version,
            "--cargo-index",
            registries.cargo.url(),
            "--npm-registry",
            registries.npm.url(),
            "--api-url",
            registries.github.url(),
        ],
        &[("GITHUB_TOKEN", "a-read-token".as_ref())],
    )
}

/// Each step's state, by the step it names.
fn states(stdout: &str) -> BTreeMap<&str, &str> {
    stdout
        .lines()
        .filter(|line| !line.starts_with("error[") && !line.starts_with("note["))
        .filter_map(|line| line.split_once(": "))
        .collect()
}

#[test]
fn plan_orders_every_step_of_both_products_for_publishing() {
    let workspace = prepared("1.4.0");
    let registries = Registries::serving(&Published::previous_release());

    let outcome = plan(&workspace, "1.4.0", &registries);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    insta::assert_snapshot!(outcome.stdout);
    for registry in [&registries.github, &registries.npm, &registries.cargo] {
        assert_eq!(registry.writes(), Vec::<Request>::new());
    }
}

#[test]
fn plan_skips_each_step_a_partial_publish_finished() {
    let workspace = prepared("1.4.0");
    let mut published = Published::previous_release();
    published.release = Some(true);
    published.crate_out("goodfetch-collie-client", "1.4.0");
    published.npm_out("@goodfetch/collie-client", "1.4.0");
    for platform in PLATFORMS {
        published.npm_out(&format!("@goodfetch/collie-{platform}"), "1.4.0");
    }
    let registries = Registries::serving(&published);

    let outcome = plan(&workspace, "1.4.0", &registries);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let states = states(&outcome.stdout);
    for (step, state) in [
        ("github draft v1.4.0", "exists"),
        ("crates.io goodfetch-engine 0.0.0", "published"),
        ("crates.io goodfetch-collie-client 1.4.0", "published"),
        ("crates.io goodfetch-collie 1.4.0", "publish"),
        ("npm @goodfetch/collie-client 1.4.0", "published"),
        ("npm @goodfetch/collie-win32-x64 1.4.0", "published"),
        ("npm @goodfetch/collie 1.4.0", "publish"),
        ("npm @goodfetch/foxhound-darwin-arm64 1.4.0", "publish"),
        ("github tag v1.4.0", "push"),
        ("github release v1.4.0", "publish"),
    ] {
        assert_eq!(states.get(step), Some(&state), "{step}\n{}", outcome.stdout);
    }
}

#[test]
fn plan_leaves_nothing_to_do_once_the_release_is_published() {
    let workspace = prepared("1.4.0");
    let mut published = Published::previous_release();
    published.release = Some(false);
    published.tag = true;
    for name in [
        "goodfetch-collie",
        "goodfetch-collie-client",
        "goodfetch-foxhound",
    ] {
        published.crate_out(name, "1.4.0");
    }
    for name in npm_packages() {
        published.npm_out(&name, "1.4.0");
    }
    let registries = Registries::serving(&published);

    let outcome = plan(&workspace, "1.4.0", &registries);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let states = states(&outcome.stdout);
    let left: Vec<_> = states
        .iter()
        .filter(|(_, state)| !matches!(**state, "published" | "pushed"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    assert_eq!(states.len(), 1 + CRATES.len() + 15 + 1 + 1);
}

#[test]
fn plan_flags_each_name_not_on_its_registry() {
    let workspace = prepared("1.4.0");
    let mut published = Published::previous_release();
    published.crates.remove("goodfetch-foxhound");
    published.npm.remove("@goodfetch/foxhound-linux-x64");
    let registries = Registries::serving(&published);

    let outcome = plan(&workspace, "1.4.0", &registries);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(
        outcome.findings(),
        [
            finding("new-name", "crates.io goodfetch-foxhound"),
            finding("new-name", "npm @goodfetch/foxhound-linux-x64"),
        ]
    );
}

#[test]
fn plan_publishes_no_crate_for_a_nightly() {
    let workspace = prepared("1.5.0-dev.7");
    let registries = Registries::serving(&Published::previous_release());

    let outcome = plan(&workspace, "1.5.0-dev.7", &registries);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let states = states(&outcome.stdout);
    assert!(
        states
            .keys()
            .all(|step| step.starts_with("npm ") || step.starts_with("github ")),
        "{}",
        outcome.stdout
    );
    assert_eq!(
        states.get("npm @goodfetch/foxhound 1.5.0-dev.7"),
        Some(&"publish")
    );
    assert_eq!(registries.cargo.requests(), Vec::<Request>::new());
}

#[test]
fn plan_refuses_a_version_the_released_crates_were_not_prepared_at() {
    let workspace = prepared("1.4.0");
    let registries = Registries::serving(&Published::previous_release());

    let outcome = plan(&workspace, "1.4.1", &registries);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(
        outcome.findings(),
        [
            finding("unprepared", "crates/goodfetch-collie-client/Cargo.toml"),
            finding("unprepared", "crates/goodfetch-collie/Cargo.toml"),
            finding(
                "unprepared",
                "crates/foxhound/goodfetch-foxhound/Cargo.toml"
            ),
        ]
    );
}
