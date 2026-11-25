//! The policy an instance of a process Plugin runs under (ADR 0033),
//! generated from its approved manifest and the instance's folders, the same
//! on every OS: what each OS's applier turns into its own mechanisms.
//!
//! There is no way to build a policy that confines less: no constructor
//! takes a path, a network level or a limit from anywhere but the manifest
//! and the instance, and the deny list holds whatever Operator policy
//! approved (ADRs 0016, 0033).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use goodfetch_protocol::plugin::{Limits, NetworkLevel, PluginManifest};
use serde::{Deserialize, Serialize};

use crate::deny_list::{self, Base, Denied, Host, Places};

/// The rules a policy's paths give on every OS, each with its probes: an
/// instance reads its bundle and runtime, writes its own folders, and reads
/// or writes nothing else, its bundle included.
pub const FILE_RULES: &[&str] = &[
    "files.read_bundle",
    "files.read_runtime",
    "files.write_instance",
    "files.read_elsewhere",
    "files.write_elsewhere",
    "files.write_bundle",
];

/// One instance's own folders and values, which the engine sets up before
/// it starts the instance.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
    /// The Plugin's unpacked bundle in the Plugin store.
    pub bundle: PathBuf,
    /// Its runtime in the runtime cache, none for a Plugin without one.
    pub runtime: Option<PathBuf>,
    /// The folders that belong to this instance alone, its temporary folder
    /// among them, deleted when it stops.
    pub folders: Vec<PathBuf>,
    /// The values of the variables the manifest declares, each from the
    /// Plugin's setting that values it.
    pub env: BTreeMap<String, String>,
}

/// What one instance may do, which its OS's applier enforces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub network_level: NetworkLevel,
    /// The paths it may read: its bundle, its runtime and the files its
    /// manifest declares.
    pub reads: Vec<PathBuf>,
    /// The paths it may write: its instance's folders.
    pub writes: Vec<PathBuf>,
    /// The executables it may start: its bundle's entrypoint, its runtime
    /// and the processes its manifest declares.
    pub executables: Vec<PathBuf>,
    /// Its whole environment; nothing is inherited.
    pub env: BTreeMap<String, String>,
    pub limits: Limits,
}

/// Why no policy can be built for an instance.
#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    /// A declared path is on the fixed deny list, or a declared folder holds
    /// a path on it.
    #[error("`{path}` is on the deny list ({rule}), so no Plugin can read it")]
    Denied { rule: &'static str, path: PathBuf },
    /// A declared path is relative, so it would name another path in each
    /// folder the instance starts from.
    #[error("`{path}` is not an absolute path")]
    Relative { path: PathBuf },
    /// A declared path cannot be read here.
    #[error("cannot read `{path}`: {source}")]
    Unreadable { path: PathBuf, source: io::Error },
}

impl Policy {
    /// The policy of an instance of the Plugin `manifest` describes, refusing
    /// a declared file or process the deny list covers, wherever it points
    /// and whatever a declared folder holds. It reads the file system, so it
    /// blocks: call it from blocking work, never on an async worker.
    pub fn of(
        manifest: &PluginManifest,
        instance: &Instance,
        places: &Places,
    ) -> Result<Self, PolicyError> {
        let denied: Vec<&Denied> = match Host::current() {
            Some(host) => deny_list::on(host).collect(),
            None => deny_list::DENY_LIST.iter().collect(),
        };
        let mut reads = vec![instance.bundle.clone()];
        reads.extend(instance.runtime.clone());
        let mut executables = vec![instance.bundle.join(&manifest.entrypoint)];
        executables.extend(instance.runtime.clone());
        for declared in &manifest.files {
            reads.push(checked(Path::new(declared), &denied, places)?);
        }
        for declared in &manifest.processes {
            executables.push(checked(Path::new(declared), &denied, places)?);
        }
        let env = instance
            .env
            .iter()
            .filter(|(name, _)| manifest.env.contains_key(*name))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        Ok(Self {
            network_level: manifest.network_level,
            reads,
            writes: instance.folders.clone(),
            executables,
            env,
            limits: manifest.limits,
        })
    }
}

/// `declared` as the policy grants it, refused when an entry covers it as
/// written or once its links are followed, or covers a path a declared
/// folder holds.
fn checked(declared: &Path, denied: &[&Denied], places: &Places) -> Result<PathBuf, PolicyError> {
    if !declared.is_absolute() {
        return Err(PolicyError::Relative {
            path: declared.to_path_buf(),
        });
    }
    let unreadable = |source| PolicyError::Unreadable {
        path: declared.to_path_buf(),
        source,
    };
    let real = fs::canonicalize(declared).map_err(unreadable)?;
    for path in [declared, real.as_path()] {
        refuse_covered(path, denied, places)?;
    }
    if real.is_dir() {
        walk(&real, denied)?;
    }
    Ok(real)
}

fn refuse_covered(path: &Path, denied: &[&Denied], places: &Places) -> Result<(), PolicyError> {
    let covering = denied.iter().find(|entry| {
        entry.covers(path, places)
            || entry
                .resolve(places)
                .and_then(|resolved| fs::canonicalize(resolved).ok())
                .is_some_and(|real| path.starts_with(&real) || real.starts_with(path))
    });
    match covering {
        Some(entry) => Err(PolicyError::Denied {
            rule: entry.rule,
            path: path.to_path_buf(),
        }),
        None => Ok(()),
    }
}

/// Refuses a folder that holds a name the deny list covers anywhere (a
/// `.env` file, `.git/hooks`). Links are not followed: what one points to
/// is refused by the applier, which grants only the declared folder.
#[expect(
    clippy::disallowed_methods,
    reason = "a policy is built before an instance starts, in the engine's blocking work or in the sandbox helper, never on an async worker"
)]
fn walk(folder: &Path, denied: &[&Denied]) -> Result<(), PolicyError> {
    let anywhere: Vec<&Denied> = denied
        .iter()
        .copied()
        .filter(|entry| entry.base == Base::Anywhere)
        .collect();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(next) = pending.pop() {
        let unreadable = |source| PolicyError::Unreadable {
            path: next.clone(),
            source,
        };
        for child in fs::read_dir(&next).map_err(unreadable)? {
            let child = child.map_err(unreadable)?;
            let path = child.path();
            if let Some(entry) = anywhere.iter().find(|entry| entry.names_in(&path)) {
                return Err(PolicyError::Denied {
                    rule: entry.rule,
                    path,
                });
            }
            if child.file_type().map_err(unreadable)?.is_dir() {
                pending.push(path);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
