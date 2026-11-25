//! The probe tables (ADR 0067): per OS and network level, one case per rule
//! of the seccomp list, per entry of the deny list and per file rule of the
//! policy, naming the call or the path, the outcome without confinement (the
//! positive control) and the exact outcome under it. A runner makes each
//! case's call unconfined first, then confined, and asserts both and the
//! mechanism in use.
//!
//! The tables are TOML files in `probes/`, in the form `xtask probes
//! generate` (#429) writes from the rules; [`check`] fails on a rule
//! without a case or a case without a rule, in Quick on every OS.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use goodfetch_protocol::plugin::NetworkLevel;
use serde::{Deserialize, Serialize};

use crate::deny_list::{Denied, Host, Places};
use crate::policy::FILE_RULES;
use crate::seccomp::{Call, Rule, Verdict};

/// The probe table of `level` on `host`, as committed; none where that
/// level's rules are not built.
pub fn committed(host: Host, level: NetworkLevel) -> Option<&'static str> {
    match (host, level) {
        (Host::Linux, NetworkLevel::None) => Some(include_str!("../probes/linux/none.toml")),
        _ => None,
    }
}

/// One OS's and network level's cases.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeTable {
    /// One case per seccomp rule, or several for a rule over many argument
    /// values (one per socket family).
    pub call: Vec<CallCase>,
    /// One case per deny-list entry and per file rule.
    pub path: Vec<PathCase>,
}

impl ProbeTable {
    pub fn read(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }
}

/// A system call made with fixed arguments, chosen so that it fails or
/// succeeds harmlessly unconfined with an outcome other than the refusal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallCase {
    pub rule: String,
    pub call: Call,
    pub args: Vec<Arg>,
    pub unconfined: Outcome,
    pub confined: Outcome,
}

/// One argument of a call: a number, or a process the runner knows only
/// when it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Arg {
    Number(i64),
    Process(ProcessArg),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessArg {
    /// The probe's own process.
    #[serde(rename = "self")]
    Own,
    /// The runner, which is not confined.
    Parent,
}

/// A path opened for reading or created for writing. The outcome confined
/// depends on the mechanism: Landlock refuses a path it grants nothing on,
/// and the mount-namespace fallback has no such path at all.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathCase {
    pub rule: String,
    pub access: Access,
    pub path: Placed,
    pub unconfined: Outcome,
    pub landlock: Outcome,
    pub fallback: Outcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Read,
    Write,
}

/// A folder a probe's path starts from, which the runner creates for the
/// run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placeholder {
    Home,
    GoodfetchHome,
    Bundle,
    Runtime,
    Instance,
}

const PLACEHOLDERS: &[(Placeholder, &str)] = &[
    (Placeholder::Home, "home"),
    (Placeholder::GoodfetchHome, "goodfetch_home"),
    (Placeholder::Bundle, "bundle"),
    (Placeholder::Runtime, "runtime"),
    (Placeholder::Instance, "instance"),
];

/// A probe's path: under a placeholder (`{home}/.ssh/id_ed25519`) or
/// absolute.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Placed {
    pub base: Option<Placeholder>,
    /// Relative to `base`, or absolute without one.
    pub rest: PathBuf,
}

impl TryFrom<String> for Placed {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let Some(after) = text.strip_prefix('{') else {
            return if Path::new(&text).is_absolute() {
                Ok(Self {
                    base: None,
                    rest: PathBuf::from(text),
                })
            } else {
                Err(format!(
                    "`{text}` starts with neither a placeholder nor `/`"
                ))
            };
        };
        let (name, rest) = after
            .split_once('}')
            .ok_or_else(|| format!("`{text}` does not close its placeholder"))?;
        let base = PLACEHOLDERS
            .iter()
            .find(|(_, known)| *known == name)
            .map(|(placeholder, _)| *placeholder)
            .ok_or_else(|| format!("`{name}` is not a placeholder"))?;
        Ok(Self {
            base: Some(base),
            rest: PathBuf::from(rest.trim_start_matches('/')),
        })
    }
}

impl From<Placed> for String {
    fn from(placed: Placed) -> Self {
        let rest = placed.rest.to_string_lossy().into_owned();
        match placed.base {
            None => rest,
            Some(base) => {
                let name = PLACEHOLDERS
                    .iter()
                    .find(|(placeholder, _)| *placeholder == base)
                    .map_or("", |(_, name)| *name);
                if rest.is_empty() {
                    format!("{{{name}}}")
                } else {
                    format!("{{{name}}}/{rest}")
                }
            }
        }
    }
}

/// Where a runner puts each placeholder's folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub places: Places,
    pub bundle: PathBuf,
    pub runtime: PathBuf,
    pub instance: PathBuf,
}

impl Layout {
    /// The folders under `root`, laid out as the engine lays out a Plugin's
    /// bundle, runtime and instance inside the Goodfetch home.
    pub fn under(root: &Path) -> Self {
        let home = root.join("home");
        let goodfetch_home = root.join("goodfetch");
        Self {
            bundle: goodfetch_home.join("plugins/probe/1.0.0"),
            runtime: goodfetch_home.join("runtimes/probe"),
            instance: goodfetch_home.join("instances/probe-1"),
            places: Places {
                home,
                goodfetch_home,
                app_data: None,
                local_app_data: None,
            },
        }
    }

    pub fn resolve(&self, placed: &Placed) -> PathBuf {
        let base = match placed.base {
            None => return placed.rest.clone(),
            Some(Placeholder::Home) => &self.places.home,
            Some(Placeholder::GoodfetchHome) => &self.places.goodfetch_home,
            Some(Placeholder::Bundle) => &self.bundle,
            Some(Placeholder::Runtime) => &self.runtime,
            Some(Placeholder::Instance) => &self.instance,
        };
        base.join(&placed.rest)
    }
}

/// What a probe's call gives back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Outcome {
    /// It returned without an error.
    Ok,
    /// It failed with this error.
    Errno(Errno),
    /// It ended the process with this status (`exit`, `exit_group`).
    Exit(u8),
}

impl TryFrom<String> for Outcome {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        if text == "ok" {
            return Ok(Self::Ok);
        }
        if let Some(status) = text.strip_prefix("exit:") {
            return status
                .parse()
                .map(Self::Exit)
                .map_err(|_| format!("`{text}` names no exit status"));
        }
        ERRNOS
            .iter()
            .find(|(_, name)| *name == text)
            .map(|(errno, _)| Self::Errno(*errno))
            .ok_or_else(|| format!("`{text}` is neither `ok`, `exit:<status>` nor a known error"))
    }
}

impl From<Outcome> for String {
    fn from(outcome: Outcome) -> Self {
        outcome.to_string()
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok => formatter.write_str("ok"),
            Self::Exit(status) => write!(formatter, "exit:{status}"),
            Self::Errno(errno) => formatter.write_str(errno.name()),
        }
    }
}

/// The errors a probe expects, by their Linux names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Errno {
    Eperm,
    Enoent,
    Esrch,
    Ebadf,
    Echild,
    Eacces,
    Efault,
    Einval,
    Erofs,
    Erange,
    Enosys,
    Eopnotsupp,
}

const ERRNOS: &[(Errno, &str)] = &[
    (Errno::Eperm, "EPERM"),
    (Errno::Enoent, "ENOENT"),
    (Errno::Esrch, "ESRCH"),
    (Errno::Ebadf, "EBADF"),
    (Errno::Echild, "ECHILD"),
    (Errno::Eacces, "EACCES"),
    (Errno::Efault, "EFAULT"),
    (Errno::Einval, "EINVAL"),
    (Errno::Erofs, "EROFS"),
    (Errno::Erange, "ERANGE"),
    (Errno::Enosys, "ENOSYS"),
    (Errno::Eopnotsupp, "EOPNOTSUPP"),
];

impl Errno {
    pub fn name(self) -> &'static str {
        ERRNOS
            .iter()
            .find(|(errno, _)| *errno == self)
            .map_or("", |(_, name)| *name)
    }
}

/// Why a probe table does not match the rules it probes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mismatch {
    /// A rule no case probes.
    RuleWithoutProbe(String),
    /// A case whose rule does not exist.
    ProbeWithoutRule(String),
    /// A case that makes another call than its rule is about.
    OtherCall { rule: String, call: Call },
    /// A case whose outcome confined is not what its rule gives.
    WrongOutcome {
        rule: String,
        expected: Outcome,
        found: Outcome,
    },
    /// A refusal whose positive control gives the refusal's own outcome, so
    /// the probe could pass with the call never working.
    NoPositiveControl { rule: String },
    /// A deny-list case whose path lies outside its entry.
    OutsideEntry { rule: String, path: PathBuf },
}

impl fmt::Display for Mismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuleWithoutProbe(rule) => write!(formatter, "rule `{rule}` has no probe"),
            Self::ProbeWithoutRule(rule) => {
                write!(formatter, "a probe names `{rule}`, which is no rule")
            }
            Self::OtherCall { rule, call } => {
                write!(
                    formatter,
                    "the probe of `{rule}` makes `{call:?}`, which the rule is not about"
                )
            }
            Self::WrongOutcome {
                rule,
                expected,
                found,
            } => write!(
                formatter,
                "the probe of `{rule}` expects `{found}` where the rule gives `{expected}`"
            ),
            Self::NoPositiveControl { rule } => {
                write!(
                    formatter,
                    "the probe of `{rule}` gives its refusal unconfined too"
                )
            }
            Self::OutsideEntry { rule, path } => {
                write!(
                    formatter,
                    "the probe of `{rule}` opens `{}`, outside its entry",
                    path.display()
                )
            }
        }
    }
}

/// Every way `table` fails to probe exactly `rules`, the deny list's
/// `denied` entries and the policy's file rules, one case at least per
/// rule; empty when it matches.
pub fn check(table: &ProbeTable, rules: &[Rule], denied: &[&Denied]) -> Vec<Mismatch> {
    let mut mismatches = Vec::new();
    let layout = Layout::under(Path::new("/probe"));
    for case in &table.call {
        let Some(rule) = rules.iter().find(|rule| rule.name == case.rule) else {
            mismatches.push(Mismatch::ProbeWithoutRule(case.rule.clone()));
            continue;
        };
        if rule.call != case.call {
            mismatches.push(Mismatch::OtherCall {
                rule: case.rule.clone(),
                call: case.call,
            });
        }
        let expected = match rule.verdict {
            Verdict::Allow => case.unconfined,
            Verdict::Refuse => Outcome::Errno(Errno::Eperm),
            Verdict::NotImplemented => Outcome::Errno(Errno::Enosys),
        };
        if case.confined != expected {
            mismatches.push(Mismatch::WrongOutcome {
                rule: case.rule.clone(),
                expected,
                found: case.confined,
            });
        }
        if rule.verdict != Verdict::Allow && case.unconfined == expected {
            mismatches.push(Mismatch::NoPositiveControl {
                rule: case.rule.clone(),
            });
        }
    }
    for case in &table.path {
        if let Some(entry) = denied.iter().find(|entry| entry.rule == case.rule) {
            let path = layout.resolve(&case.path);
            if !entry.covers(&path, &layout.places) {
                mismatches.push(Mismatch::OutsideEntry {
                    rule: case.rule.clone(),
                    path,
                });
            }
            let refused = [
                (Outcome::Ok, case.unconfined),
                (Outcome::Errno(Errno::Eacces), case.landlock),
                (Outcome::Errno(Errno::Enoent), case.fallback),
            ];
            for (expected, found) in refused {
                if expected != found {
                    mismatches.push(Mismatch::WrongOutcome {
                        rule: case.rule.clone(),
                        expected,
                        found,
                    });
                }
            }
        } else if !FILE_RULES.contains(&case.rule.as_str()) {
            mismatches.push(Mismatch::ProbeWithoutRule(case.rule.clone()));
        }
    }
    let probed: BTreeSet<&str> = table
        .call
        .iter()
        .map(|case| case.rule.as_str())
        .chain(table.path.iter().map(|case| case.rule.as_str()))
        .collect();
    let named = rules
        .iter()
        .map(|rule| rule.name)
        .chain(denied.iter().map(|entry| entry.rule))
        .chain(FILE_RULES.iter().copied());
    for name in named {
        if !probed.contains(name) {
            mismatches.push(Mismatch::RuleWithoutProbe(name.to_owned()));
        }
    }
    mismatches
}

#[cfg(test)]
mod tests;
