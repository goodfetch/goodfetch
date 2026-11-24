//! The released artifacts the skew and upgrade tests run (ADR 0066): every
//! file `compat/releases.toml` lists, both Products' of each release (ADR
//! 0094), with its Sigstore attestation bundle. `fetch` downloads them from
//! the pinned artifact mirror before ADR 0043's deny, with Sigstore's trusted
//! root, checks each SHA-256 and verifies each attestation offline; `verify`
//! checks a kept set again without the network, as after a CI cache restores
//! it. `just artifacts` is the local command.
//!
//! A set is kept under `target/artifacts/<the manifest's SHA-256>/`, each
//! file at `<version>/<group>/<kind>/[<target>/]<file>`, its bundle beside it
//! as `<file>.sigstore.json` or `.sigstore.jsonl`. Its `versions.json` lists
//! the versions the harnesses run: each release and its folder, or, while the
//! list is empty, the current build of both Products alone, so every harness
//! runs before the first release. A missing or mismatched artifact and a
//! refused attestation fail; nothing is skipped.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode};

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::releases::{Attestation, HttpsUrl, ReleasesFile, Sha256 as Pinned};
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::output;

/// The release list, from the workspace's folder.
const MANIFEST: &str = "compat/releases.toml";

/// Where the sets are kept, from the workspace's folder.
const CACHE: &str = "target/artifacts";

/// The list of versions in a set's folder.
const VERSIONS: &str = "versions.json";

/// Sigstore's trusted root in a set's folder, as `gh attestation
/// trusted-root` prints it.
const TRUSTED_ROOT: &str = "trusted_root.jsonl";

/// The repository whose workflows sign every release.
const REPOSITORY: &str = "goodfetch/goodfetch";

/// The largest file kept: an OCI image's archive is the largest artifact.
const MAXIMUM_DOWNLOAD: u64 = 8 << 30;

/// The command that fetches a set, for a finding.
const FETCH: &str = "just artifacts";

#[derive(clap::Args)]
pub(crate) struct Set {
    /// The release list, from the workspace's folder.
    #[arg(long, default_value = MANIFEST)]
    manifest: PathBuf,
    /// Where the sets are kept, one folder per manifest's SHA-256.
    #[arg(long, default_value = CACHE)]
    cache: PathBuf,
}

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Keeps each listed artifact and its attestation bundle, downloading
    /// only a file that lacks its SHA-256, and Sigstore's trusted root, then
    /// verifies each attestation offline with `gh attestation verify`;
    /// prints the set's `versions.json`. A second fetch of one manifest uses
    /// the kept set.
    Fetch {
        #[command(flatten)]
        set: Set,
        /// Downloads each URL's path from this origin in place of its own: a
        /// CI cache of the pinned artifact mirror, or a test's stand-in.
        #[arg(long)]
        mirror: Option<String>,
    },
    /// Checks the kept set offline: each file's SHA-256 and each attestation
    /// against the kept trusted root; prints the set's `versions.json`.
    Verify {
        #[command(flatten)]
        set: Set,
    },
}

/// The harnesses' list of versions.
#[derive(Serialize)]
struct Versions {
    versions: Vec<Version>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Version {
    /// The current build of both Products, alone while nothing is released.
    Current,
    /// A release, its files in `folder`, from the set's folder.
    Release { version: String, folder: String },
}

/// One listed artifact.
struct Kept<'a> {
    /// Where it is kept, from the set's folder.
    path: String,
    url: &'a HttpsUrl,
    sha256: &'a Pinned,
    attestation: &'a Attestation,
}

impl Kept<'_> {
    /// Where its bundle is kept, from the set's folder: `gh` reads a bundle
    /// by its extension.
    fn bundle(&self) -> String {
        let lines = Path::new(path_of(&self.attestation.url))
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"));
        format!(
            "{}.sigstore.{}",
            self.path,
            if lines { "jsonl" } else { "json" }
        )
    }
}

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    let (set, mirror) = match command {
        Command::Fetch { set, mirror } => (set, Some(mirror.as_deref())),
        Command::Verify { set } => (set, None),
    };
    let manifest = environment.path(&set.manifest);
    let bytes =
        std::fs::read(&manifest).with_context(|| format!("reading {}", manifest.display()))?;
    let releases: ReleasesFile = toml::from_str(
        std::str::from_utf8(&bytes).with_context(|| format!("reading {}", manifest.display()))?,
    )
    .with_context(|| format!("parsing {}", manifest.display()))?;
    let cache = environment.path(&set.cache);
    let folder = std::path::absolute(&cache)
        .with_context(|| format!("resolving {}", cache.display()))?
        .join(hex(&Sha256::digest(&bytes)));
    let kept = listed(&releases)?;
    if !kept.is_empty() {
        let findings = match mirror {
            Some(mirror) => fetch(&folder, &kept, mirror)?,
            None => verify(&folder, &kept)?,
        };
        if !findings.is_empty() {
            return Ok(check::report(&findings));
        }
    }
    let versions = Versions {
        versions: if releases.release.is_empty() {
            vec![Version::Current]
        } else {
            releases
                .release
                .iter()
                .map(|release| Version::Release {
                    version: release.version.to_string(),
                    folder: release.version.to_string(),
                })
                .collect()
        },
    };
    std::fs::create_dir_all(&folder).with_context(|| format!("creating {}", folder.display()))?;
    let path = folder.join(VERSIONS);
    let text = serde_json::to_string_pretty(&versions).context("writing the versions")? + "\n";
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    output::line(&path.display().to_string());
    Ok(ExitCode::SUCCESS)
}

/// Every artifact of `releases`, in the file's order. Two kept at one path
/// are refused: the list names one file twice.
fn listed<'a>(releases: &'a ReleasesFile) -> Result<Vec<Kept<'a>>> {
    let mut kept = Vec::new();
    let mut add = |folder: String,
                   url: &'a HttpsUrl,
                   sha256: &'a Pinned,
                   attestation: &'a Attestation|
     -> Result<()> {
        kept.push(Kept {
            path: format!("{folder}/{}", file_name(url)?),
            url,
            sha256,
            attestation,
        });
        Ok(())
    };
    for release in &releases.release {
        let version = &release.version;
        for (group, product) in [("collie", &release.collie), ("foxhound", &release.foxhound)] {
            for executable in &product.executables {
                add(
                    format!(
                        "{version}/{group}/executables/{}",
                        plain(&executable.target)?
                    ),
                    &executable.url,
                    &executable.sha256,
                    &executable.attestation,
                )?;
            }
            for (kind, packages) in [
                ("npm", &product.npm),
                ("pypi", &product.pypi),
                ("crates_io", &product.crates_io),
            ] {
                for package in packages {
                    let folder = format!("{version}/{group}/{kind}");
                    add(folder, &package.url, &package.sha256, &package.attestation)?;
                }
            }
            for (kind, artifact) in [
                ("helm_chart", &product.helm_chart),
                ("image", &product.image),
            ] {
                let folder = format!("{version}/{group}/{kind}");
                add(
                    folder,
                    &artifact.url,
                    &artifact.sha256,
                    &artifact.attestation,
                )?;
            }
        }
        let client = &release.collie_client;
        for (kind, package) in [
            ("npm", &client.npm),
            ("pypi", &client.pypi),
            ("crates_io", &client.crates_io),
        ] {
            let folder = format!("{version}/collie_client/{kind}");
            add(folder, &package.url, &package.sha256, &package.attestation)?;
        }
        for plugin in &release.plugins {
            add(
                format!("{version}/plugins/{}", plain(&plugin.target)?),
                &plugin.url,
                &plugin.sha256,
                &plugin.attestation,
            )?;
        }
    }
    let mut paths = BTreeSet::new();
    for artifact in &kept {
        if !paths.insert(artifact.path.as_str()) {
            bail!("{MANIFEST} lists two files kept as {}", artifact.path);
        }
    }
    Ok(kept)
}

/// `url`'s path, without its origin, query or fragment.
fn path_of(url: &HttpsUrl) -> &str {
    let rest = url.as_str().trim_start_matches("https://");
    let path = rest.find('/').map_or("/", |at| &rest[at..]);
    path.split(['?', '#']).next().unwrap_or(path)
}

/// The last segment of `url`'s path as a file name every OS takes: each
/// character but an ASCII letter, digit, `.`, `_` and `-` becomes `_`.
fn file_name(url: &HttpsUrl) -> Result<String> {
    let segment = path_of(url).rsplit('/').next().unwrap_or_default();
    let name: String = segment
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect();
    plain(&name).with_context(|| format!("{url} names no file", url = url.as_str()))
}

/// `name` as one path segment: not empty, not `.` or `..`, no separator.
fn plain(name: &str) -> Result<String> {
    if name.is_empty() || name.chars().all(|c| c == '.') || name.contains(['/', '\\']) {
        bail!("`{name}` is no file name");
    }
    Ok(name.to_owned())
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// The SHA-256 of the file at `path`, or `None` when there is none.
fn digest_of(path: &Path) -> Result<Option<String>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let digest = copy(&mut file, None).with_context(|| format!("reading {}", path.display()))?;
    Ok(Some(digest))
}

/// Reads `reader` to its end, into `sink` when given, and returns the
/// SHA-256 of what it read.
fn copy(reader: &mut impl std::io::Read, mut sink: Option<&mut File>) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(hex(&hasher.finalize()));
        }
        hasher.update(&buffer[..read]);
        if let Some(sink) = sink.as_mut() {
            sink.write_all(&buffer[..read])?;
        }
    }
}

/// Downloads the set into `folder` unless every file is kept with its
/// SHA-256 and `versions.json` says a fetch passed, then verifies each
/// attestation.
fn fetch(folder: &Path, kept: &[Kept<'_>], mirror: Option<&str>) -> Result<Vec<Finding>> {
    let versions = folder.join(VERSIONS);
    if versions.is_file() && complete(folder, kept)? {
        return Ok(Vec::new());
    }
    if versions.exists() {
        std::fs::remove_file(&versions)
            .with_context(|| format!("removing {}", versions.display()))?;
    }
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    trusted_root(folder)?;
    let mut findings = Vec::new();
    for artifact in kept {
        let file = download(
            folder,
            &artifact.path,
            artifact.url,
            artifact.sha256,
            mirror,
        )?;
        let bundle = download(
            folder,
            &artifact.bundle(),
            &artifact.attestation.url,
            &artifact.attestation.sha256,
            mirror,
        )?;
        match (file, bundle) {
            (None, None) => findings.extend(attested(folder, artifact)?),
            (file, bundle) => findings.extend(file.into_iter().chain(bundle)),
        }
    }
    Ok(findings)
}

/// Whether every file and bundle is kept with its SHA-256, beside the trusted
/// root.
fn complete(folder: &Path, kept: &[Kept<'_>]) -> Result<bool> {
    if !folder.join(TRUSTED_ROOT).is_file() {
        return Ok(false);
    }
    for artifact in kept {
        for (path, sha256) in [
            (artifact.path.clone(), artifact.sha256),
            (artifact.bundle(), &artifact.attestation.sha256),
        ] {
            if digest_of(&folder.join(path))?.as_deref() != Some(sha256.as_str()) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Keeps Sigstore's trusted root, as `gh attestation trusted-root` prints it.
fn trusted_root(folder: &Path) -> Result<()> {
    let output = Process::new("gh")
        .args(["attestation", "trusted-root"])
        .output()
        .context("running `gh attestation trusted-root`: install GitHub's CLI")?;
    if !output.status.success() {
        bail!(
            "`gh attestation trusted-root` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let path = folder.join(TRUSTED_ROOT);
    let partial = folder.join(format!("{TRUSTED_ROOT}.partial"));
    std::fs::write(&partial, &output.stdout)
        .with_context(|| format!("writing {}", partial.display()))?;
    std::fs::rename(&partial, &path).with_context(|| format!("writing {}", path.display()))
}

/// Keeps `url`'s file at `relative` in `folder`, from `mirror` when given,
/// unless it is kept with `sha256` already; a file the mirror lacks or serves
/// with another SHA-256 is a finding, and leaves nothing kept.
fn download(
    folder: &Path,
    relative: &str,
    url: &HttpsUrl,
    sha256: &Pinned,
    mirror: Option<&str>,
) -> Result<Option<Finding>> {
    let path = folder.join(relative);
    if digest_of(&path)?.as_deref() == Some(sha256.as_str()) {
        return Ok(None);
    }
    let source = match mirror {
        Some(mirror) => format!("{}{}", mirror.trim_end_matches('/'), path_of(url)),
        None => url.as_str().to_owned(),
    };
    let response = match ureq::get(&source).call() {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404)) => {
            return Ok(Some(Finding::new(
                "missing-artifact",
                relative,
                format!(
                    "{source} is not found: the pinned artifact mirror must hold every file {MANIFEST} lists"
                ),
            )));
        }
        Err(error) => return Err(error).with_context(|| format!("downloading {source}")),
    };
    let parent = path.parent().context("a kept file sits in a folder")?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let partial = PathBuf::from(format!("{}.partial", path.display()));
    let mut file =
        File::create(&partial).with_context(|| format!("writing {}", partial.display()))?;
    let mut reader = response
        .into_body()
        .into_with_config()
        .limit(MAXIMUM_DOWNLOAD)
        .reader();
    let served = copy(&mut reader, Some(&mut file))
        .with_context(|| format!("downloading {source} into {}", partial.display()))?;
    drop(file);
    if served != sha256.as_str() {
        std::fs::remove_file(&partial)
            .with_context(|| format!("removing {}", partial.display()))?;
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
        return Ok(Some(Finding::new(
            "digest-mismatch",
            relative,
            format!(
                "{source} served SHA-256 {served}, not the {} {MANIFEST} lists",
                sha256.as_str()
            ),
        )));
    }
    std::fs::rename(&partial, &path).with_context(|| format!("writing {}", path.display()))?;
    Ok(None)
}

/// Checks the kept set without the network.
fn verify(folder: &Path, kept: &[Kept<'_>]) -> Result<Vec<Finding>> {
    let not_fetched = |subject: &str| {
        Finding::new(
            "not-fetched",
            subject,
            format!("it is not kept: run `{FETCH}` before the deny"),
        )
    };
    let mut findings = Vec::new();
    let root = folder.join(TRUSTED_ROOT).is_file();
    if !root {
        findings.push(not_fetched(TRUSTED_ROOT));
    }
    for artifact in kept {
        let bundle = artifact.bundle();
        let mut sound = true;
        let mut missing = false;
        for (relative, sha256) in [
            (artifact.path.as_str(), artifact.sha256),
            (bundle.as_str(), &artifact.attestation.sha256),
        ] {
            match digest_of(&folder.join(relative))? {
                None => missing = true,
                Some(digest) if digest != sha256.as_str() => {
                    sound = false;
                    findings.push(Finding::new(
                        "digest-mismatch",
                        relative,
                        format!(
                            "its SHA-256 is {digest}, not the {} {MANIFEST} lists: run `{FETCH}` again",
                            sha256.as_str()
                        ),
                    ));
                }
                Some(_) => {}
            }
        }
        if missing {
            findings.push(not_fetched(&artifact.path));
        } else if sound && root {
            findings.extend(attested(folder, artifact)?);
        }
    }
    Ok(findings)
}

/// A finding when `gh attestation verify` refuses the kept file's bundle
/// against the kept trusted root, which it checks offline.
fn attested(folder: &Path, artifact: &Kept<'_>) -> Result<Option<Finding>> {
    let output = Process::new("gh")
        .args(["attestation", "verify"])
        .arg(folder.join(&artifact.path))
        .arg("--bundle")
        .arg(folder.join(artifact.bundle()))
        .arg("--custom-trusted-root")
        .arg(folder.join(TRUSTED_ROOT))
        .args(["--repo", REPOSITORY])
        .output()
        .context("running `gh attestation verify`: install GitHub's CLI")?;
    Ok((!output.status.success()).then(|| {
        Finding::new(
            "attestation-refused",
            artifact.path.as_str(),
            format!(
                "`gh attestation verify` refused its bundle: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        )
    }))
}
