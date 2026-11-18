//! The runtime matrix check (ADR 0069): `ci/runtimes.toml` holds every
//! runtime the SDKs' tests run on and each suite's set of them, and this
//! check holds it to the ADR's placement.
//!
//! - Every runtime the ADR lists is in it: the oldest Node line
//!   `package.json`'s `engines` accepts, a Bun line, Deno 2, workerd,
//!   Vercel's Edge Runtime, each CPython line from the one `pyproject.toml`'s
//!   `requires-python` accepts to the newest, and the newest line's
//!   free-threaded build. Each is pinned to a version of its own line; the
//!   edge runtimes take no pin, since no runner installs them yet.
//! - Each set holds exactly the runtimes its suite runs: Pre-merge the newest
//!   Node LTS and the newest CPython; linux-x64-gnu every runtime; the other
//!   Linux cells the oldest and newest Node and CPython lines, Bun, Deno and
//!   the free-threaded builds; Windows and macOS the same without the
//!   free-threaded builds; the nightly-only job the free-threaded builds.
//! - Each runtime and each set sits on one line, which `ci/runtimes.sh`
//!   reads with sed.
//! - Each workflow step that runs `.github/actions/host-tools` names a set,
//!   and the action installs each Node, Bun and Deno runtime from its pin.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;
use crate::workflow;

const KIND: &str = "runtime-matrix";
const FILE: &str = "ci/runtimes.toml";
const ACTION: &str = ".github/actions/host-tools/action.yml";

/// A suite's set of runtimes, the value a job passes host-tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Set {
    PreMerge,
    ReleaseLinuxX64Gnu,
    ReleaseLinux,
    ReleaseOther,
    Nightly,
}

impl Set {
    const ALL: [Self; 5] = [
        Self::PreMerge,
        Self::ReleaseLinuxX64Gnu,
        Self::ReleaseLinux,
        Self::ReleaseOther,
        Self::Nightly,
    ];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::PreMerge => "pre-merge",
            Self::ReleaseLinuxX64Gnu => "release-linux-x64-gnu",
            Self::ReleaseLinux => "release-linux",
            Self::ReleaseOther => "release-other",
            Self::Nightly => "nightly",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|set| set.key() == key)
    }
}

/// What a runtime's ID names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Node(u32),
    Bun(u32),
    Deno(u32),
    Workerd,
    VercelEdge,
    CPython { minor: u32, free_threaded: bool },
}

impl Family {
    fn parse(id: &str) -> Option<Self> {
        match id {
            "workerd" => return Some(Self::Workerd),
            "vercel-edge" => return Some(Self::VercelEdge),
            _ => {}
        }
        if let Some(major) = id.strip_prefix("node-") {
            return number(major).map(Self::Node);
        }
        if let Some(major) = id.strip_prefix("bun-") {
            return number(major).map(Self::Bun);
        }
        if let Some(major) = id.strip_prefix("deno-") {
            return number(major).map(Self::Deno);
        }
        let minor = id.strip_prefix("cpython-3.")?;
        let (minor, free_threaded) = match minor.strip_suffix('t') {
            Some(minor) => (minor, true),
            None => (minor, false),
        };
        number(minor).map(|minor| Self::CPython {
            minor,
            free_threaded,
        })
    }

    /// The start every version of the line has, or `None` for a runtime
    /// that takes no pin.
    fn line(self) -> Option<String> {
        match self {
            Self::Node(major) | Self::Bun(major) | Self::Deno(major) => Some(format!("{major}.")),
            Self::CPython { minor, .. } => Some(format!("3.{minor}.")),
            Self::Workerd | Self::VercelEdge => None,
        }
    }

    /// Whether host-tools installs it from its pin; the CPython builds are
    /// installed by `ci/host/fetch.sh` from the set.
    fn installed_by_action(self) -> bool {
        matches!(self, Self::Node(_) | Self::Bun(_) | Self::Deno(_))
    }
}

/// A number written without leading zeros.
fn number(text: &str) -> Option<u32> {
    let value: u32 = text.parse().ok()?;
    (value.to_string() == text).then_some(value)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pin {
    version: Option<String>,
    #[serde(default)]
    lts: bool,
}

struct Runtime {
    id: String,
    family: Family,
    lts: bool,
}

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let text =
        std::fs::read_to_string(root.join(FILE)).with_context(|| format!("reading {FILE}"))?;
    let file: toml::Table = toml::from_str(&text).with_context(|| format!("parsing {FILE}"))?;
    let node_floor = node_floor(root)?;
    let cpython_floor = cpython_floor(root)?;

    let mut findings = Vec::new();
    let mut runtimes = Vec::new();
    let empty = toml::Table::new();
    let declared = if let Some(toml::Value::Table(table)) = file.get("runtimes") {
        table
    } else {
        findings.push(finding("runtimes", "has no [runtimes] table"));
        &empty
    };
    for (id, value) in declared {
        let Some(family) = Family::parse(id) else {
            findings.push(finding(
                id,
                "names no runtime: the IDs are node-<major>, bun-<major>, deno-<major>, workerd, vercel-edge and cpython-3.<minor>, with t for a free-threaded build",
            ));
            continue;
        };
        let mut problems = Vec::new();
        if !on_one_line(&text, &[id.clone(), format!("\"{id}\"")], "{", "}") {
            problems.push("does not sit on one line, `<id> = { … }`".to_owned());
        }
        let mut lts = false;
        match value.clone().try_into::<Pin>() {
            Err(error) => problems.push(format!("is no pin: {}", error.message())),
            Ok(pin) => {
                lts = pin.lts;
                problems.extend(pin_problems(family, &pin));
            }
        }
        match family {
            Family::Node(major) if major < node_floor => problems.push(format!(
                "is below the oldest line package.json's engines accepts, Node {node_floor}"
            )),
            Family::CPython { minor, .. } if minor < cpython_floor => problems.push(format!(
                "is below the oldest line pyproject.toml's requires-python accepts, CPython 3.{cpython_floor}"
            )),
            _ => {}
        }
        if !problems.is_empty() {
            findings.push(finding(id, &problems.join("; ")));
        }
        runtimes.push(Runtime {
            id: id.clone(),
            family,
            lts,
        });
    }

    findings.extend(missing(&runtimes, node_floor, cpython_floor));
    findings.extend(sets(&text, &file, &runtimes));
    findings.extend(jobs(root)?);
    findings.extend(action(root, &runtimes)?);
    Ok(findings)
}

fn finding(id: &str, message: &str) -> Finding {
    Finding::new(KIND, format!("{FILE}#{id}"), message)
}

fn pin_problems(family: Family, pin: &Pin) -> Vec<String> {
    let mut problems = Vec::new();
    match (family.line(), &pin.version) {
        (Some(line), Some(version)) if !version.starts_with(&line) => {
            problems.push(format!("is pinned to {version}, outside its line {line}x"));
        }
        (Some(_), None) => problems.push("has no version".to_owned()),
        (None, Some(_)) => problems.push("takes no version: no runner installs it".to_owned()),
        _ => {}
    }
    if pin.lts && !matches!(family, Family::Node(_)) {
        problems.push("marks lts, which only a Node line carries".to_owned());
    }
    problems
}

/// The runtimes ADR 0069 lists that the matrix lacks.
fn missing(runtimes: &[Runtime], node_floor: u32, cpython_floor: u32) -> Vec<Finding> {
    let has =
        |wanted: &dyn Fn(Family) -> bool| runtimes.iter().any(|runtime| wanted(runtime.family));
    let mut findings = Vec::new();
    let mut require = |id: &str, present: bool, why: &str| {
        if !present {
            findings.push(finding(id, &format!("is missing: {why} (ADR 0069)")));
        }
    };
    require(
        &format!("node-{node_floor}"),
        has(&|family| family == Family::Node(node_floor)),
        "package.json's engines accept it, so the oldest-line cells run it",
    );
    require(
        "bun",
        has(&|family| matches!(family, Family::Bun(_))),
        "the Release suite runs the smoke scripts on Bun",
    );
    require(
        "deno-2",
        has(&|family| family == Family::Deno(2)),
        "the Release suite runs the smoke scripts on Deno 2",
    );
    require(
        "workerd",
        has(&|family| family == Family::Workerd),
        "the Release suite runs the smoke scripts on workerd against a Daemon",
    );
    require(
        "vercel-edge",
        has(&|family| family == Family::VercelEdge),
        "the Release suite runs the smoke scripts on Vercel's Edge Runtime against a Daemon",
    );
    let newest = runtimes
        .iter()
        .filter_map(|runtime| match runtime.family {
            Family::CPython { minor, .. } => Some(minor),
            _ => None,
        })
        .max()
        .unwrap_or(cpython_floor)
        .max(cpython_floor);
    for minor in cpython_floor..=newest {
        require(
            &format!("cpython-3.{minor}"),
            has(&|family| {
                family
                    == Family::CPython {
                        minor,
                        free_threaded: false,
                    }
            }),
            "linux-x64-gnu runs every CPython line pyproject.toml's requires-python accepts",
        );
    }
    require(
        &format!("cpython-3.{newest}t"),
        has(&|family| {
            family
                == Family::CPython {
                    minor: newest,
                    free_threaded: true,
                }
        }),
        "the Linux cells run the newest line's free-threaded build",
    );
    findings
}

/// The runtimes each set holds by ADR 0069, from the runtimes the matrix
/// declares.
fn expected(set: Set, runtimes: &[Runtime]) -> BTreeSet<&str> {
    let node = |pick: fn(u32, u32) -> u32, lts_only: bool| {
        runtimes
            .iter()
            .filter_map(|runtime| match runtime.family {
                Family::Node(major) if runtime.lts || !lts_only => Some(major),
                _ => None,
            })
            .reduce(pick)
    };
    let gil = |pick: fn(u32, u32) -> u32| {
        runtimes
            .iter()
            .filter_map(|runtime| match runtime.family {
                Family::CPython {
                    minor,
                    free_threaded: false,
                } => Some(minor),
                _ => None,
            })
            .reduce(pick)
    };
    let newest_lts = node(u32::max, true);
    let (oldest_node, newest_node) = (node(u32::min, false), node(u32::max, false));
    let (oldest_cpython, newest_cpython) = (gil(u32::min), gil(u32::max));
    let ends = |runtime: &Runtime| match runtime.family {
        Family::Node(major) => Some(major) == oldest_node || Some(major) == newest_node,
        Family::Bun(_) | Family::Deno(_) => true,
        Family::CPython {
            minor,
            free_threaded: false,
        } => Some(minor) == oldest_cpython || Some(minor) == newest_cpython,
        Family::CPython {
            free_threaded: true,
            ..
        }
        | Family::Workerd
        | Family::VercelEdge => false,
    };
    let free_threaded = |runtime: &Runtime| {
        matches!(
            runtime.family,
            Family::CPython {
                free_threaded: true,
                ..
            }
        )
    };
    runtimes
        .iter()
        .filter(|runtime| match set {
            Set::PreMerge => match runtime.family {
                Family::Node(major) => Some(major) == newest_lts,
                Family::CPython {
                    minor,
                    free_threaded: false,
                } => Some(minor) == newest_cpython,
                _ => false,
            },
            Set::ReleaseLinuxX64Gnu => true,
            Set::ReleaseLinux => ends(runtime) || free_threaded(runtime),
            Set::ReleaseOther => ends(runtime),
            Set::Nightly => free_threaded(runtime),
        })
        .map(|runtime| runtime.id.as_str())
        .collect()
}

fn sets(text: &str, file: &toml::Table, runtimes: &[Runtime]) -> Vec<Finding> {
    let mut findings = Vec::new();
    let empty = toml::Table::new();
    let declared = match file.get("sets") {
        Some(toml::Value::Table(table)) => table,
        _ => &empty,
    };
    for name in declared.keys() {
        if Set::from_key(name).is_none() {
            findings.push(finding(
                name,
                "is no suite's set: the sets are pre-merge, release-linux-x64-gnu, release-linux, release-other and nightly",
            ));
        }
    }
    let known: BTreeSet<&str> = runtimes.iter().map(|runtime| runtime.id.as_str()).collect();
    for set in Set::ALL {
        let key = set.key();
        let Some(value) = declared.get(key) else {
            findings.push(finding(key, "is missing from [sets] (ADR 0069)"));
            continue;
        };
        let mut problems = Vec::new();
        if !on_one_line(text, &[key.to_owned()], "[", "]") {
            problems.push("does not sit on one line, `<set> = [ … ]`".to_owned());
        }
        let ids: Vec<&str> = match value.as_array() {
            Some(items) => items.iter().filter_map(toml::Value::as_str).collect(),
            None => Vec::new(),
        };
        if value
            .as_array()
            .is_none_or(|items| items.len() != ids.len())
        {
            problems.push("is no list of runtime IDs".to_owned());
        }
        let unknown: Vec<&str> = ids
            .iter()
            .copied()
            .filter(|id| !known.contains(id))
            .collect();
        if !unknown.is_empty() {
            problems.push(format!(
                "names runtimes [runtimes] lacks: {}",
                unknown.join(", ")
            ));
        }
        let held: BTreeSet<&str> = ids
            .iter()
            .copied()
            .filter(|id| known.contains(id))
            .collect();
        let wanted = expected(set, runtimes);
        let lacking: Vec<&str> = wanted.difference(&held).copied().collect();
        if !lacking.is_empty() {
            problems.push(format!("lacks {}", lacking.join(", ")));
        }
        let extra: Vec<&str> = held.difference(&wanted).copied().collect();
        if !extra.is_empty() {
            problems.push(format!(
                "holds {}, which its suite does not run",
                extra.join(", ")
            ));
        }
        if ids.len() != ids.iter().collect::<BTreeSet<_>>().len() {
            problems.push("names a runtime twice".to_owned());
        }
        if !problems.is_empty() {
            findings.push(finding(key, &format!("{} (ADR 0069)", problems.join("; "))));
        }
    }
    findings
}

/// Whether a line of `text` starts with one of `keys` and ` = ` and holds the
/// whole value from `open` to `close`.
fn on_one_line(text: &str, keys: &[String], open: &str, close: &str) -> bool {
    text.lines().any(|line| {
        keys.iter().any(|key| {
            line.strip_prefix(key.as_str())
                .and_then(|rest| rest.strip_prefix(" = "))
                .is_some_and(|value| value.starts_with(open) && value.trim_end().ends_with(close))
        })
    })
}

/// The workflow steps that run host-tools with a value that is no set.
fn jobs(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for workflow in workflow::read_all(root)? {
        for job in &workflow.jobs {
            let named: Vec<&str> = job
                .steps
                .iter()
                .filter(|step| {
                    step.uses.as_deref().is_some_and(|uses| {
                        uses.trim_end_matches('/')
                            .ends_with(".github/actions/host-tools")
                    })
                })
                .filter_map(|step| step.with.get("runtimes").map(String::as_str))
                .filter(|value| !value.starts_with("${{") && Set::from_key(value).is_none())
                .collect();
            if !named.is_empty() {
                findings.push(Finding::new(
                    KIND,
                    format!("{}#{}", workflow.file, job.id),
                    format!(
                        "passes host-tools runtimes `{}`, which is no set of {FILE}",
                        named.join("`, `")
                    ),
                ));
            }
        }
    }
    Ok(findings)
}

/// The Node, Bun and Deno runtimes host-tools never installs from their pin.
fn action(root: &Path, runtimes: &[Runtime]) -> Result<Vec<Finding>> {
    let text =
        std::fs::read_to_string(root.join(ACTION)).with_context(|| format!("reading {ACTION}"))?;
    Ok(runtimes
        .iter()
        .filter(|runtime| runtime.family.installed_by_action())
        .filter(|runtime| !names_output(&text, &runtime.id))
        .map(|runtime| {
            Finding::new(
                KIND,
                format!("{ACTION}#{}", runtime.id),
                format!(
                    "installs no {id}: no step reads steps.runtimes.outputs.{id}, its pin",
                    id = runtime.id
                ),
            )
        })
        .collect())
}

/// Whether `text` reads the step output `steps.runtimes.outputs.<id>`, and
/// not one whose name only starts with `id`.
fn names_output(text: &str, id: &str) -> bool {
    let needle = format!("steps.runtimes.outputs.{id}");
    text.match_indices(&needle).any(|(at, _)| {
        text[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|next| !(next.is_ascii_alphanumeric() || next == '-' || next == '_'))
    })
}

/// The oldest Node line `package.json`'s `engines.node` accepts, `>=<major>…`.
fn node_floor(root: &Path) -> Result<u32> {
    let text =
        std::fs::read_to_string(root.join("package.json")).context("reading package.json")?;
    let manifest: serde_json::Value =
        serde_json::from_str(&text).context("parsing package.json")?;
    let range = manifest["engines"]["node"]
        .as_str()
        .context("package.json has no engines.node")?;
    range
        .strip_prefix(">=")
        .and_then(|version| number(version.split('.').next().unwrap_or_default()))
        .with_context(|| format!("package.json's engines.node `{range}` is no `>=<major>…`"))
}

/// The oldest CPython line `pyproject.toml`'s `requires-python` accepts,
/// `>=3.<minor>…`.
fn cpython_floor(root: &Path) -> Result<u32> {
    let text =
        std::fs::read_to_string(root.join("pyproject.toml")).context("reading pyproject.toml")?;
    let manifest: toml::Table = toml::from_str(&text).context("parsing pyproject.toml")?;
    let range = manifest
        .get("project")
        .and_then(|project| project.get("requires-python"))
        .and_then(toml::Value::as_str)
        .context("pyproject.toml has no project.requires-python")?;
    range
        .strip_prefix(">=3.")
        .and_then(|version| number(version.split('.').next().unwrap_or_default()))
        .with_context(|| format!("pyproject.toml's requires-python `{range}` is no `>=3.<minor>…`"))
}
