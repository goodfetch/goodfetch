//! gungraun's summaries (format 7, `--output-format=json`): one JSON object
//! a line, one line per benchmark. A head run with `--baseline=base` holds
//! each metric's head value as `new` and the base's as `old`; a benchmark
//! the base lacks has `new` only.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use super::figure::{Figure, Measured};

/// The summary format `xtask perf` reads.
const VERSION: &str = "7";

#[derive(Deserialize)]
struct Summary {
    version: String,
    module_path: String,
    id: Option<String>,
    package_dir: String,
    profiles: Vec<Profile>,
}

#[derive(Deserialize)]
struct Profile {
    data: Data,
}

#[derive(Deserialize)]
struct Data {
    total: Total,
}

#[derive(Deserialize)]
struct Total {
    metrics: BTreeMap<String, MetricResult>,
}

#[derive(Deserialize)]
struct MetricResult {
    values: Values,
}

#[derive(Deserialize)]
struct Values {
    new: Option<Value>,
    old: Option<Value>,
}

/// A benchmark's ID: its crate (the folder of its package, named after the
/// crate), its module path and, when it has one, its `#[bench::id]`.
fn benchmark_id(summary: &Summary) -> String {
    let package = summary
        .package_dir
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    match &summary.id {
        Some(id) => format!("{package}/{}/{id}", summary.module_path),
        None => format!("{package}/{}", summary.module_path),
    }
}

/// Where a benchmark is written, read back from its ID.
pub(crate) struct Source<'a> {
    /// The folder of its package.
    pub(crate) package: &'a str,
    /// Its bench target, `benches/<file>.rs`.
    pub(crate) file: &'a str,
    pub(crate) function: &'a str,
}

/// Where the benchmark `subject` names is written, read back from
/// [`benchmark_id`]'s `<package>/<file>::<group>::<function>[/<id>]`; none
/// for a subject that names no benchmark.
pub(crate) fn source(subject: &str) -> Option<Source<'_>> {
    let (package, module_path) = subject.split_once('/')?;
    let module_path = module_path
        .split_once('/')
        .map_or(module_path, |(module_path, _)| module_path);
    let segments: Vec<&str> = module_path.split("::").collect();
    let [file, .., function] = segments[..] else {
        return None;
    };
    if segments.len() < 3 || [package, file, function].contains(&"") {
        return None;
    }
    Some(Source {
        package,
        file,
        function,
    })
}

/// Every gated figure of every benchmark in the files at `paths`; a figure
/// the head lacks is not measured.
pub(crate) fn read(paths: &[impl AsRef<Path>]) -> Result<Vec<Measured>> {
    let mut measured = Vec::new();
    for summary in summaries(paths)? {
        let subject = benchmark_id(&summary);
        for profile in &summary.profiles {
            for (key, figure) in Figure::BENCHMARK {
                let Some(result) = profile.data.total.metrics.get(key) else {
                    continue;
                };
                let Some(head) = count(result.values.new.as_ref(), &subject, key)? else {
                    continue;
                };
                measured.push(Measured {
                    subject: subject.clone(),
                    figure,
                    base: count(result.values.old.as_ref(), &subject, key)?,
                    head,
                });
            }
        }
    }
    Ok(measured)
}

/// The IDs of the benchmarks the files at `paths` measured, whatever their
/// figures.
pub(crate) fn benchmarks(paths: &[impl AsRef<Path>]) -> Result<BTreeSet<String>> {
    Ok(summaries(paths)?.iter().map(benchmark_id).collect())
}

fn summaries(paths: &[impl AsRef<Path>]) -> Result<Vec<Summary>> {
    let mut summaries = Vec::new();
    for path in paths.iter().map(AsRef::as_ref) {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        for (number, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let summary: Summary = serde_json::from_str(line)
                .with_context(|| format!("parsing line {} of {}", number + 1, path.display()))?;
            if summary.version != VERSION {
                bail!(
                    "{} holds gungraun's summary format {}, not {VERSION}",
                    path.display(),
                    summary.version
                );
            }
            summaries.push(summary);
        }
    }
    Ok(summaries)
}

fn count(value: Option<&Value>, subject: &str, key: &str) -> Result<Option<u64>> {
    value
        .map(|value| {
            value
                .as_u64()
                .with_context(|| format!("{subject}'s {key} is no whole count: {value}"))
        })
        .transpose()
}

#[cfg(test)]
mod tests;
