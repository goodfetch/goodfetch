//! `xtask counts --strace` (ADR 0140): the merge gate's cross-check of the
//! counted opens, folders and syncs from outside the process. It runs B1 and
//! B2 under `strace`, tracing opens, folder creations and syncs alone, and
//! fails when what `strace` saw differs from what the counters counted, so
//! work done beside a counted function, by a raw call, fails the cell:
//!
//! - **Opens and folders:** each creation of a Run trace's folder, and each
//!   open for writing of a file in it, against `E5.run_file_opens`,
//!   `E5.file_opens` and `C1.file_opens`, which count both at the one
//!   function each goes through.
//!   The Provenance log's, which its Run holds, are the Run's and no page's
//!   `F1.file_opens`. A read of a
//!   trace file is no row's work: the benchmark reads the entries it divides
//!   by. A creation `create_dir_all` makes of a missing parent is that
//!   parent's own, and its first try at the folder, refused for the missing
//!   parent, is not one.
//! - **Syncs:** each sync of a file the cell's counting VFS opened, against
//!   `F2.syncs` and the syncs the benchmark's own setup, such as its Limit,
//!   did through a counting VFS of its own on the same files. A folder's
//!   sync, which SQLite does inside a counted file sync once a journal is
//!   new, is of no such file.
//!
//! Each family's benchmark runs as `--strace`, which leaves out what runs
//! beside its cells, such as Callgrind's own runs, and `--strace --bypass
//! <work>` has it do one raw unit of that work per unit beside the counted
//! one. Off Linux, or with no `strace` on a local host, the cell lists itself
//! as skipped (ADR 0061); in CI a missing `strace` fails.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context as _, Result, bail};

use super::{Arguments, Cell, FAMILIES, Family};
use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::output;

/// A raw open of a Run-trace file.
pub(super) const OPEN: &str = "open";
/// A raw creation of a Run trace's folder.
pub(super) const DIRECTORY: &str = "directory";
/// A raw sync of a file the counting VFS holds.
pub(super) const SYNC: &str = "sync";

/// What `strace` traces: opens, folder creations and syncs, nothing else, as
/// other calls depend on scheduling (ADR 0140).
const CALLS: &str = "trace=open,openat,openat2,creat,mkdir,mkdirat,fsync,fdatasync";

/// The rows that count a Run trace's folders and the opens of its files.
const TRACE_ROWS: [&str; 3] = ["E5.run_file_opens", "E5.file_opens", "C1.file_opens"];

/// The row that counts the syncs of the files the counting VFS holds.
const SYNC_ROW: &str = "F2.syncs";

/// The folder a Run trace's files sit in.
const TRACE_FOLDER: &str = "trace";

/// The part of the merge gate this is, as a skip names it.
const PART: &str = "xtask counts --strace";

pub(super) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    if !cfg!(target_os = "linux") {
        return skip(environment, "strace runs on Linux only");
    }
    if !environment.has_program("strace") {
        return skip(environment, "strace is not installed on this host");
    }
    let named: Vec<(&str, &str)> = arguments
        .executable
        .iter()
        .filter_map(|named| named.split_once('='))
        .collect();
    let families: Vec<&Family> = FAMILIES
        .iter()
        .filter(|family| !family.bypasses.is_empty())
        .filter(|family| {
            arguments
                .family
                .as_deref()
                .is_none_or(|asked| asked == family.name)
        })
        // A family named by its executable is what this run asks for.
        .filter(|family| named.is_empty() || named.iter().any(|(name, _)| *name == family.name))
        .collect();
    if families.is_empty() {
        bail!("no family the strace cell runs matches the family asked for");
    }
    if let Some(bypass) = &arguments.bypass
        && !families
            .iter()
            .any(|family| family.bypasses.contains(&bypass.as_str()))
    {
        bail!("no family asked for plants the bypass {bypass}");
    }
    let at_once = super::callgrind_at_once(arguments, environment)?;
    let mut findings = Vec::new();
    for family in families {
        let executable = match named.iter().find(|(name, _)| *name == family.name) {
            Some((_, path)) => environment.path(Path::new(path)),
            None => super::build(family, false, environment)?,
        };
        if arguments.prove {
            for bypass in family.bypasses {
                let caught = !traced(family, &executable, Some(bypass), at_once)?
                    .findings()
                    .is_empty();
                output::line(&format!(
                    "{} {bypass}: one planted bypass {}",
                    family.name,
                    if caught { "fails" } else { "passes unseen" }
                ));
                if !caught {
                    findings.push(Finding::new(
                        "strace-blind",
                        format!("{}.{bypass}", family.name),
                        "one raw unit of its work beside the counted function did not fail the cell",
                    ));
                }
            }
        } else {
            let bypass = arguments
                .bypass
                .as_deref()
                .filter(|bypass| family.bypasses.contains(bypass));
            let seen = traced(family, &executable, bypass, at_once)?;
            seen.print();
            findings.extend(seen.findings());
        }
    }
    Ok(check::report(&findings))
}

/// Lists the cell as skipped for `reason`, as `ci/tool.sh` does; CI runs
/// every part, so there it fails.
fn skip(environment: &Environment, reason: &str) -> Result<ExitCode> {
    if environment.ci {
        bail!("{reason}, and CI runs every part");
    }
    let target = environment.root.join("target");
    std::fs::create_dir_all(&target).with_context(|| format!("creating {}", target.display()))?;
    output::append(
        &target.join("skipped.txt"),
        &format!("{PART}: skipped, {reason}\n"),
    )?;
    output::error(&format!("skipped: {PART} ({reason})"));
    Ok(ExitCode::SUCCESS)
}

/// What `strace` saw of one family's run and what its cells counted.
struct Seen {
    family: &'static str,
    /// Whether its cells count syncs, which the cell then holds them to.
    syncs_counted: bool,
    traced: Totals,
    counted: Totals,
    /// The syncs of `counted` the benchmark's own setup did.
    setup_syncs: u64,
}

#[derive(Default)]
struct Totals {
    /// A Run trace's folder creations and its files' opens for writing.
    opens: u64,
    /// The syncs of the files the counting VFS holds.
    syncs: u64,
}

impl Seen {
    fn print(&self) {
        output::line(&format!(
            "{} opens and folders: strace {}, counted {} ({})",
            self.family,
            self.traced.opens,
            self.counted.opens,
            TRACE_ROWS.join(", ")
        ));
        if self.syncs_counted {
            let setup = match self.setup_syncs {
                0 => String::new(),
                syncs => format!(", and {syncs} of the fixture's setup"),
            };
            output::line(&format!(
                "{} syncs: strace {}, counted {} ({SYNC_ROW}{setup})",
                self.family, self.traced.syncs, self.counted.syncs
            ));
        }
    }

    fn findings(&self) -> Vec<Finding> {
        let mut findings = Vec::new();
        if self.traced.opens != self.counted.opens {
            findings.push(Finding::new(
                "strace-differs",
                format!("{}.opens", self.family),
                format!(
                    "strace saw {} Run-trace opens and folder creations, the counters {}",
                    self.traced.opens, self.counted.opens
                ),
            ));
        }
        if self.syncs_counted && self.traced.syncs != self.counted.syncs {
            findings.push(Finding::new(
                "strace-differs",
                format!("{}.syncs", self.family),
                format!(
                    "strace saw {} syncs of the counted files, the counters {}",
                    self.traced.syncs, self.counted.syncs
                ),
            ));
        }
        findings
    }
}

/// Runs `executable`, `family`'s benchmark, under `strace` with `bypass`
/// planted and at most `at_once` Callgrind processes at once, and answers
/// what it saw beside what its cells counted.
fn traced(
    family: &Family,
    executable: &Path,
    bypass: Option<&str>,
    at_once: Option<NonZeroUsize>,
) -> Result<Seen> {
    let folder = tempfile::tempdir().context("creating the folder of strace's output")?;
    let prefix = folder.path().join("calls");
    let mut command = Command::new("strace");
    // `-ff` writes each thread's calls to a file of its own, so no call is
    // split by another thread's; `-y` names the file behind each descriptor.
    command
        .args(["-f", "-ff", "-qq", "-y", "--seccomp-bpf", "-e", CALLS])
        .args(["-e", "signal=none", "-o"])
        .arg(&prefix)
        .arg(executable)
        .arg("--strace");
    if let Some(bypass) = bypass {
        command.args(["--bypass", bypass]);
    }
    let cells = super::printed_cells(command, executable, at_once)?;
    let files: BTreeSet<&Path> = cells
        .iter()
        .flat_map(|cell| cell.vfs_files.iter().map(PathBuf::as_path))
        .collect();
    let mut traced = Totals::default();
    for entry in std::fs::read_dir(folder.path())
        .with_context(|| format!("reading {}", folder.path().display()))?
    {
        let thread = entry.context("reading strace's output")?.path();
        let log = std::fs::read_to_string(&thread)
            .with_context(|| format!("reading {}", thread.display()))?;
        for call in log.lines().filter_map(Call::of) {
            traced.opens += u64::from(call.is_trace_work());
            traced.syncs += u64::from(call.syncs_one_of(&files));
        }
    }
    let count = |row: &str| -> u64 {
        cells
            .iter()
            .filter_map(|cell: &Cell| cell.counts.get(row))
            .sum()
    };
    let setup_syncs = cells.iter().map(|cell| cell.setup_syncs).sum();
    Ok(Seen {
        family: family.name,
        syncs_counted: family.bypasses.contains(&SYNC),
        traced,
        counted: Totals {
            opens: TRACE_ROWS.iter().map(|row| count(row)).sum(),
            syncs: count(SYNC_ROW) + setup_syncs,
        },
        setup_syncs,
    })
}

/// One system call as `strace -y` prints it: `name(arguments) = result`.
struct Call<'a> {
    name: &'a str,
    arguments: &'a str,
    result: &'a str,
}

impl<'a> Call<'a> {
    fn of(line: &'a str) -> Option<Self> {
        let (name, rest) = line.split_once('(')?;
        let (arguments, result) = rest.rsplit_once(") = ")?;
        Some(Self {
            name,
            arguments,
            result,
        })
    }

    /// Whether this creates a Run trace's folder or opens one of its files
    /// for writing.
    fn is_trace_work(&self) -> bool {
        match self.name {
            "mkdir" | "mkdirat" => {
                // The first try at a folder whose parent is missing is
                // refused, and `create_dir_all` tries again once it made it.
                !self.result.contains("ENOENT")
                    && quoted(self.arguments).is_some_and(|(path, _)| {
                        Path::new(&path).file_name() == Some(TRACE_FOLDER.as_ref())
                    })
            }
            "open" | "openat" | "openat2" | "creat" => {
                let Some((path, flags)) = quoted(self.arguments) else {
                    return false;
                };
                let writes = self.name == "creat"
                    || ["O_WRONLY", "O_RDWR", "O_CREAT"]
                        .iter()
                        .any(|flag| flags.contains(flag));
                writes
                    && self
                        .opened(&path)
                        .parent()
                        .and_then(Path::file_name)
                        .is_some_and(|folder| folder == TRACE_FOLDER)
            }
            _ => false,
        }
    }

    /// The file an open of `path` reached: the one its descriptor names, else
    /// `path` from the folder its call started from.
    fn opened(&self, path: &str) -> PathBuf {
        if let Some(file) = decoration(self.result) {
            return PathBuf::from(file);
        }
        let start = self
            .arguments
            .split_once('"')
            .map_or("", |(start, _)| start);
        decoration(start).map_or_else(
            || PathBuf::from(path),
            |folder| Path::new(folder).join(path),
        )
    }

    /// Whether this syncs one of `files`.
    fn syncs_one_of(&self, files: &BTreeSet<&Path>) -> bool {
        matches!(self.name, "fsync" | "fdatasync")
            && decoration(self.arguments).is_some_and(|file| files.contains(Path::new(file)))
    }
}

/// The first quoted string of `arguments`, unescaped, and what follows it.
fn quoted(arguments: &str) -> Option<(String, &str)> {
    let (_, rest) = arguments.split_once('"')?;
    let mut text = String::new();
    let mut characters = rest.char_indices();
    while let Some((at, character)) = characters.next() {
        match character {
            '"' => return Some((text, &rest[at + 1..])),
            '\\' => text.extend(characters.next().map(|(_, escaped)| escaped)),
            other => text.push(other),
        }
    }
    None
}

/// The file `-y` names in `text`, between its first `<` and its last `>`.
fn decoration(text: &str) -> Option<&str> {
    let start = text.find('<')?;
    let end = text.rfind('>')?;
    (end > start).then(|| &text[start + 1..end])
}
