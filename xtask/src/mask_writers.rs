//! The Mask writers check (spec #564): every writer the Products' crates
//! hold is listed with how it meets the `Mask`, in
//! `crates/goodfetch-engine/src/mask/writers.toml`. Each place their code
//! opens a file, writes one whole, gives the `tracing` subscriber a writer,
//! takes stdout or stderr, creates a SQLite table or writes a Connection's
//! lines is found in the source and must be listed, so a new writer cannot
//! skip the `Mask` unseen. Tests, benchmarks, examples, fuzz targets, build
//! scripts, generated code and the test-only crates write no Product output
//! and are left out.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;
use crate::tree;

/// The list, from the root.
const LIST: &str = "crates/goodfetch-engine/src/mask/writers.toml";

/// What finds a writer in a line of source.
const WRITING: [&str; 9] = [
    "OpenOptions::new()",
    "fs::write(",
    "File::create(",
    "File::create_new(",
    ".with_writer(",
    "io::stdout()",
    "io::stderr()",
    "CREATE TABLE",
    "fn write_lines(",
];

/// The parts of a path that hold no Product code.
const LEFT_OUT: [&str; 9] = [
    "/tests/",
    "/benches/",
    "/examples/",
    "/fuzz/",
    "/build/",
    "/generated/",
    "crates/goodfetch-test-support/",
    "crates/goodfetch-conformance/",
    "crates/goodfetch-stub-gateway/",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    writer: Vec<Writer>,
}

/// One file holding writers.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Writer {
    path: String,
    /// How many of [`WRITING`] it holds.
    count: usize,
    /// How they meet the `Mask`.
    how: String,
}

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let text =
        std::fs::read_to_string(root.join(LIST)).with_context(|| format!("reading {LIST}"))?;
    let list: List = toml::from_str(&text).with_context(|| format!("reading {LIST}"))?;
    let mut found = found(root)?;
    let mut findings = Vec::new();
    for writer in &list.writer {
        match found.remove(&writer.path) {
            None => findings.push(Finding::new(
                "mask-writer-gone",
                &writer.path,
                format!("{LIST} lists it, and it holds no writer now: remove its entry"),
            )),
            Some(count) if count != writer.count => findings.push(Finding::new(
                "mask-writer-count",
                &writer.path,
                format!(
                    "{LIST} lists {} writers, and it holds {count}: say how the new one meets the `Mask`, then change its count",
                    writer.count
                ),
            )),
            Some(_) if writer.how.trim().is_empty() => findings.push(Finding::new(
                "mask-writer-unexplained",
                &writer.path,
                format!("{LIST} does not say how it meets the `Mask`"),
            )),
            Some(_) => {}
        }
    }
    findings.extend(found.into_iter().map(|(path, count)| {
        Finding::new(
            "mask-writer-unlisted",
            path,
            format!(
                "it holds {count} writers {LIST} does not list: add it, saying how they meet the `Mask`"
            ),
        )
    }));
    Ok(findings)
}

/// Each Product file holding writers, with how many [`WRITING`] finds in its
/// lines that are not comments.
fn found(root: &Path) -> Result<BTreeMap<String, usize>> {
    let mut found = BTreeMap::new();
    for path in tree::files(root, "crates")? {
        if !is_product_code(&path) {
            continue;
        }
        let source =
            std::fs::read_to_string(root.join(&path)).with_context(|| format!("reading {path}"))?;
        let count = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .map(|line| {
                WRITING
                    .iter()
                    .filter(|writing| line.contains(*writing))
                    .count()
            })
            .sum();
        if count > 0 {
            found.insert(path, count);
        }
    }
    Ok(found)
}

/// Whether `path`, a file relative to the root, is Product code in Rust.
fn is_product_code(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|extension| extension == "rs")
        && !path.ends_with("tests.rs")
        && !path.ends_with("/build.rs")
        && !LEFT_OUT.iter().any(|part| path.contains(part))
}
