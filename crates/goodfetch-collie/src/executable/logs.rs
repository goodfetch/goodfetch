//! The executable's `tracing` subscriber (ADRs 0049, 0122 and 0132; RT-17,
//! RT-18): logs filtered by `GOODFETCH_LOG` in `EnvFilter`'s syntax, as text
//! on stderr or JSON lines on stdout under `daemon run`, in the format
//! `log.format` names, and as JSON lines in a file of the Goodfetch home's
//! log folder when `GOODFETCH_LOG_OUTPUT` asks for one; each Connection's
//! `$/log` events at its own level; the spans exported over OTLP/HTTP once
//! `otlp.traces_endpoint` names a collector, and never otherwise, their
//! trace and span IDs drawn from an `Entropy` of their own. Every
//! timestamp reads the `Clock`.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::sync::Arc;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::Home;
use goodfetch_engine::log::{ClockTime, LogEvents};
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::trace::EntropyIds;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::log::{LOG_FORMAT, OTLP_TRACES_ENDPOINT};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{WithExportConfig as _, WithHttpConfig as _};
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::Layer as _;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use super::otlp::{EXPORT_TIMEOUT, Ureq};
use super::{Command, DaemonCommand, LOG_FOLDER, LOG_OUTPUT_VARIABLE, Parsed};

/// Where `parsed`'s logs go: whether to their file by default, as a Daemon
/// a start launched does, which has no terminal (ADR 0122); and to stdout
/// as JSON lines, as the foreground Daemon does, whose stdout no protocol
/// uses (ADR 0049), or to stderr.
pub(super) fn logs_of(parsed: &Parsed) -> (bool, LogsTo) {
    match parsed {
        Parsed::HandWritten(Command::Daemon {
            command: DaemonCommand::Launched { .. },
            ..
        }) => (true, LogsTo::Stderr),
        Parsed::HandWritten(Command::Daemon {
            command: DaemonCommand::Run { .. },
            ..
        }) => (false, LogsTo::Stdout),
        _ => (false, LogsTo::Stderr),
    }
}

/// Where the logs go besides their file.
#[derive(Clone, Copy)]
pub(super) enum LogsTo {
    /// stderr: every command but `daemon run`, text unless `log.format`
    /// says `json`.
    Stderr,
    /// stdout: `daemon run` (ADR 0049), JSON lines unless `log.format`
    /// says `text`.
    Stdout,
}

/// The file of this process's logs in the home's log folder, when
/// `GOODFETCH_LOG_OUTPUT` is `file`, or it is unset and `by_default`; none
/// when it is `none` or unset.
pub(super) fn log_file(
    binary: &str,
    environment: &BTreeMap<OsString, OsString>,
    home: Option<&Home>,
    by_default: bool,
) -> Result<Option<File>, String> {
    let output = environment.get(OsStr::new(LOG_OUTPUT_VARIABLE));
    match output.map(|output| output.to_str()) {
        None if !by_default => Ok(None),
        Some(Some("none")) => Ok(None),
        None | Some(Some("file")) => {
            let home = home.ok_or("no Goodfetch home holds a log folder")?;
            let folder = home.root().join(LOG_FOLDER);
            #[expect(
                clippy::disallowed_methods,
                reason = "main opens the log file before the runtime starts (ADR 0122)"
            )]
            let file = std::fs::create_dir_all(&folder).and_then(|()| {
                File::options()
                    .create(true)
                    .append(true)
                    .open(folder.join(format!("{binary}-{}.jsonl", std::process::id())))
            });
            file.map(Some)
                .map_err(|error| format!("{}: {error}", folder.display()))
        }
        Some(_) => Err("it is `file` or `none`".to_owned()),
    }
}

/// What the subscriber is built from.
pub(super) struct Logs<'a> {
    /// `GOODFETCH_LOG`'s filter, `info` when unset or unreadable.
    pub(super) filter: Option<&'a OsString>,
    /// The log file, if any.
    pub(super) file: Option<File>,
    /// Where the logs go besides it.
    pub(super) to: LogsTo,
    /// The machine's settings, when they read.
    pub(super) settings: Option<&'a Resolved>,
    /// The log events the engine's Connections receive.
    pub(super) events: &'a LogEvents,
    /// The Product whose spans an export names.
    pub(super) product: Product,
    /// Where its time and its trace and span IDs come from.
    pub(super) handles: Handles,
}

/// The handles the subscriber reads (ADR 0059).
pub(super) struct Handles {
    /// Where every timestamp is read.
    pub(super) clock: Clock,
    /// Where an export draws its trace and span IDs: an `Entropy` of their
    /// own, none when the OS gave no seed, which ends the process anyway.
    pub(super) trace_ids: Option<Entropy>,
}

/// The installed subscriber's export, which sends the spans still waiting
/// when it is dropped.
pub(super) struct Installed {
    export: Option<SdkTracerProvider>,
}

impl Drop for Installed {
    fn drop(&mut self) {
        if let Some(export) = self.export.take() {
            let _ = export.shutdown();
        }
    }
}

/// Installs the process's `tracing` subscriber; an OTLP export that cannot
/// start is logged and left off.
pub(super) fn install(logs: Logs<'_>) -> Installed {
    let filter = || {
        logs.filter
            .and_then(|filter| filter.to_str())
            .and_then(|filter| tracing_subscriber::EnvFilter::try_new(filter).ok())
            .unwrap_or_else(|| tracing_subscriber::EnvFilter::new("info"))
    };
    let format = logs
        .settings
        .and_then(|settings| settings.get(LOG_FORMAT.name))
        .and_then(|format| format.value.as_str().map(str::to_owned));
    let json = match (logs.to, format.as_deref()) {
        (_, Some("json")) | (LogsTo::Stdout, None) => true,
        (_, Some(_)) | (LogsTo::Stderr, None) => false,
    };
    let Handles { clock, trace_ids } = logs.handles;
    let timer = ClockTime(clock);
    #[expect(
        clippy::disallowed_methods,
        reason = "the tracing subscriber's writer is one of the three writers of stderr and, in `daemon run`, of stdout (STYLE.md)"
    )]
    let writer = move || -> Box<dyn std::io::Write> {
        match logs.to {
            LogsTo::Stderr => Box::new(std::io::stderr()),
            LogsTo::Stdout => Box::new(std::io::stdout()),
        }
    };
    let (text, lines) = if json {
        (
            None,
            Some(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_timer(timer)
                    .with_span_events(FmtSpan::CLOSE)
                    .with_writer(writer)
                    .with_filter(filter()),
            ),
        )
    } else {
        (
            Some(
                tracing_subscriber::fmt::layer()
                    .with_timer(timer)
                    .with_writer(writer)
                    .with_ansi(false)
                    .with_filter(filter()),
            ),
            None,
        )
    };
    let file = logs.file.map(|file| {
        tracing_subscriber::fmt::layer()
            .json()
            .with_timer(timer)
            .with_span_events(FmtSpan::CLOSE)
            .with_writer(Arc::new(file))
            .with_filter(filter())
    });
    let endpoint = logs
        .settings
        .and_then(|settings| settings.get(OTLP_TRACES_ENDPOINT.name))
        .and_then(|endpoint| endpoint.value.as_str().map(str::to_owned));
    let (export, failed) = match endpoint.map(|endpoint| export(&endpoint, logs.product, trace_ids))
    {
        Some(Ok(export)) => (Some(export), None),
        Some(Err(error)) => (None, Some(error)),
        None => (None, None),
    };
    let spans = export.as_ref().map(|export| {
        tracing_opentelemetry::layer()
            .with_tracer(export.tracer("goodfetch"))
            // Span fields hold bounded values alone (ADR 0132): the target,
            // but no source path or thread name.
            .with_location(false)
            .with_threads(false)
            .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                metadata.is_span()
            }))
    });
    tracing_subscriber::registry()
        .with(text)
        .with(lines)
        .with(file)
        .with(logs.events.layer())
        .with(spans)
        .init();
    if let Some(error) = failed {
        tracing::error!(%error, "the spans' OTLP export cannot start, and is off");
    }
    Installed { export }
}

/// The export of every span to the OTLP/HTTP collector at `endpoint`, its
/// trace and span IDs drawn from `trace_ids` (ADR 0132).
fn export(
    endpoint: &str,
    product: Product,
    trace_ids: Option<Entropy>,
) -> Result<SdkTracerProvider, String> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_http_client(Ureq::new())
        .with_endpoint(endpoint)
        .with_timeout(EXPORT_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    let resource = opentelemetry_sdk::Resource::builder_empty()
        .with_service_name(format!("goodfetch-{}", product.as_str()))
        .build();
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource);
    Ok(match trace_ids {
        Some(entropy) => provider.with_id_generator(EntropyIds::new(entropy)),
        None => provider,
    }
    .build())
}
