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
        Self {
            crates,
            npm,
            pypi,
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

    fn pypi_out(&mut self, project: &str, version: &str, sizes: Vec<u64>) {
        self.pypi
            .entry(project.to_owned())
            .or_default()
            .push((version.to_owned(), sizes));
    }
}

/// The registries' stand-ins, serving what [`Published`] holds: a static
/// sparse Cargo index served from a temporary folder, npm's and PyPI's JSON
/// documents, and GitHub.
struct Registries {
    _index: tempfile::TempDir,
    cargo: GitHub,
    npm: GitHub,
    pypi: GitHub,
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
                    format!("/pypi/{project}/json"),
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

        Self {
            _index: index,
            cargo,
            npm: serve(&npm),
            pypi: serve(&pypi),
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
    plan_with_npm(workspace, version, registries, registries.npm.url())
}

fn plan_with_npm(
    workspace: &Workspace,
    version: &str,
    registries: &Registries,
    npm: &str,
) -> Outcome {
    workspace.xtask_with_env(
        &[
            "release",
            "plan",
            "--version",
            version,
            "--cargo-index",
            registries.cargo.url(),
            "--npm-registry",
            npm,
            "--pypi",
            registries.pypi.url(),
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
    for registry in [
        &registries.github,
        &registries.npm,
        &registries.pypi,
        &registries.cargo,
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
    let mut published = Published::previous_release();
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
    published.tag = true;
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
    for project in PYPI_PROJECTS {
        published.pypi_out(project, "1.4.0", vec![GIB]);
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
    assert_eq!(states.len(), 1 + CRATES.len() + 15 + 1 + 3 + 1);
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

#[test]
fn plan_publishes_neither_crates_nor_wheels_for_a_nightly() {
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
    assert_eq!(registries.pypi.requests(), Vec::<Request>::new());
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

    let outcome = plan_with_npm(&workspace, "1.4.0", &registries, &verdaccio.url);

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

    let outcome = plan_with_npm(&workspace, "1.4.0", &registries, &verdaccio.url);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(
        outcome.findings(),
        [finding("new-name", "npm @goodfetch/foxhound")]
    );
}
