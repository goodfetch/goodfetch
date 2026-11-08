//! The `covers` lists that declare Features outside a runner (ADRs 0063,
//! 0069 and 0072): each active Journey's, in `journeys/J-*.toml`, and each
//! conformance case's, in every `cases.toml` under `conformance/`, the
//! Client conformance cases of `conformance/clients/` included. A retired
//! Journey is never run, so it declares nothing.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use goodfetch_protocol::journey::{Journey, Status};
use serde::Deserialize;

use super::scan::{Declaration, Level, Source};
use crate::tree;

/// The folder of the Journey catalog.
const JOURNEYS: &str = "journeys";
/// The folder of the conformance suites.
const CONFORMANCE: &str = "conformance";
/// The name of a suite's case list.
const CASE_LIST: &str = "cases.toml";

/// Every declaration of the workspace's Journeys and conformance cases.
pub(crate) fn read(root: &Path) -> Result<Vec<Declaration>> {
    let mut declarations = journeys(root)?;
    declarations.extend(conformance(root)?);
    Ok(declarations)
}

fn journeys(root: &Path) -> Result<Vec<Declaration>> {
    let mut declarations = Vec::new();
    let folder = root.join(JOURNEYS);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Ok(declarations);
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| {
            name.starts_with("J-") && Path::new(name).extension() == Some(OsStr::new("toml"))
        })
        .collect();
    names.sort();
    for name in names {
        let path = format!("{JOURNEYS}/{name}");
        let text =
            std::fs::read_to_string(root.join(&path)).with_context(|| format!("reading {path}"))?;
        let journey: Journey = toml::from_str(&text)
            .with_context(|| format!("parsing {path}; `xtask journeys check` says why"))?;
        if journey.status == Status::Retired {
            continue;
        }
        declarations.extend(journey.covers.iter().map(|feature| Declaration {
            feature: feature.as_str().to_owned(),
            level: Level::Journey,
            source: Source::Journey,
            test: journey.id.to_string(),
            package: None,
            path: path.clone(),
        }));
    }
    Ok(declarations)
}

/// A suite's case list, as far as it declares Features; the harness reads
/// the rest (`goodfetch-conformance`).
#[derive(Deserialize)]
struct CaseList {
    suite: Suite,
    #[serde(default)]
    capability: Vec<Capability>,
}

#[derive(Deserialize)]
struct Suite {
    name: String,
}

#[derive(Deserialize)]
struct Capability {
    #[serde(default)]
    case: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    #[serde(default)]
    covers: Vec<String>,
}

fn conformance(root: &Path) -> Result<Vec<Declaration>> {
    let mut lists = Vec::new();
    case_lists(&root.join(CONFORMANCE), &mut lists)?;
    lists.sort();
    let mut declarations = Vec::new();
    for file in lists {
        let path = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(&file).with_context(|| format!("reading {path}"))?;
        let list: CaseList = toml::from_str(&text).with_context(|| format!("parsing {path}"))?;
        for case in list
            .capability
            .iter()
            .flat_map(|capability| &capability.case)
        {
            declarations.extend(case.covers.iter().map(|feature| Declaration {
                feature: feature.clone(),
                level: Level::Conformance,
                source: Source::Conformance,
                test: format!("{} {}", list.suite.name, case.id),
                package: None,
                path: path.clone(),
            }));
        }
    }
    Ok(declarations)
}

/// Every case list below `folder`, outside installed packages and builds.
fn case_lists(folder: &Path, lists: &mut Vec<PathBuf>) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry.with_context(|| format!("listing {}", folder.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        if entry.file_type()?.is_dir() {
            if !tree::generated(&name.to_string_lossy()) {
                case_lists(&path, lists)?;
            }
        } else if name == CASE_LIST {
            lists.push(path);
        }
    }
    Ok(())
}
