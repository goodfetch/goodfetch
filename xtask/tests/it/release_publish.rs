//! `xtask release plan`: both Products' publish as one step per registry and
//! package, in the order ADR 0094 sets, each read from its registry's public
//! interface. What is already out is skipped, so a re-run finishes a partial
//! publish; a name not on its registry yet is flagged, since trusted
//! publishing needs a first publish by hand (spec #130).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use rstest::rstest;

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

const PYPI_PROJECTS: [&str; 3] = [
    "goodfetch-collie",
    "goodfetch-collie-client",
    "goodfetch-foxhound",
];

/// One GiB, PyPI's unit for a project's storage limit.
const GIB: u64 = 1 << 30;

/// Each Product's image repository on GHCR (ADR 0090).
const IMAGES: [&str; 2] = ["goodfetch/collie", "goodfetch/foxhound"];

/// ADR 0049's tags of each image, after its version.
const VARIANTS: [&str; 5] = ["", "-slim", "-full", "-node", "-python"];

/// The fixture's first-party Plugin's repository (ADR 0033).
const PLUGIN: &str = "goodfetch/goodfetch-plugin-archive";

/// Every OCI repository of a stable release: each Product's image and
/// chart, and the Plugin.
fn oci_repositories() -> Vec<String> {
    let mut repositories: Vec<String> = IMAGES.iter().map(|image| (*image).to_owned()).collect();
    repositories.extend(
        IMAGES
            .iter()
            .map(|image| image.replace("goodfetch/", "goodfetch/charts/")),
    );
    repositories.push(PLUGIN.to_owned());
    repositories
}

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
    /// Each project's published versions, each with its files' sizes.
    pypi: BTreeMap<String, Vec<(String, Vec<u64>)>>,
    /// Each OCI repository's tags, each with the digest of the manifest it
    /// points at.
    oci: BTreeMap<String, BTreeMap<String, String>>,
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
        let pypi = PYPI_PROJECTS
            .iter()
            .map(|name| ((*name).to_owned(), vec![("1.3.0".to_owned(), vec![GIB])]))
            .collect();
        let mut published = Self {
            crates,
            npm,
            pypi,
            oci: BTreeMap::new(),
            release: None,
            tag: false,
        };
        published.oci_release("1.3.0");
        published
    }

    /// Every step of both Products out at `1.4.0`: a draft GitHub Release
    /// holding their files, and the tag.
    fn every_step_out() -> Self {
        let mut published = Self::previous_release();
        published.release = Some(true);
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
        for project in PYPI_PROJECTS {
            published.pypi_out(project, "1.4.0", vec![GIB]);
        }
        published.oci_release("1.4.0");
        published.tag = true;
        published
    }

    /// Each OCI repository's tags of the stable `version`, `latest` moved to
    /// each image's `version`.
    fn oci_release(&mut self, version: &str) {
        for repository in oci_repositories() {
            let variants = if IMAGES.contains(&repository.as_str()) {
                &VARIANTS[..]
            } else {
                &VARIANTS[..1]
            };
            for variant in variants {
                self.oci_out(&repository, &format!("{version}{variant}"));
            }
        }
        for image in IMAGES {
            self.oci_moved(image, "latest", version);
        }
    }

    /// Adds `tag` to the repository, at a manifest of its own.
    fn oci_out(&mut self, repository: &str, tag: &str) {
        self.oci
            .entry(repository.to_owned())
            .or_default()
            .insert(tag.to_owned(), manifest_digest(repository, tag));
    }

    /// Points the moving tag at the manifest `tag` points at.
    fn oci_moved(&mut self, repository: &str, moving: &str, tag: &str) {
        self.oci
            .entry(repository.to_owned())
            .or_default()
            .insert(moving.to_owned(), manifest_digest(repository, tag));
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

    fn pypi_out(&mut self, project: &str, version: &str, sizes: Vec<u64>) {
        self.pypi
            .entry(project.to_owned())
            .or_default()
            .push((version.to_owned(), sizes));
    }
}

/// A digest of its own for each tag's manifest, as a registry would answer.
fn manifest_digest(repository: &str, tag: &str) -> String {
    format!("sha256:{}-{tag}", repository.replace('/', "-"))
}

/// The registries' stand-ins, serving what [`Published`] holds: a static
/// sparse Cargo index served from a temporary folder, npm's and PyPI's JSON
/// documents, an OCI registry's tag lists and manifest digests, and GitHub.
struct Registries {
    _index: tempfile::TempDir,
    cargo: GitHub,
    npm: GitHub,
    pypi: GitHub,
    oci: GitHub,
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

        let pypi: Vec<(String, String)> = published
            .pypi
            .iter()
            .map(|(project, releases)| {
                let listed: Vec<String> = releases
                    .iter()
                    .map(|(version, sizes)| {
                        let files: Vec<String> = sizes
                            .iter()
                            .map(|size| {
                                format!(r#"{{"filename":"{project}-{version}.whl","size":{size}}}"#)
                            })
                            .collect();
                        format!(r#""{version}":[{}]"#, files.join(","))
                    })
                    .collect();
                (
                    format!("/{project}/json"),
                    format!(
                        r#"{{"info":{{"name":"{project}"}},"releases":{{{}}}}}"#,
                        listed.join(",")
                    ),
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

        let mut oci = Vec::new();
        let mut digests = Vec::new();
        for (repository, tags) in &published.oci {
            let names: Vec<String> = tags.keys().map(|tag| format!(r#""{tag}""#)).collect();
            oci.push((
                format!("/v2/{repository}/tags/list"),
                format!(r#"{{"name":"{repository}","tags":[{}]}}"#, names.join(",")),
            ));
            for (tag, digest) in tags {
                let path = format!("/v2/{repository}/manifests/{tag}");
                oci.push((path.clone(), "{}".to_owned()));
                digests.push((path, format!("docker-content-digest: {digest}")));
            }
        }
        let oci_routes: Vec<(&str, &str)> = oci
            .iter()
            .map(|(path, body)| (path.as_str(), body.as_str()))
            .collect();
        let digest_headers: Vec<(&str, &str)> = digests
            .iter()
            .map(|(path, header)| (path.as_str(), header.as_str()))
            .collect();

        Self {
            _index: index,
            cargo,
            npm: serve(&npm),
            pypi: serve(&pypi),
            oci: GitHub::serving_with_headers(&oci_routes, &digest_headers),
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
    plan_with(workspace, version, registries, &[])
}

/// The plan, reading each registry its option names in `instead` at the
/// address beside it, such as Verdaccio's for `--npm-registry`, and every
/// other at its stand-in in `registries`.
fn plan_with(
    workspace: &Workspace,
    version: &str,
    registries: &Registries,
    instead: &[(&str, &str)],
) -> Outcome {
    let mut arguments = vec!["release", "plan", "--version", version];
    for (option, stand_in) in [
        ("--cargo-index", &registries.cargo),
        ("--npm-registry", &registries.npm),
        ("--pypi", &registries.pypi),
        ("--oci-registry", &registries.oci),
        ("--api-url", &registries.github),
    ] {
        let address = instead
            .iter()
            .find(|(name, _)| *name == option)
            .map_or(stand_in.url(), |(_, address)| *address);
        arguments.extend([option, address]);
    }
    workspace.xtask_with_env(&arguments, &[("GITHUB_TOKEN", "a-read-token".as_ref())])
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
    for registry in [
        &registries.github,
        &registries.npm,
        &registries.pypi,
        &registries.cargo,
        &registries.oci,
    ] {
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
        ("github tag v1.4.0", "held"),
        ("pypi goodfetch-collie 1.4.0", "publish"),
        ("github release v1.4.0", "held"),
    ] {
        assert_eq!(states.get(step), Some(&state), "{step}\n{}", outcome.stdout);
    }
}

/// Every step of both Products out but `left`, whose registry still lacks
/// it.
fn published_but(left: &str) -> Published {
    let mut published = Published::every_step_out();
    match left.split_once(' ') {
        Some(("npm", name)) => {
            published
                .npm
                .get_mut(name)
                .unwrap()
                .retain(|v| v != "1.4.0");
            published.tag = false;
        }
        Some(("pypi", name)) => {
            published
                .pypi
                .get_mut(name)
                .unwrap()
                .retain(|(v, _)| v != "1.4.0");
        }
        Some(("crates.io", name)) => {
            published
                .crates
                .get_mut(name)
                .unwrap()
                .retain(|v| v != "1.4.0");
        }
        Some(("oci", step)) => {
            let (repository, tag) = step.split_once(' ').unwrap();
            published
                .oci
                .get_mut(repository)
                .unwrap()
                .remove(tag)
                .unwrap();
        }
        Some(("github", "tag")) => published.tag = false,
        _ => unreachable!("{left}"),
    }
    published
}

/// A step of either Product left holds what waits on both: the tag waits
/// on every npm package of both Products, the GitHub Release's publishing
/// on every other step (ADR 0094).
#[rstest]
#[case::foxhound_npm_root("npm @goodfetch/foxhound", "held", "held")]
#[case::collie_platform_package("npm @goodfetch/collie-linux-arm64", "held", "held")]
#[case::collie_wheel("pypi goodfetch-collie", "pushed", "held")]
#[case::foxhound_crate("crates.io goodfetch-foxhound", "pushed", "held")]
#[case::collie_image_tag("oci goodfetch/collie 1.4.0-python", "pushed", "held")]
#[case::foxhound_chart("oci goodfetch/charts/foxhound 1.4.0", "pushed", "held")]
#[case::plugin("oci goodfetch/goodfetch-plugin-archive 1.4.0", "pushed", "held")]
#[case::foxhound_latest("oci goodfetch/foxhound latest", "pushed", "held")]
#[case::tag_alone("github tag", "push", "held")]
fn plan_holds_what_waits_on_a_step_either_product_left(
    #[case] left: &str,
    #[case] tag: &str,
    #[case] release: &str,
) {
    let workspace = prepared("1.4.0");
    let registries = Registries::serving(&published_but(left));

    let outcome = plan(&workspace, "1.4.0", &registries);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let states = states(&outcome.stdout);
    assert_eq!(
        states.get("github tag v1.4.0"),
        Some(&tag),
        "{}",
        outcome.stdout
    );
    assert_eq!(
        states.get("github release v1.4.0"),
        Some(&release),
        "{}",
        outcome.stdout
    );
}

#[test]
fn plan_publishes_the_release_once_every_other_step_is_out() {
    let workspace = prepared("1.4.0");
    let mut published = published_but("github tag");
    published.tag = true;
    let registries = Registries::serving(&published);

    let outcome = plan(&workspace, "1.4.0", &registries);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        states(&outcome.stdout).get("github release v1.4.0"),
        Some(&"publish"),
        "{}",
        outcome.stdout
    );
}

#[test]
fn plan_leaves_nothing_to_do_once_the_release_is_published() {
    let workspace = prepared("1.4.0");
    let mut published = Published::every_step_out();
    published.release = Some(false);
    let registries = Registries::serving(&published);

    let outcome = plan(&workspace, "1.4.0", &registries);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let states = states(&outcome.stdout);
    let left: Vec<_> = states
        .iter()
        .filter(|(_, state)| !matches!(**state, "published" | "pushed"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    // Each image's six tags and its chart, and the Plugin.
    let oci = 2 * (6 + 1) + 1;
    assert_eq!(states.len(), 1 + CRATES.len() + 15 + 1 + 3 + oci + 1);
}

#[test]
fn plan_flags_each_name_not_on_its_registry() {
    let workspace = prepared("1.4.0");
    let mut published = Published::previous_release();
    published.crates.remove("goodfetch-foxhound");
    published.npm.remove("@goodfetch/foxhound-linux-x64");
    published.pypi.remove("goodfetch-foxhound");
    let registries = Registries::serving(&published);

    let outcome = plan(&workspace, "1.4.0", &registries);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(
        outcome.findings(),
        [
            finding("new-name", "crates.io goodfetch-foxhound"),
            finding("new-name", "npm @goodfetch/foxhound-linux-x64"),
            finding("new-name", "pypi goodfetch-foxhound"),
        ]
    );
}

/// The fixture records 10 GiB for Collie's project and 20 GiB for
/// Foxhound's, so a project read against the other's limit shows.
#[rstest]
#[case::collie_under("goodfetch-collie", 10, 69, false)]
#[case::collie_past("goodfetch-collie", 10, 71, true)]
#[case::foxhound_under("goodfetch-foxhound", 20, 69, false)]
#[case::foxhound_past("goodfetch-foxhound", 20, 71, true)]
fn plan_warns_when_a_pypi_project_passes_70_percent_of_its_quota(
    #[case] project: &str,
    #[case] limit_gib: u64,
    #[case] percent: u64,
    #[case] warned: bool,
) {
    let workspace = prepared("1.4.0");
    let mut published = Published::previous_release();
    // The sizes of every file of every version count, the previous
    // release's 1 GiB with them.
    let used = limit_gib * GIB * percent / 100;
    published.pypi_out(project, "1.2.0", vec![used / 2, used / 2 - GIB]);
    let registries = Registries::serving(&published);

    let outcome = plan(&workspace, "1.4.0", &registries);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let expected = if warned {
        vec![finding("pypi-quota", project)]
    } else {
        Vec::new()
    };
    assert_eq!(outcome.notes(), expected);
}

/// A nightly publishes npm packages and each image's version and `nightly`
/// tags: no crates, wheels, image variants, charts or Plugins (ADRs 0030,
/// 0057, 0094).
#[test]
fn plan_publishes_neither_crates_nor_wheels_for_a_nightly() {
    let workspace = prepared("1.5.0-dev.7");
    let registries = Registries::serving(&Published::previous_release());

    let outcome = plan(&workspace, "1.5.0-dev.7", &registries);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let states = states(&outcome.stdout);
    assert!(
        states.keys().all(|step| step.starts_with("npm ")
            || step.starts_with("github ")
            || step.starts_with("oci ")),
        "{}",
        outcome.stdout
    );
    assert_eq!(
        states.get("npm @goodfetch/foxhound 1.5.0-dev.7"),
        Some(&"publish")
    );
    let oci: Vec<&str> = states
        .keys()
        .copied()
        .filter(|step| step.starts_with("oci "))
        .collect();
    assert_eq!(
        oci,
        [
            "oci goodfetch/collie 1.5.0-dev.7",
            "oci goodfetch/collie nightly",
            "oci goodfetch/foxhound 1.5.0-dev.7",
            "oci goodfetch/foxhound nightly",
        ]
    );
    assert_eq!(registries.pypi.requests(), Vec::<Request>::new());
    assert_eq!(registries.cargo.requests(), Vec::<Request>::new());
}

/// `latest` moves to a stable version no stable tag of the image passes; a
/// release candidate never moves it (ADR 0049).
#[rstest]
#[case::above_every_stable("1.4.0", "1.3.1", Some("publish"))]
#[case::below_a_newer_stable("1.4.0", "1.5.0", None)]
#[case::below_a_newer_candidate("1.4.0", "1.5.0-rc.1", Some("publish"))]
#[case::a_candidate("1.5.0-rc.1", "1.4.1", None)]
fn plan_moves_latest_to_a_stable_version_no_stable_tag_passes(
    #[case] version: &str,
    #[case] other: &str,
    #[case] latest: Option<&str>,
) {
    let workspace = prepared(version);
    let mut published = Published::previous_release();
    for image in IMAGES {
        published.oci_out(image, other);
    }
    let registries = Registries::serving(&published);

    let outcome = plan(&workspace, version, &registries);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let states = states(&outcome.stdout);
    for image in IMAGES {
        assert_eq!(
            states.get(format!("oci {image} latest").as_str()).copied(),
            latest,
            "{}",
            outcome.stdout
        );
    }
}

/// A moving tag is out only once it points at the version's manifest.
#[test]
fn plan_moves_latest_still_at_the_previous_version() {
    let workspace = prepared("1.4.0");
    let mut published = Published::every_step_out();
    published.oci_moved("goodfetch/foxhound", "latest", "1.3.0");
    let registries = Registries::serving(&published);

    let outcome = plan(&workspace, "1.4.0", &registries);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let states = states(&outcome.stdout);
    assert_eq!(
        states.get("oci goodfetch/foxhound latest"),
        Some(&"publish")
    );
    assert_eq!(
        states.get("oci goodfetch/collie latest"),
        Some(&"published")
    );
    assert_eq!(states.get("github release v1.4.0"), Some(&"held"));
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

/// Verdaccio, the npm registry the TypeScript tests install from, run on a
/// free local port with anonymous publishing and no uplink.
struct Verdaccio {
    child: Child,
    url: String,
    _folder: tempfile::TempDir,
}

impl Verdaccio {
    fn start() -> Self {
        let binary = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test-support/typescript/node_modules/.bin/verdaccio");
        assert!(
            binary.exists(),
            "{} is missing: run `pnpm install` at the repository's root",
            binary.display()
        );
        let folder = tempfile::tempdir().unwrap();
        let config = folder.path().join("config.yaml");
        std::fs::write(
            &config,
            format!(
                "storage: {storage}\nauth:\n  htpasswd:\n    file: {storage}/htpasswd\npackages:\n  '**':\n    access: $all\n    publish: $all\nlog: {{ type: stdout, format: pretty, level: warn }}\n",
                storage = folder.path().display()
            ),
        )
        .unwrap();
        let port = free_port();
        let url = format!("http://127.0.0.1:{port}");
        let log = folder.path().join("verdaccio.log");
        let output = std::fs::File::create(&log).unwrap();
        let child = Command::new(binary)
            .arg("--config")
            .arg(&config)
            .arg("--listen")
            .arg(format!("127.0.0.1:{port}"))
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .spawn()
            .unwrap();
        let mut verdaccio = Self {
            child,
            url,
            _folder: folder,
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        while ureq::get(&format!("{}/-/ping", verdaccio.url))
            .call()
            .is_err()
        {
            let exited = verdaccio.child.try_wait().unwrap();
            assert!(
                exited.is_none() && Instant::now() < deadline,
                "Verdaccio did not start ({exited:?}):\n{}",
                std::fs::read_to_string(&log).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        verdaccio
    }

    /// Publishes `name` at `version` through npm's publish interface.
    fn publish(&self, name: &str, version: &str) {
        let tarball = package_tarball(name, version);
        let body = serde_json::json!({
            "_id": name,
            "name": name,
            "dist-tags": { "latest": version },
            "versions": { version: { "name": name, "version": version, "dist": {
                "tarball": format!("{}/{name}/-/package-{version}.tgz", self.url),
            } } },
            "_attachments": { format!("package-{version}.tgz"): {
                "content_type": "application/octet-stream",
                "data": data_encoding::BASE64.encode(&tarball),
                "length": tarball.len(),
            } },
        });
        ureq::put(&format!("{}/{}", self.url, name.replace('/', "%2f")))
            .send_json(body)
            .unwrap();
    }
}

/// A loopback port free when asked, for Verdaccio to listen on.
#[expect(
    clippy::disallowed_types,
    reason = "a test asks its loopback for a free port for a stand-in; ADR 0060's sockets govern product code"
)]
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

impl Drop for Verdaccio {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A package's gzipped tarball holding its `package/package.json`.
fn package_tarball(name: &str, version: &str) -> Vec<u8> {
    let manifest = format!(r#"{{"name":"{name}","version":"{version}"}}"#);
    let mut header = tar::Header::new_gnu();
    header.set_size(manifest.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    archive
        .append_data(&mut header, "package/package.json", manifest.as_bytes())
        .unwrap();
    archive.into_inner().unwrap().finish().unwrap()
}

#[test]
fn plan_reads_npm_packages_through_verdaccio() {
    let workspace = prepared("1.4.0");
    let registries = Registries::serving(&Published::previous_release());
    let verdaccio = Verdaccio::start();
    verdaccio.publish("@goodfetch/collie-client", "1.4.0");
    for name in npm_packages().iter().skip(1) {
        verdaccio.publish(name, "1.3.0");
    }

    let outcome = plan_with(
        &workspace,
        "1.4.0",
        &registries,
        &[("--npm-registry", &verdaccio.url)],
    );

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let states = states(&outcome.stdout);
    assert_eq!(
        states.get("npm @goodfetch/collie-client 1.4.0"),
        Some(&"published")
    );
    assert_eq!(
        states.get("npm @goodfetch/collie-linux-x64 1.4.0"),
        Some(&"publish")
    );
    assert_eq!(
        states.get("npm @goodfetch/foxhound 1.4.0"),
        Some(&"publish")
    );
}

#[test]
fn plan_flags_an_npm_package_verdaccio_has_never_seen() {
    let workspace = prepared("1.4.0");
    let registries = Registries::serving(&Published::previous_release());
    let verdaccio = Verdaccio::start();
    for name in npm_packages()
        .iter()
        .filter(|name| *name != "@goodfetch/foxhound")
    {
        verdaccio.publish(name, "1.3.0");
    }

    let outcome = plan_with(
        &workspace,
        "1.4.0",
        &registries,
        &[("--npm-registry", &verdaccio.url)],
    );

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(
        outcome.findings(),
        [finding("new-name", "npm @goodfetch/foxhound")]
    );
}

/// Distribution's `registry:2` (`ci/registry/image.toml`), the reference
/// implementation of the OCI Distribution interface GHCR serves, run where
/// the tests run images.
#[cfg(target_os = "linux")]
mod distribution {
    use sha2::{Digest as _, Sha256};

    use super::{Published, Registries, VARIANTS, plan_with, prepared, states};
    use crate::container::Container;

    const PIN: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ci/registry/image.toml"
    ));

    /// Pushes one image manifest, with no layers and a config blob of
    /// `content`, and tags it with each of `tags`, as `docker push` does.
    fn push(url: &str, repository: &str, content: &str, tags: &[&str]) {
        let config = format!(r#"{{"goodfetch":"{content}"}}"#);
        let digest = format!(
            "sha256:{}",
            data_encoding::HEXLOWER.encode(&Sha256::digest(config.as_bytes()))
        );
        let started = ureq::post(&format!("{url}/v2/{repository}/blobs/uploads/"))
            .send_empty()
            .unwrap();
        let location = started
            .headers()
            .get("location")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let location = if location.starts_with("http") {
            location
        } else {
            format!("{url}{location}")
        };
        let separator = if location.contains('?') { '&' } else { '?' };
        ureq::put(&format!("{location}{separator}digest={digest}"))
            .header("Content-Type", "application/octet-stream")
            .send(config.as_bytes())
            .unwrap();
        let manifest = format!(
            r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{{"mediaType":"application/vnd.oci.image.config.v1+json","digest":"{digest}","size":{}}},"layers":[]}}"#,
            config.len()
        );
        for tag in tags {
            ureq::put(&format!("{url}/v2/{repository}/manifests/{tag}"))
                .header("Content-Type", "application/vnd.oci.image.manifest.v1+json")
                .send(manifest.as_bytes())
                .unwrap();
        }
    }

    /// The plan reads each repository's tags and compares `latest`'s
    /// manifest with the version's; a repository the registry lacks is to
    /// publish. One registry serves every case, as starting one is the
    /// test's cost.
    #[test]
    fn plan_reads_oci_tags_through_distribution() {
        let workspace = prepared("1.4.0");
        let registries = Registries::serving(&Published::previous_release());
        let registry = Container::start(PIN, 5000, "/v2/", &[]);
        let url = registry.url();
        for variant in VARIANTS {
            let tag = format!("1.4.0{variant}");
            push(url, "goodfetch/collie", &tag, &[&tag]);
        }
        push(url, "goodfetch/collie", "1.3.0", &["1.3.0", "latest"]);
        push(url, "goodfetch/foxhound", "1.4.0", &["1.4.0", "latest"]);
        push(url, "goodfetch/charts/collie", "1.4.0", &["1.4.0"]);

        let outcome = plan_with(&workspace, "1.4.0", &registries, &[("--oci-registry", url)]);

        assert_eq!(outcome.code, 0, "{}", outcome.stderr);
        let states = states(&outcome.stdout);
        let read: Vec<(&str, &str)> = [
            "oci goodfetch/collie 1.4.0-python",
            "oci goodfetch/collie latest",
            "oci goodfetch/foxhound 1.4.0",
            "oci goodfetch/foxhound 1.4.0-slim",
            "oci goodfetch/foxhound latest",
            "oci goodfetch/charts/collie 1.4.0",
            "oci goodfetch/charts/foxhound 1.4.0",
            "oci goodfetch/goodfetch-plugin-archive 1.4.0",
        ]
        .into_iter()
        .map(|step| (step, states.get(step).copied().unwrap_or("absent")))
        .collect();
        assert_eq!(
            read,
            [
                ("oci goodfetch/collie 1.4.0-python", "published"),
                // At 1.3.0's manifest.
                ("oci goodfetch/collie latest", "publish"),
                ("oci goodfetch/foxhound 1.4.0", "published"),
                ("oci goodfetch/foxhound 1.4.0-slim", "publish"),
                ("oci goodfetch/foxhound latest", "published"),
                ("oci goodfetch/charts/collie 1.4.0", "published"),
                // Repositories the registry never saw.
                ("oci goodfetch/charts/foxhound 1.4.0", "publish"),
                ("oci goodfetch/goodfetch-plugin-archive 1.4.0", "publish"),
            ],
            "{}",
            outcome.stdout
        );
    }
}

/// pypiserver (`ci/pypiserver/image.toml`), whose root serves the
/// `<project>/json` document PyPI serves under `https://pypi.org/pypi`, run
/// where the tests run images. It lists no file sizes: the quota's sums are
/// read from PyPI's own documents, served statically above.
#[cfg(target_os = "linux")]
mod pypiserver {
    use super::{Published, Registries, plan_with, prepared, states};
    use crate::container::Container;
    use crate::workspace::finding;

    const PIN: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ci/pypiserver/image.toml"
    ));

    /// Uploads a wheel of `project` at `version` through the legacy upload
    /// form `twine upload` posts, which pypiserver takes with no password
    /// when started with `-a . -P .`.
    fn upload(url: &str, project: &str, version: &str) {
        let boundary = "goodfetch-upload-boundary";
        let file = format!("{}-{version}-py3-none-any.whl", project.replace('-', "_"));
        let mut parts: Vec<String> = [
            (":action", "file_upload"),
            ("name", project),
            ("version", version),
        ]
        .into_iter()
        .map(|(name, value)| {
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
        })
        .collect();
        parts.push(format!(
            "Content-Disposition: form-data; name=\"content\"; filename=\"{file}\"\r\nContent-Type: application/octet-stream\r\n\r\nnot a real wheel\r\n"
        ));
        let body: String = parts
            .iter()
            .map(|part| format!("--{boundary}\r\n{part}"))
            .chain([format!("--{boundary}--\r\n")])
            .collect();
        ureq::post(&format!("{url}/"))
            .header(
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .send(body.as_bytes())
            .unwrap();
    }

    #[test]
    fn plan_reads_pypi_projects_through_pypiserver() {
        let workspace = prepared("1.4.0");
        let registries = Registries::serving(&Published::previous_release());
        let pypiserver = Container::start(
            PIN,
            8080,
            "/",
            &["run", "-a", ".", "-P", ".", "/data/packages"],
        );
        let url = pypiserver.url();
        upload(url, "goodfetch-collie-client", "1.4.0");
        upload(url, "goodfetch-collie", "1.3.0");

        let outcome = plan_with(&workspace, "1.4.0", &registries, &[("--pypi", url)]);

        assert_eq!(outcome.code, 1, "{}", outcome.stderr);
        let states = states(&outcome.stdout);
        assert_eq!(
            (
                states.get("pypi goodfetch-collie-client 1.4.0"),
                states.get("pypi goodfetch-collie 1.4.0"),
            ),
            (Some(&"published"), Some(&"publish")),
            "{}",
            outcome.stdout
        );
        assert_eq!(
            outcome.findings(),
            [finding("new-name", "pypi goodfetch-foxhound")]
        );
        // pypiserver lists no sizes, so the use of the limit is not summed.
        assert_eq!(outcome.notes(), [finding("pypi-quota", "goodfetch-collie")]);
    }
}
