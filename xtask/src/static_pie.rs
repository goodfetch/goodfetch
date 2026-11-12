//! `xtask static-pie`: each Product's executable built for a static musl
//! target is a static-PIE with packed relative relocations (ADR 0024's note
//! of 2026-11-12), so it loads at a randomised base and its relocations
//! cost a few kibibytes. It reads the executables a manifest of built files
//! names, as `xtask size` does, and holds each `*-linux-musl` one to four
//! facts of its ELF image:
//!
//! - its type is `ET_DYN`, position-independent, not `ET_EXEC` at a fixed
//!   base, which is what Zig's `cc` links by itself;
//! - it has no `PT_INTERP` program header, so no dynamic loader;
//! - its dynamic section has no `DT_NEEDED` entry, so no shared library;
//! - its dynamic section has a `DT_RELR` entry, the packed relocations
//!   musl's start-up applies.
//!
//! The other targets' executables are not read.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;

use anyhow::{Context as _, Result, bail};

use crate::check::Finding;
use crate::size;

const ET_DYN: u16 = 3;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_RELR: u64 = 36;

/// A 64-bit file header's and program header's sizes, and a dynamic
/// entry's.
const FILE_HEADER: usize = 64;
const PROGRAM_HEADER: usize = 56;
const DYNAMIC_ENTRY: usize = 16;

/// How a linked ELF executable loads, as its headers say.
struct Linkage {
    position_independent: bool,
    interpreter: bool,
    shared_libraries: usize,
    packed_relocations: bool,
}

pub(crate) fn check(root: &Path, files: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for executable in size::executables(root, files)? {
        if !executable.target.ends_with("-linux-musl") {
            continue;
        }
        let linkage = read(&executable.path)
            .with_context(|| format!("reading {}", executable.path.display()))?;
        findings.extend(
            problems(&linkage)
                .into_iter()
                .map(|(kind, message)| Finding::new(kind, executable.id.clone(), message)),
        );
    }
    Ok(findings)
}

/// What keeps an executable from being a static-PIE with packed
/// relocations. One linked at a fixed base has nothing else to say.
fn problems(linkage: &Linkage) -> Vec<(&'static str, String)> {
    if !linkage.position_independent {
        return vec![(
            "fixed-base",
            "it is linked at a fixed base (its type is not `ET_DYN`), so it loads without address-space layout randomisation; the static musl build links through ci/musl/link-cc".to_owned(),
        )];
    }
    let mut problems = Vec::new();
    if linkage.interpreter {
        problems.push((
            "interpreter",
            "it names a dynamic loader (`PT_INTERP`), so it is not static".to_owned(),
        ));
    }
    if linkage.shared_libraries > 0 {
        problems.push((
            "shared-library",
            format!(
                "it needs {} shared libraries (`DT_NEEDED`), so it is not static",
                linkage.shared_libraries
            ),
        ));
    }
    if !linkage.packed_relocations {
        problems.push((
            "relocations-not-packed",
            "its dynamic section has no `DT_RELR` entry: its relative relocations are not packed (`-z pack-relative-relocs`)".to_owned(),
        ));
    }
    problems
}

/// Reads the linkage of the 64-bit little-endian ELF image at `path`, the
/// form both static musl targets have.
fn read(path: &Path) -> Result<Linkage> {
    let mut file = File::open(path)?;
    let header = bytes_at(&mut file, 0, FILE_HEADER).context("reading the ELF file header")?;
    if !header.starts_with(b"\x7fELF\x02\x01") {
        bail!("it is not a 64-bit little-endian ELF image");
    }
    let kind = u16::from_le_bytes(field(&header, 16));
    let program_headers = u64::from_le_bytes(field(&header, 32));
    let entry_size = usize::from(u16::from_le_bytes(field(&header, 54)));
    let count = u64::from(u16::from_le_bytes(field(&header, 56)));
    if count > 0 && entry_size < PROGRAM_HEADER {
        bail!(
            "its program headers are {entry_size} bytes each, under a 64-bit header's {PROGRAM_HEADER}"
        );
    }

    let mut linkage = Linkage {
        position_independent: kind == ET_DYN,
        interpreter: false,
        shared_libraries: 0,
        packed_relocations: false,
    };
    for index in 0..count {
        let header = bytes_at(
            &mut file,
            program_headers + index * entry_size as u64,
            PROGRAM_HEADER,
        )
        .context("reading a program header")?;
        match u32::from_le_bytes(field(&header, 0)) {
            PT_INTERP => linkage.interpreter = true,
            PT_DYNAMIC => {
                let offset = u64::from_le_bytes(field(&header, 8));
                let size = usize::try_from(u64::from_le_bytes(field(&header, 32)))
                    .context("the dynamic section's size")?;
                let dynamic =
                    bytes_at(&mut file, offset, size).context("reading the dynamic section")?;
                for entry in dynamic.as_chunks::<DYNAMIC_ENTRY>().0 {
                    match u64::from_le_bytes(field(entry, 0)) {
                        DT_NULL => break,
                        DT_NEEDED => linkage.shared_libraries += 1,
                        DT_RELR => linkage.packed_relocations = true,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(linkage)
}

/// The `size` bytes of `file` from `offset`.
fn bytes_at(file: &mut File, offset: u64, size: usize) -> Result<Vec<u8>> {
    // An image cut short, or a size its header lies about, fails here
    // before anything that large is allocated.
    let length = file.metadata()?.len();
    if offset.saturating_add(size as u64) > length {
        bail!("the file ends at {length} bytes, before {size} bytes from {offset}");
    }
    let mut bytes = vec![0; size];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// The `N` bytes of `bytes` from `offset`, which the caller read long
/// enough to hold them.
fn field<const N: usize>(bytes: &[u8], offset: usize) -> [u8; N] {
    let mut field = [0; N];
    field.copy_from_slice(&bytes[offset..offset + N]);
    field
}
