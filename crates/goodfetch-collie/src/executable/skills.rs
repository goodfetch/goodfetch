//! `skill install` (ADRs 0065 and 0095, story:11-237): writes the agent
//! skills the executable embeds, its Product's renderings of `skills/`,
//! into a host's skills folder, each in a folder of its name, and prints
//! each with its folder and digest.
//!
//! Each skill folder it writes holds [`MARKER`], the digest of the files
//! beside it as written, which is how a later run knows the folder is one
//! it installed and nobody changed since: only such a folder is replaced. A
//! folder of a skill's name without it, or whose files no longer have that
//! digest, is the person's, and the run is refused before anything is
//! written. A skill already as the executable carries it is left untouched,
//! so a second run writes nothing. A replaced skill is written beside its
//! folder first, then moved in its place.
//!
//! A skill's digest is SHA-256 over, for each of its files but [`MARKER`]
//! in the order of their `/`-separated paths from the skill's folder, the
//! path, a NUL, the SHA-256 of the file's bytes in lowercase hexadecimal
//! and a newline, written `sha256:<hex>`.
#![expect(
    clippy::disallowed_methods,
    reason = "the command writes the skills folder from the executable's own thread, before and outside any runtime"
)]

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Subcommand, ValueEnum};
use goodfetch_engine::CallError;
use goodfetch_engine::pinned_download::Digest;
use goodfetch_protocol::checkpoint::{FILE_WRITE_FAILED, FileWriteFailedDetails, WriteSide};
use goodfetch_protocol::skills::{
    InstalledSkill, SKILL_INSTALL_REFUSED, SkillHost, SkillScope, SkillsInstalled,
};

use super::output::{self, Human};

/// The file, in each skill folder `skill install` writes, that holds the
/// digest of the files it wrote there.
pub(crate) const MARKER: &str = ".goodfetch-skill";

/// One file of a skill an executable embeds.
#[derive(Clone, Copy, Debug)]
pub struct SkillFile {
    /// Its `/`-separated path from the skills folder, the skill's name
    /// first, such as `goodfetch-collie/SKILL.md`.
    pub path: &'static str,
    /// Its bytes.
    pub contents: &'static [u8],
}

/// The agent skills an executable embeds, which `skill install` writes.
#[derive(Clone, Copy, Debug)]
pub struct Skills {
    files: &'static [SkillFile],
}

impl Skills {
    /// Collie's executable's: its renderings of `skills/`.
    pub const COLLIE: Self = Self::new(&include!("../../generated/skills.rs"));

    /// The skills of these files.
    pub const fn new(files: &'static [SkillFile]) -> Self {
        Self { files }
    }

    /// Their files.
    #[must_use]
    pub const fn files(self) -> &'static [SkillFile] {
        self.files
    }

    /// Each skill's files, by its name, each by its path from the skill's
    /// folder.
    fn by_name(self) -> BTreeMap<&'static str, BTreeMap<&'static str, &'static [u8]>> {
        let mut skills: BTreeMap<_, BTreeMap<_, _>> = BTreeMap::new();
        for file in self.files {
            if let Some((name, path)) = file.path.split_once('/') {
                skills.entry(name).or_default().insert(path, file.contents);
            }
        }
        skills
    }
}

// The `skill` group's commands. No doc comment: clap would make it the
// group's help.
#[derive(Subcommand)]
pub(super) enum SkillCommand {
    /// Writes the agent skills this executable carries into a coding
    /// agent's skills folder, each in a folder of its name, replacing only
    /// one it installed itself that nobody changed since; a skill already
    /// there as it carries it is left as it is.
    Install {
        /// The coding agent whose skills folder it writes: `.claude/skills`
        /// for Claude Code, `.cursor/skills` for Cursor, and the Agent
        /// Skills folder, `.agents/skills`, for Codex and other agents.
        #[arg(long = "for", value_enum, default_value_t = SkillHostFlag::Agents)]
        host: SkillHostFlag,
        /// Writes the skills folder in the working folder, for the project
        /// the agent opens there, as it does unless `--user` is given.
        #[arg(long)]
        project: bool,
        /// Writes the skills folder in the user's home folder, for every
        /// project.
        #[arg(long, conflicts_with = "project")]
        user: bool,
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// The coding agents `skill install --for` names.
#[derive(Clone, Copy, ValueEnum)]
pub(super) enum SkillHostFlag {
    ClaudeCode,
    Codex,
    Cursor,
    Agents,
}

impl From<SkillHostFlag> for SkillHost {
    fn from(flag: SkillHostFlag) -> Self {
        match flag {
            SkillHostFlag::ClaudeCode => Self::ClaudeCode,
            SkillHostFlag::Codex => Self::Codex,
            SkillHostFlag::Cursor => Self::Cursor,
            SkillHostFlag::Agents => Self::Agents,
        }
    }
}

impl SkillCommand {
    /// Runs the command with the `skills` the executable carries.
    pub(super) fn run(
        self,
        skills: Skills,
        environment: &BTreeMap<OsString, OsString>,
        human: &Human,
    ) -> ExitCode {
        let Self::Install {
            host, user, json, ..
        } = self;
        let scope = if user {
            SkillScope::User
        } else {
            SkillScope::Project
        };
        install(skills, (host.into(), scope), environment, json, human)
    }
}

/// The skills folder of `host` in `base`, the folder of the install's
/// scope: `skills` in the host's own folder, joined one name at a time so
/// that the path takes the platform's separator throughout.
fn skills_folder(base: &Path, host: SkillHost) -> PathBuf {
    let host_folder = match host {
        SkillHost::ClaudeCode => ".claude",
        SkillHost::Codex | SkillHost::Agents => ".agents",
        SkillHost::Cursor => ".cursor",
    };
    base.join(host_folder).join("skills")
}

/// Installs `skills` for `host` at `scope`: in the working folder, or in
/// the user's home folder, `HOME` (`USERPROFILE` on Windows) of
/// `environment`. A skill folder it did not install is
/// `skill_install_refused`, and a file it cannot read or write
/// `file_write_failed`, each printed as every command's failure is.
fn install(
    skills: Skills,
    (host, scope): (SkillHost, SkillScope),
    environment: &BTreeMap<OsString, OsString>,
    json: bool,
    human: &Human,
) -> ExitCode {
    let base = match scope {
        SkillScope::Project => std::env::current_dir().ok(),
        SkillScope::User => ["HOME", "USERPROFILE"]
            .into_iter()
            .find_map(|name| environment.get(OsStr::new(name)))
            .filter(|folder| !folder.is_empty())
            .map(PathBuf::from),
    };
    let Some(base) = base else {
        tracing::error!(
            scope = ?scope,
            "no folder to install the skills in: the working folder cannot be read, or HOME is unset"
        );
        return ExitCode::FAILURE;
    };
    let folder = skills_folder(&base, host);
    let installed = match install_into(skills, &folder) {
        Ok(installed) => installed,
        Err(Refused::Taken(path)) => {
            let failure = CallError::new(
                &SKILL_INSTALL_REFUSED,
                format!(
                    "`{}` holds a skill that `skill install` did not install, or one changed since it did; move it away and run `skill install` again. No skill was written.",
                    path.display()
                ),
                &serde_json::Map::new(),
            );
            return output::fail(failure.to_error_object(), None, json, human);
        }
        Err(Refused::Io { path, error }) => {
            let failure = CallError::new(
                &FILE_WRITE_FAILED,
                format!(
                    "`{}` could not be read or written: {error}. The skills written before it stay.",
                    path.display()
                ),
                &FileWriteFailedDetails {
                    reason: "other".to_owned(),
                    os_code: None,
                    path: Some(path.display().to_string()),
                    side: WriteSide::Engine,
                },
            );
            return output::fail(failure.to_error_object(), None, json, human);
        }
    };
    if json {
        output::json(&SkillsInstalled {
            target: host,
            scope,
            installed: installed.into_iter().map(|skill| skill.skill).collect(),
        });
        return ExitCode::SUCCESS;
    }
    for skill in installed {
        let InstalledSkill { name, path, digest } = &skill.skill;
        output::line(&if skill.written {
            format!("Installed {name} in {path} ({digest}).")
        } else {
            format!("{name} is already installed in {path} ({digest}).")
        });
    }
    ExitCode::SUCCESS
}

/// One skill an install made sure of, and whether it wrote it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Installed {
    pub(crate) skill: InstalledSkill,
    pub(crate) written: bool,
}

/// Why an install wrote nothing more.
#[derive(Debug)]
pub(crate) enum Refused {
    /// This skill folder is not one it installed, or was changed since.
    Taken(PathBuf),
    /// This file or folder could not be read or written.
    Io { path: PathBuf, error: io::Error },
}

/// What an install does with one skill.
enum Step {
    Keep,
    Write,
}

/// Installs `skills` into the skills folder `folder`, by the rules of this
/// module's header.
pub(crate) fn install_into(skills: Skills, folder: &Path) -> Result<Vec<Installed>, Refused> {
    let mut planned = Vec::new();
    for (name, files) in skills.by_name() {
        let shipped = digest(files.iter().map(|(path, bytes)| (*path, *bytes)));
        let target = folder.join(name);
        let step = match on_disk(&target)? {
            None => Step::Write,
            Some((found, marked))
                if found == shipped && marked.as_deref() == Some(shipped.as_str()) =>
            {
                Step::Keep
            }
            Some((found, marked))
                if found == shipped || marked.as_deref() == Some(found.as_str()) =>
            {
                Step::Write
            }
            Some(_) => return Err(Refused::Taken(target)),
        };
        planned.push((name, files, shipped, target, step));
    }
    let mut installed = Vec::new();
    for (name, files, shipped, target, step) in planned {
        let written = matches!(step, Step::Write);
        if written {
            write_skill(folder, name, &files, &shipped, &target)?;
        }
        installed.push(Installed {
            skill: InstalledSkill {
                name: name.to_owned(),
                path: target.display().to_string(),
                digest: shipped,
            },
            written,
        });
    }
    Ok(installed)
}

/// The digest of the skill folder `target`'s files and the digest its
/// [`MARKER`] holds, if any; none when there is no such folder.
fn on_disk(target: &Path) -> Result<Option<(String, Option<String>)>, Refused> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |error| Refused::Io { path, error }
    };
    if !target.try_exists().map_err(io(target))? {
        return Ok(None);
    }
    let mut files = BTreeMap::new();
    let mut folders = vec![target.to_path_buf()];
    while let Some(next) = folders.pop() {
        for item in std::fs::read_dir(&next).map_err(io(&next))? {
            let path = item.map_err(io(&next))?.path();
            if path.is_dir() {
                folders.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(target)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(relative, std::fs::read(&path).map_err(io(&path))?);
        }
    }
    let marked = files
        .remove(MARKER)
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned());
    let found = digest(
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
    );
    Ok(Some((found, marked)))
}

/// Writes the skill `name`'s `files` and its [`MARKER`] beside `target`,
/// then moves them in its place.
fn write_skill(
    folder: &Path,
    name: &str,
    files: &BTreeMap<&str, &[u8]>,
    shipped: &str,
    target: &Path,
) -> Result<(), Refused> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |error| Refused::Io { path, error }
    };
    let staged = folder.join(format!(".{name}{MARKER}"));
    if staged.try_exists().map_err(io(&staged))? {
        std::fs::remove_dir_all(&staged).map_err(io(&staged))?;
    }
    for (path, bytes) in files.iter().chain([(&MARKER, &shipped.as_bytes())]) {
        let file = staged.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        std::fs::write(&file, bytes).map_err(io(&file))?;
    }
    if target.try_exists().map_err(io(target))? {
        std::fs::remove_dir_all(target).map_err(io(target))?;
    }
    std::fs::rename(&staged, target).map_err(io(target))
}

/// The digest of a skill's `files`, each by its path from its folder, by
/// the rule of this module's header.
fn digest<'a>(files: impl Iterator<Item = (&'a str, &'a [u8])>) -> String {
    let mut sorted: Vec<_> = files.collect();
    sorted.sort_by_key(|(path, _)| *path);
    let mut tree = String::new();
    for (path, bytes) in sorted {
        let _ = writeln!(tree, "{path}\0{}", Digest::of(bytes).hex());
    }
    Digest::of(tree.as_bytes()).to_string()
}

#[cfg(test)]
mod tests;
