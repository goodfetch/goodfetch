//! Admission's time per URL at scale, Foxhound's side of `xtask compare
//! admission` (#2318): the Frontier admits `--urls` new synthetic links
//! found on one seed into a run file on disk, at the default Durability
//! level, `batch`, with memory holding visits within `--budget` bytes (the
//! Run state's default budget when not given), in batches of 1,000, then
//! admits each again. It times each pass, reads what memory holds after
//! each batch, counts the lookups that read the run file
//! (`goodfetch.run_state.spilled_lookups`, on a Prometheus registry as the
//! ops listener renders it) and writes the figures as one JSON object to
//! `--out`. It fails when an admission of the first pass is not `added` or
//! one of the second is not `duplicate`, so its figures are an exact
//! visited set's. It writes nothing to stdout or stderr: a failure is its
//! exit status, with `{"error": …}` written to `--out` when it has one.
//!
//! It times the Frontier's own admission on the writer's connection. A
//! crawl past the budget reads a links batch's new URLs on one of the run
//! file's readers before the batch's job (#2169), which this does not run.
//!
//!   cargo run --release -p goodfetch-frontier --example admission -- \
//!     --urls 1000000 --out admission.json

#![expect(
    clippy::disallowed_methods,
    reason = "a benchmark harness reads the time itself (ADR 0071)"
)]

#[path = "../benches/scenarios.rs"]
#[expect(
    dead_code,
    reason = "the benchmarks' other scenarios, which this harness does not run"
)]
mod scenarios;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use goodfetch_engine::metrics::{METER, Metrics};
use goodfetch_protocol::foxhound::frontier::{Enqueued, NotAddedReason};
use goodfetch_protocol::run::durability::MEMORY_BUDGET;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use serde_json::{Value, json};

use scenarios::{BATCH, RunFile};

/// The harness's arguments.
struct Arguments {
    urls: usize,
    budget: u64,
    out: PathBuf,
}

fn arguments() -> Result<Arguments, String> {
    let usage = "usage: admission --urls <n> [--budget <bytes>] --out <file>";
    let mut urls = None;
    let mut budget = MEMORY_BUDGET;
    let mut out = None;
    let mut given = std::env::args().skip(1);
    while let Some(name) = given.next() {
        let value = given.next().ok_or(usage)?;
        match name.as_str() {
            "--urls" => urls = Some(value.parse().map_err(|_| "--urls takes a count")?),
            "--budget" => budget = value.parse().map_err(|_| "--budget takes bytes")?,
            "--out" => out = Some(PathBuf::from(value)),
            _ => return Err(usage.to_owned()),
        }
    }
    Ok(Arguments {
        urls: urls.ok_or(usage)?,
        budget,
        out: out.ok_or(usage)?,
    })
}

/// One pass over every synthetic link: its time, and the most memory held
/// after any of its batches.
struct Pass {
    elapsed: Duration,
    held: u64,
}

/// Admits every link of `file`'s `urls` once more, batch by batch, and
/// fails on the first whose answer `expected` refuses.
fn pass(
    file: &mut RunFile,
    urls: usize,
    expected: fn(&Enqueued) -> bool,
    word: &str,
) -> Result<Pass, String> {
    let mut elapsed = Duration::ZERO;
    let mut held = 0;
    let mut wrong = None;
    for batch in (0..urls).step_by(BATCH) {
        let links = batch..urls.min(batch + BATCH);
        let first = links.start;
        let mut each = first;
        let started = Instant::now();
        file.admit_batch(links, |answer| {
            if wrong.is_none() && !expected(answer) {
                wrong = Some(each);
            }
            each += 1;
        });
        elapsed += started.elapsed();
        if let Some(link) = wrong {
            return Err(format!("link {link} was not {word}"));
        }
        held = held.max(
            file.invariants
                .unwritten()
                .map_or(0, goodfetch_frontier::UnwrittenVisits::bytes),
        );
    }
    Ok(Pass { elapsed, held })
}

/// Nanoseconds per URL of `pass` over `urls`.
fn per_url(pass: &Pass, urls: usize) -> f64 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "nanoseconds and counts stay far below 2^52"
    )]
    let per = pass.elapsed.as_nanos() as f64 / urls.max(1) as f64;
    per
}

/// The figures of the run `arguments` asks for.
fn measure(arguments: &Arguments) -> Result<Value, String> {
    let registry = prometheus::Registry::new();
    let exporter = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .without_target_info()
        .scope_info_enabled(false)
        .build()
        .map_err(|error| format!("building the Prometheus exporter: {error}"))?;
    let provider = SdkMeterProvider::builder().with_reader(exporter).build();
    let metrics = Metrics::new(&provider.meter(METER));
    let mut file = RunFile::open(Some(arguments.budget), &metrics);
    let added = pass(
        &mut file,
        arguments.urls,
        |answer| matches!(answer, Enqueued::Added { .. }),
        "added",
    )?;
    let duplicates = pass(
        &mut file,
        arguments.urls,
        |answer| {
            matches!(
                answer,
                Enqueued::NotAdded {
                    reason: NotAddedReason::Duplicate,
                    ..
                }
            )
        },
        "a duplicate",
    )?;
    let spilled_lookups = registry
        .gather()
        .iter()
        .find(|family| family.name() == "goodfetch_run_state_spilled_lookups_total")
        .and_then(|family| family.get_metric().first())
        .map_or(0.0, |metric| metric.get_counter().get_value());
    Ok(json!({
        "urls": arguments.urls,
        "budget_bytes": arguments.budget,
        "added_ns_per_url": per_url(&added, arguments.urls),
        "duplicate_ns_per_url": per_url(&duplicates, arguments.urls),
        "memory_held_bytes": added.held.max(duplicates.held),
        "spilled_lookups": spilled_lookups,
        "added": arguments.urls,
        "duplicates": arguments.urls,
    }))
}

/// Writes `figures` to `out`.
fn write(out: &Path, figures: &Value) -> Result<(), String> {
    std::fs::write(out, figures.to_string())
        .map_err(|error| format!("writing {}: {error}", out.display()))
}

fn main() -> ExitCode {
    let Ok(arguments) = arguments() else {
        return ExitCode::from(2);
    };
    let written = match measure(&arguments) {
        Ok(figures) => write(&arguments.out, &figures),
        Err(error) => write(&arguments.out, &json!({ "error": error })).and(Err(error)),
    };
    if written.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
