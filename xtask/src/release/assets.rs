//! `xtask release assets`: the GitHub Release's files, gathered in one
//! folder, made ready to upload. Each Product's archive and debug symbols
//! per target and its `CycloneDX` SBOM, each named after its executable, must
//! be there (ADR 0094); the other files, the Goodfetch Chromium's bundles
//! and chromedriver, the Clean-up rules' assets and the rest, are taken as
//! they are. A file over the part size, under GitHub's 2 GiB limit per
//! release asset, is split into numbered parts below it, and the digest
//! table `SHA256SUMS` lists each uploaded file and part with the whole
//! file's digest beside its parts' (ADR 0074), in `sha256sum`'s form: a
//! download joins the parts in order (`cat <file>.part-* > <file>`) and
//! checks the whole with `sha256sum --check --ignore-missing SHA256SUMS`.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read as _, Write};
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail, ensure};
use sha2::{Digest as _, Sha256};

use crate::check::{self, Finding};
use crate::output;
use crate::products::Product;

/// The digest table's name, among the files it lists.
pub(super) const TABLE: &str = "SHA256SUMS";
/// The largest part, in bytes: 2,000 MiB, below GitHub's 2 GiB limit.
pub(super) const PART_SIZE: u64 = 2000 << 20;
/// What a part's name adds to its file's, before its number.
const PART: &str = ".part-";
/// The number of digits in a part's number.
const PART_DIGITS: usize = 3;

/// The targets each Product ships an archive and debug symbols for: Linux
/// x64 and arm64 on glibc and musl, macOS arm64 and x64, and Windows x64
/// and arm64 (ADR 0024).
const TARGETS: [&str; 8] = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
];

/// The files each Product must put on the GitHub Release, each named after
/// its executable (ADR 0094): its archive per target, which
/// `[package.metadata.binstall]` and `docs/install.md` name, its debug
/// symbols per target, and its SBOM.
fn product_files() -> Vec<String> {
    let mut files = Vec::new();
    for product in Product::ALL {
        let executable = product.names().executable;
        for target in TARGETS {
            files.push(format!("{executable}-{target}.tar.xz"));
            files.push(format!("{executable}-{target}-debug.tar.xz"));
        }
        files.push(format!("{executable}.cdx.json"));
    }
    files
}

/// Splits each file of `folder` over `part_size` bytes and writes its
/// digest table, or only an `error[missing]` for each Product file it
/// lacks. A re-run reads a split file back from its parts.
pub(super) fn run(folder: &Path, part_size: u64) -> Result<ExitCode> {
    let files = gathered(folder)?;
    let mut missing: Vec<String> = product_files()
        .into_iter()
        .filter(|file| !files.contains_key(file))
        .collect();
    missing.sort();
    if !missing.is_empty() {
        let findings: Vec<Finding> = missing
            .iter()
            .map(|file| {
                Finding::new(
                    "missing",
                    file,
                    format!("not in {}: each Product's archive and debug symbols per target and its SBOM go on the release", folder.display()),
                )
            })
            .collect();
        return Ok(check::report(&findings));
    }

    let mut table = BTreeMap::new();
    for (file, parts) in files {
        let parts = match parts {
            Parts::Whole if folder.join(&file).metadata()?.len() > part_size => {
                let parts = split(folder, &file, part_size)?;
                for part in &parts {
                    output::line(&folder.join(part).display().to_string());
                }
                parts
            }
            Parts::Whole => vec![file.clone()],
            Parts::Split(parts) => parts,
        };
        let (whole, digests) = digests(folder, &parts)?;
        if parts.len() > 1 {
            table.extend(parts.into_iter().zip(digests));
        }
        table.insert(file, whole);
    }
    let text = table
        .iter()
        .map(|(file, digest)| format!("{digest}  {file}\n"))
        .collect::<Vec<_>>()
        .concat();
    let path = folder.join(TABLE);
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    output::line(&path.display().to_string());
    Ok(ExitCode::SUCCESS)
}

/// How a file of the folder is there.
enum Parts {
    Whole,
    /// Its parts' names, in order, the file itself gone.
    Split(Vec<String>),
}

/// Each file of `folder` but the digest table, by its name. A part left
/// beside its whole file, from a split cut short, is removed, so the split
/// runs again.
fn gathered(folder: &Path) -> Result<BTreeMap<String, Parts>> {
    let mut wholes = Vec::new();
    let mut parts: BTreeMap<String, BTreeMap<usize, String>> = BTreeMap::new();
    let entries =
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?;
    for entry in entries {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|name| anyhow::anyhow!("{} is not UTF-8", name.display()))?;
        ensure!(
            entry.file_type()?.is_file(),
            "{} holds {name}, which is not a file: the release's files are gathered flat",
            folder.display()
        );
        if name == TABLE {
            continue;
        }
        match part_of(&name) {
            Some((file, number)) => {
                parts
                    .entry(file.to_owned())
                    .or_default()
                    .insert(number, name);
            }
            None => wholes.push(name),
        }
    }

    let mut files: BTreeMap<String, Parts> = BTreeMap::new();
    for whole in wholes {
        for stale in parts.remove(&whole).unwrap_or_default().into_values() {
            std::fs::remove_file(folder.join(&stale))
                .with_context(|| format!("removing {stale}"))?;
        }
        files.insert(whole, Parts::Whole);
    }
    for (file, numbered) in parts {
        if !numbered.keys().copied().eq(1..=numbered.len()) {
            bail!("{file}'s parts in {} skip a number", folder.display());
        }
        files.insert(file, Parts::Split(numbered.into_values().collect()));
    }
    Ok(files)
}

/// The file and number a part's name holds, `x.part-001` being `x`'s first.
fn part_of(name: &str) -> Option<(&str, usize)> {
    let (file, number) = name.rsplit_once(PART)?;
    if number.len() != PART_DIGITS || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((file, number.parse().ok()?))
}

fn part_name(file: &str, number: usize) -> String {
    format!("{file}{PART}{number:0PART_DIGITS$}")
}

/// Cuts `file` into parts of at most `part_size` bytes, then removes it.
fn split(folder: &Path, file: &str, part_size: u64) -> Result<Vec<String>> {
    let path = folder.join(file);
    let mut source = File::open(&path).with_context(|| format!("opening {}", path.display()))?;
    let size = source.metadata()?.len();
    let count = size.div_ceil(part_size);
    ensure!(
        count < 10_u64.pow(u32::try_from(PART_DIGITS)?),
        "{file} would take {count} parts, more than {PART_DIGITS} digits number"
    );
    let mut parts = Vec::new();
    for number in 1..=usize::try_from(count)? {
        let name = part_name(file, number);
        let target = folder.join(&name);
        let mut sink =
            File::create(&target).with_context(|| format!("creating {}", target.display()))?;
        io::copy(&mut (&mut source).take(part_size), &mut sink)
            .with_context(|| format!("writing {}", target.display()))?;
        sink.flush()?;
        parts.push(name);
    }
    std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    Ok(parts)
}

/// The SHA-256 of `parts` joined in order, and of each.
fn digests(folder: &Path, parts: &[String]) -> Result<(String, Vec<String>)> {
    let mut whole = Sha256::new();
    let mut each = Vec::new();
    for part in parts {
        let path = folder.join(part);
        let mut file = File::open(&path).with_context(|| format!("opening {}", path.display()))?;
        let mut sink = Hashing {
            part: Sha256::new(),
            whole: &mut whole,
        };
        io::copy(&mut file, &mut sink).with_context(|| format!("reading {}", path.display()))?;
        each.push(format!("{:x}", sink.part.finalize()));
    }
    Ok((format!("{:x}", whole.finalize()), each))
}

/// A part's bytes hashed alone and as part of the whole.
struct Hashing<'a> {
    part: Sha256,
    whole: &'a mut Sha256,
}

impl Write for Hashing<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.part.update(buffer);
        self.whole.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
