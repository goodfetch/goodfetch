//! Whether the Usage ledger sits on a temporary filesystem (ADR 0041): in
//! a container without a persistent volume, the ledger is lost on each
//! restart and period Limits start again silently, so the CLI warns. On
//! Linux the mount table names each mount's filesystem type; elsewhere no
//! temporary filesystem is found.

use std::path::{Path, PathBuf};

/// The filesystem types that live only in memory.
const TEMPORARY: &[&str] = &["tmpfs", "ramfs"];

/// The temporary filesystem type of the mount holding `path`, read from
/// the OS's mount table, if it is one. Blocking.
pub fn temporary_filesystem(path: &Path) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        #[expect(
            clippy::disallowed_methods,
            reason = "one small read of the mount table on the CLI's own thread, before any runtime starts"
        )]
        let mounts = std::fs::read_to_string("/proc/self/mounts").ok()?;
        temporary_in(&mounts, &existing(path))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        None
    }
}

/// The temporary filesystem type of the mount of `mounts`, a mount table
/// in `/proc/self/mounts`' form, that holds `path`, an absolute path with
/// no `..`: the deepest mount point above it, the last mounted there when
/// several are.
pub fn temporary_in(mounts: &str, path: &Path) -> Option<String> {
    let mut holding: Option<(usize, &str)> = None;
    for line in mounts.lines() {
        let mut fields = line.split(' ');
        let (Some(_), Some(point), Some(kind)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let point = unescape(point);
        if !path.starts_with(&point) {
            continue;
        }
        let depth = point.components().count();
        if holding.is_none_or(|(deepest, _)| depth >= deepest) {
            holding = Some((depth, kind));
        }
    }
    holding
        .map(|(_, kind)| kind)
        .filter(|kind| TEMPORARY.contains(kind))
        .map(str::to_owned)
}

/// A mount point as the mount table writes it, with its space, tab,
/// newline and backslash as octal escapes.
fn unescape(field: &str) -> PathBuf {
    let mut text = String::with_capacity(field.len());
    let mut rest = field;
    while let Some(at) = rest.find('\\') {
        text.push_str(&rest[..at]);
        let escape = rest.get(at + 1..at + 4);
        if let Some(byte) = escape.and_then(|digits| u8::from_str_radix(digits, 8).ok()) {
            text.push(char::from(byte));
            rest = &rest[at + 4..];
        } else {
            text.push('\\');
            rest = &rest[at + 1..];
        }
    }
    text.push_str(rest);
    PathBuf::from(text)
}

/// `path` resolved through the nearest of its folders that exists, since
/// the ledger may not be written yet.
#[cfg(target_os = "linux")]
fn existing(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut found = path.to_owned();
    loop {
        if let Ok(resolved) = found.canonicalize() {
            return missing
                .into_iter()
                .rev()
                .fold(resolved, |path, part| path.join(part));
        }
        match (found.file_name().map(ToOwned::to_owned), found.parent()) {
            (Some(name), Some(parent)) => {
                missing.push(name);
                found = parent.to_owned();
            }
            _ => return path.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests;
