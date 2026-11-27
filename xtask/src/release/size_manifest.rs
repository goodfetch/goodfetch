//! `xtask release size-manifest`: builds the TOML manifest `xtask size
//! ceilings` reads over both Products' release-profile files (ADRs 0057, 0074,
//! 0087 and 0094; issue 203):
//! - raw executables per target (`goodfetch-collie`, `goodfetch-foxhound`);
//! - GitHub Release archives (`goodfetch-{collie,foxhound}-<target>.tar.xz`);
//! - Python `py3-none` platform wheels (`goodfetch_{collie,foxhound}-*.whl`);
//! - npm platform package tarballs (`goodfetch-{collie,foxhound}-*.tgz`) when
//!   packed;
//! - published crate tarballs (`*.crate`) when present under `crates/`.
//!
//! Goodfetch Chromium (`goodfetch-chromium-*`) and split debug symbols
//! (`*-debug.tar.xz`) are intentionally excluded: Chromium has no size ceiling
//! (its +5 MB increase threshold is the Chromium build gate's, ADR 0074), and
//! debug symbols are published beside the executables without counting against
//! the executable or archive ceilings (ADR 0094).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};

use super::assets;
use crate::output;
use crate::products::Product;

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// Folder holding `<target>/goodfetch-{collie,foxhound}[.exe]`.
    #[arg(long)]
    binaries: Option<PathBuf>,
    /// Packaged release folder (`assets/`, `wheels/`, `npm/`, optional `crates/`).
    #[arg(long)]
    packaged: PathBuf,
    /// Destination path for the generated `xtask size` TOML manifest.
    #[arg(long)]
    out: PathBuf,
    /// Cargo profile name recorded in the manifest (`release` for ceilings).
    #[arg(long, default_value = "release")]
    profile: String,
}

pub(super) fn run(root: &Path, arguments: &Arguments) -> Result<ExitCode> {
    let packaged = resolve(root, &arguments.packaged);
    let out = resolve(root, &arguments.out);

    let mut manifest = String::new();
    let _ = writeln!(manifest, "profile = {:?}", arguments.profile);

    if let Some(binaries) = &arguments.binaries {
        let binaries_dir = resolve(root, binaries);
        append_executables(root, &binaries_dir, &mut manifest);
    }

    append_release_archives(root, &packaged.join("assets"), &mut manifest);
    append_wheels(root, &packaged.join("wheels"), &mut manifest)?;
    append_npm_tarballs(root, &packaged.join("npm"), &mut manifest)?;
    append_crate_tarballs(root, &packaged.join("crates"), &mut manifest)?;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&out, manifest).with_context(|| format!("writing {}", out.display()))?;
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

fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn push_entry(
    out: &mut String,
    product: Product,
    kind: &str,
    target: Option<&str>,
    name: &str,
    path: &str,
) {
    let _ = writeln!(out, "\n[[file]]");
    let _ = writeln!(out, "product = {:?}", product.key());
    let _ = writeln!(out, "kind = {kind:?}");
    if let Some(target) = target {
        let _ = writeln!(out, "target = {target:?}");
    }
    let _ = writeln!(out, "name = {name:?}");
    let _ = writeln!(out, "path = {path:?}");
}

fn append_executables(root: &Path, binaries_dir: &Path, out: &mut String) {
    for target in assets::TARGETS {
        let suffix = if target.contains("windows") {
            ".exe"
        } else {
            ""
        };
        for product in Product::ALL {
            let name = format!("{}{suffix}", product.names().executable);
            let path = binaries_dir.join(target).join(&name);
            if path.is_file() {
                push_entry(
                    out,
                    product,
                    "executable",
                    Some(target),
                    &name,
                    &display_path(root, &path),
                );
            }
        }
    }
}

fn append_release_archives(root: &Path, assets_dir: &Path, out: &mut String) {
    // Only `<executable>-<target>.tar.xz` archives of the two Products enter
    // ceilings: Goodfetch Chromium (`goodfetch-chromium-*`) and split debug
    // symbols (`*-debug.tar.xz`) are intentionally excluded (ADRs 0074, 0094).
    for target in assets::TARGETS {
        for product in Product::ALL {
            let name = assets::archive(product, target);
            let path = assets_dir.join(&name);
            if path.is_file() {
                push_entry(
                    out,
                    product,
                    "release-archive",
                    Some(target),
                    &name,
                    &display_path(root, &path),
                );
            }
        }
    }
}

fn sorted_files(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn append_wheels(root: &Path, wheels_dir: &Path, out: &mut String) -> Result<()> {
    for path in sorted_files(wheels_dir)? {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".whl") else {
            continue;
        };
        let product = if name.starts_with("goodfetch_collie-") {
            Product::Collie
        } else if name.starts_with("goodfetch_foxhound-") {
            Product::Foxhound
        } else {
            continue;
        };
        let target = stem.rsplit('-').next().unwrap_or("any");
        push_entry(
            out,
            product,
            "wheel",
            Some(target),
            name,
            &display_path(root, &path),
        );
    }
    Ok(())
}

fn append_npm_tarballs(root: &Path, npm_dir: &Path, out: &mut String) -> Result<()> {
    for path in sorted_files(npm_dir)? {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".tgz") else {
            continue;
        };
        let (product, rest) = if let Some(rest) = stem.strip_prefix("goodfetch-collie-") {
            (Product::Collie, rest)
        } else if let Some(rest) = stem.strip_prefix("goodfetch-foxhound-") {
            (Product::Foxhound, rest)
        } else {
            continue;
        };
        if rest.starts_with("client-") {
            continue;
        }
        // `goodfetch-collie-linux-x64-1.0.0.tgz` -> target `linux-x64`.
        let mut parts = rest.split('-');
        let target = match (parts.next(), parts.next()) {
            (Some(os), Some(arch)) => format!("{os}-{arch}"),
            _ => continue,
        };
        push_entry(
            out,
            product,
            "npm-package",
            Some(&target),
            name,
            &display_path(root, &path),
        );
    }
    Ok(())
}

fn append_crate_tarballs(root: &Path, crates_dir: &Path, out: &mut String) -> Result<()> {
    for path in sorted_files(crates_dir)? {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("crate"))
        {
            continue;
        }
        let product = if name.starts_with("goodfetch-foxhound") {
            Product::Foxhound
        } else {
            Product::Collie
        };
        push_entry(
            out,
            product,
            "crate",
            None,
            name,
            &display_path(root, &path),
        );
    }
    Ok(())
}
