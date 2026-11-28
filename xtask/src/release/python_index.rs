//! `xtask release python-index`: generates the static PEP 503 HTML and PEP 691
//! JSON simple index served at `https://goodfetch.dev/nightly/simple/` for
//! nightly `.devN` wheels attached to GitHub pre-releases (ADRs 0057, 0090 and
//! 0094; issue 205).
//!
//! Only the newest `--keep` (default 30) nightly pre-releases appear in the
//! index; older GitHub pre-releases are never pruned.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::output;

/// Default number of nightly pre-releases listed in the index (ADR 0057).
const DEFAULT_KEEP: usize = 30;
/// PEP 503 / PEP 691 API version written in both HTML and JSON forms.
const API_VERSION: &str = "1.1";
/// Every Python distribution published on the nightly index, in sorted order.
const PROJECTS: [&str; 3] = [
    "goodfetch-collie",
    "goodfetch-collie-client",
    "goodfetch-foxhound",
];

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// JSON file containing the array of GitHub releases (as `gh api` returns).
    #[arg(long)]
    releases: PathBuf,
    /// Output directory where `simple/` and `vercel.json` are written.
    #[arg(long)]
    out: PathBuf,
    /// Maximum number of newest nightly pre-releases kept in the index.
    #[arg(long, default_value_t = DEFAULT_KEEP)]
    keep: usize,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    published_at: String,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    /// GitHub's asset digest (`sha256:<hex>`), when present.
    #[serde(default)]
    digest: Option<String>,
    /// Explicit 64-hex SHA-256 override for local fixtures.
    #[serde(default)]
    sha256: Option<String>,
}

struct WheelEntry {
    filename: String,
    url: String,
    sha256: String,
    provenance: Option<String>,
}

pub(super) fn run(root: &Path, arguments: &Arguments) -> Result<ExitCode> {
    ensure!(arguments.keep > 0, "--keep must be greater than zero");

    let releases_path = resolve(root, &arguments.releases);
    let out_dir = resolve(root, &arguments.out);
    let raw = std::fs::read_to_string(&releases_path)
        .with_context(|| format!("reading {}", releases_path.display()))?;
    let releases: Vec<GitHubRelease> = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {}", releases_path.display()))?;

    let mut nightlies: Vec<&GitHubRelease> = releases
        .iter()
        .filter(|release| !release.draft && release.prerelease && is_nightly_tag(&release.tag_name))
        .collect();

    // Sort newest first by `published_at`, then by numeric nightly build tag.
    nightlies.sort_by(|left, right| {
        right
            .published_at
            .cmp(&left.published_at)
            .then_with(|| nightly_sort_key(&right.tag_name).cmp(&nightly_sort_key(&left.tag_name)))
            .then_with(|| right.tag_name.cmp(&left.tag_name))
    });
    nightlies.truncate(arguments.keep);

    let mut by_project: BTreeMap<&'static str, Vec<WheelEntry>> = PROJECTS
        .into_iter()
        .map(|project| (project, Vec::new()))
        .collect();

    for release in nightlies {
        let asset_urls: BTreeMap<&str, &str> = release
            .assets
            .iter()
            .map(|asset| (asset.name.as_str(), asset.browser_download_url.as_str()))
            .collect();
        let mut seen = BTreeSet::new();
        for asset in &release.assets {
            let Some(project) = wheel_project(&asset.name) else {
                continue;
            };
            if !seen.insert(asset.name.as_str()) {
                continue;
            }
            let sha256 = extract_sha256(asset, &release.tag_name)?;
            let provenance = asset_urls
                .get(format!("{}.sigstore.json", asset.name).as_str())
                .or_else(|| asset_urls.get(format!("{}.sigstore.jsonl", asset.name).as_str()))
                .map(|url| (*url).to_owned());
            by_project.entry(project).or_default().push(WheelEntry {
                filename: asset.name.clone(),
                url: asset.browser_download_url.clone(),
                sha256,
                provenance,
            });
        }
    }

    for wheels in by_project.values_mut() {
        wheels.sort_by(|left, right| left.filename.cmp(&right.filename));
    }

    write_index(&out_dir, &by_project)?;
    output::line(&arguments.out.display().to_string());
    Ok(ExitCode::SUCCESS)
}

fn resolve(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn is_nightly_tag(tag: &str) -> bool {
    tag.contains("-dev.") || tag.contains(".dev")
}

fn nightly_sort_key(tag: &str) -> (u64, u64, u64, u64) {
    let trimmed = tag.strip_prefix('v').unwrap_or(tag);
    let (base, dev) = trimmed
        .split_once("-dev.")
        .or_else(|| trimmed.split_once(".dev"))
        .unwrap_or((trimmed, "0"));
    let mut nums = base.split('.').map(|part| part.parse::<u64>().unwrap_or(0));
    let major = nums.next().unwrap_or(0);
    let minor = nums.next().unwrap_or(0);
    let patch = nums.next().unwrap_or(0);
    let build = dev.parse::<u64>().unwrap_or(0);
    (major, minor, patch, build)
}

fn wheel_project(filename: &str) -> Option<&'static str> {
    if !Path::new(filename)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("whl"))
    {
        return None;
    }
    if filename.starts_with("goodfetch_collie_client-") {
        Some("goodfetch-collie-client")
    } else if filename.starts_with("goodfetch_collie-") {
        Some("goodfetch-collie")
    } else if filename.starts_with("goodfetch_foxhound-") {
        Some("goodfetch-foxhound")
    } else {
        None
    }
}

fn extract_sha256(asset: &GitHubAsset, tag: &str) -> Result<String> {
    let candidate = asset
        .sha256
        .as_deref()
        .or_else(|| {
            asset
                .digest
                .as_deref()
                .and_then(|d| d.strip_prefix("sha256:").or(Some(d)))
        })
        .map(str::trim);
    let Some(hex) = candidate else {
        bail!("wheel {} in release {tag} has no sha256 digest", asset.name);
    };
    let lower = hex.to_ascii_lowercase();
    ensure!(
        lower.len() == 64 && lower.bytes().all(|b| b.is_ascii_hexdigit()),
        "wheel {} in release {tag} has invalid sha256 digest {hex:?}",
        asset.name
    );
    Ok(lower)
}

fn write_index(out_dir: &Path, by_project: &BTreeMap<&'static str, Vec<WheelEntry>>) -> Result<()> {
    let simple_dir = out_dir.join("simple");
    std::fs::create_dir_all(&simple_dir)
        .with_context(|| format!("creating {}", simple_dir.display()))?;

    // Root PEP 503 HTML and PEP 691 JSON.
    let mut root_html = String::new();
    let _ = writeln!(root_html, "<!DOCTYPE html>");
    let _ = writeln!(root_html, "<html>");
    let _ = writeln!(
        root_html,
        "  <head><meta name=\"pypi:repository-version\" content=\"{API_VERSION}\"><title>Goodfetch Nightly Simple Index</title></head>"
    );
    let _ = writeln!(root_html, "  <body>");
    let mut root_projects = Vec::new();
    for project in by_project.keys() {
        let _ = writeln!(root_html, "    <a href=\"{project}/\">{project}</a><br>");
        root_projects.push(json!({ "name": project }));
    }
    let _ = writeln!(root_html, "  </body>");
    let _ = writeln!(root_html, "</html>");
    std::fs::write(simple_dir.join("index.html"), root_html)?;

    let root_json = json!({
        "meta": { "api-version": API_VERSION },
        "projects": root_projects,
    });
    std::fs::write(
        simple_dir.join("index.json"),
        serde_json::to_string_pretty(&root_json)? + "\n",
    )?;

    // Per-project PEP 503 HTML and PEP 691 JSON.
    for (project, wheels) in by_project {
        let project_dir = simple_dir.join(project);
        std::fs::create_dir_all(&project_dir)
            .with_context(|| format!("creating {}", project_dir.display()))?;

        let mut html = String::new();
        let _ = writeln!(html, "<!DOCTYPE html>");
        let _ = writeln!(html, "<html>");
        let _ = writeln!(
            html,
            "  <head><meta name=\"pypi:repository-version\" content=\"{API_VERSION}\"><title>Links for {project}</title></head>"
        );
        let _ = writeln!(html, "  <body>");
        let _ = writeln!(html, "    <h1>Links for {project}</h1>");

        let mut files_json: Vec<Value> = Vec::with_capacity(wheels.len());
        for wheel in wheels {
            let escaped_url = escape_html(&wheel.url);
            let escaped_name = escape_html(&wheel.filename);
            if let Some(prov) = &wheel.provenance {
                let escaped_prov = escape_html(prov);
                let _ = writeln!(
                    html,
                    "    <a href=\"{escaped_url}#sha256={}\" data-provenance=\"{escaped_prov}\">{escaped_name}</a><br>",
                    wheel.sha256
                );
            } else {
                let _ = writeln!(
                    html,
                    "    <a href=\"{escaped_url}#sha256={}\">{escaped_name}</a><br>",
                    wheel.sha256
                );
            }
            let mut file_obj = json!({
                "filename": wheel.filename,
                "url": wheel.url,
                "hashes": { "sha256": wheel.sha256 },
            });
            if let Some(prov) = &wheel.provenance {
                file_obj["provenance"] = json!(prov);
            }
            files_json.push(file_obj);
        }
        let _ = writeln!(html, "  </body>");
        let _ = writeln!(html, "</html>");
        std::fs::write(project_dir.join("index.html"), html)?;

        let project_json = json!({
            "meta": { "api-version": API_VERSION },
            "name": project,
            "files": files_json,
        });
        std::fs::write(
            project_dir.join("index.json"),
            serde_json::to_string_pretty(&project_json)? + "\n",
        )?;
    }

    write_vercel_config(out_dir)?;
    Ok(())
}

fn write_vercel_config(out_dir: &Path) -> Result<()> {
    // Vercel static config: serves PEP 691 JSON when requested via Accept header
    // and PEP 503 HTML otherwise, with proper PyPI Simple API media types.
    let vercel = json!({
        "cleanUrls": false,
        "trailingSlash": true,
        "rewrites": [
            {
                "source": "/simple/",
                "has": [{ "type": "header", "key": "accept", "value": ".*application/vnd\\.pypi\\.simple\\.v1\\+json.*" }],
                "destination": "/simple/index.json"
            },
            {
                "source": "/simple/:project/",
                "has": [{ "type": "header", "key": "accept", "value": ".*application/vnd\\.pypi\\.simple\\.v1\\+json.*" }],
                "destination": "/simple/:project/index.json"
            }
        ],
        "headers": [
            {
                "source": "/simple/(.*)\\.json",
                "headers": [{ "key": "Content-Type", "value": "application/vnd.pypi.simple.v1+json; charset=utf-8" }]
            },
            {
                "source": "/simple/(.*)\\.html",
                "headers": [{ "key": "Content-Type", "value": "application/vnd.pypi.simple.v1+html; charset=utf-8" }]
            }
        ]
    });
    std::fs::write(
        out_dir.join("vercel.json"),
        serde_json::to_string_pretty(&vercel)? + "\n",
    )?;
    Ok(())
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
