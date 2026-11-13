//! The executable's `tracing` subscriber (ADRs 0049, 0122 and 0132; RT-17,
//! RT-18), from the machine's `log.*` settings: logs filtered by
//! `log.level`, `info` or a `tracing` filter directive, as text on stderr
//! or JSON lines on stdout under `daemon run`, in the format `log.format`
//! names, and, where `log.output` is `file`, as JSON lines in the Goodfetch
//! home's log folder, rotated at `log.file.max_size` and kept to
//! `log.file.max_files` (the `log_file` module): one set of files per
//! Daemon, one per process for any other; each Connection's `$/log` events
//! at its own level; the spans exported over OTLP/HTTP once
//! `otlp.traces_endpoint` names a collector, and never otherwise, their
//! trace and span IDs drawn from an `Entropy` of their own. Every
//! timestamp reads the `Clock`.
//!
//! `log.output` defaults to `file` for a Daemon a start launched, which has
//! no terminal, to `stdout` under `daemon run`, whose JSON lines on stdout
//! its platform collects and rotates, and to `none` for any other process,
//! an Attached engine's logs going to its Client as log events. `stdout` is
//! `daemon run`'s alone: anywhere else stdout is the protocol or the CLI's
//! output, and it fails with `setting_not_applicable`.

use std::cell::Cell;
use std::io::Write;

use goodfetch_engine::audit::AuditEvents;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::Home;
use goodfetch_engine::log::{ClockTime, LogEvents};
use goodfetch_engine::mask::{Masks, Scope};
use goodfetch_engine::settings::{Resolved, get_or_default};
use goodfetch_engine::trace::EntropyIds;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::log::{
    LOG_FILE_MAX_FILES, LOG_FILE_MAX_SIZE, LOG_FORMAT, LOG_LEVEL, LOG_OUTPUT, OTLP_TRACES_ENDPOINT,
};
use goodfetch_protocol::settings::{SETTING_INVALID, SETTING_NOT_APPLICABLE};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{WithExportConfig as _, WithHttpConfig as _};
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::Layer as _;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use super::audit_events;
use super::log_file::{LogFile, Rotation};
use super::otlp::{EXPORT_TIMEOUT, ToCollector};
use super::{Command, DaemonCommand, Parsed};

/// The kinds of process whose logs ADR 0122 tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Process {
    /// `daemon run`, the foreground Daemon, whose stdout no protocol uses
    /// (ADR 0049).
    DaemonRun,
    /// A Daemon a start or the installed service launched, which has no
    /// terminal.
    DaemonLaunched,
    /// Any other command, an Attached engine included.
    Other,
}

impl Process {
    /// The kind of `parsed`'s process.
    pub(super) fn of(parsed: &Parsed) -> Self {
        match parsed {
            Parsed::HandWritten(Command::Daemon {
                command: DaemonCommand::Launched { .. },
                ..
            }) => Self::DaemonLaunched,
            Parsed::HandWritten(Command::Daemon {
                command: DaemonCommand::Run { .. },
                ..
            }) => Self::DaemonRun,
            _ => Self::Other,
        }
    }

    /// Where its logs go beside their file.
    pub(super) fn to(self) -> LogsTo {
        match self {
            Self::DaemonRun => LogsTo::Stdout,
            Self::DaemonLaunched | Self::Other => LogsTo::Stderr,
        }
    }

    /// Its `log.output` when unset.
    fn default_output(self) -> &'static str {
        match self {
            Self::DaemonRun => "stdout",
            Self::DaemonLaunched => "file",
            Self::Other => "none",
        }
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

/// Why the logs cannot go where the settings say, as the Error code and
/// message the process stops with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Refused {
    /// The Error code.
    pub(super) code: &'static str,
    /// What is wrong.
    pub(super) message: String,
}

/// The `tracing` filter `log.level` sets, its catalog default, `info`,
/// without settings; a directive `tracing` cannot read is refused with
/// `setting_invalid`.
pub(super) fn filter_of(settings: Option<&Resolved>) -> Result<String, Refused> {
    let level = get_or_default::<&str>(settings, &LOG_LEVEL).to_owned();
    match tracing_subscriber::EnvFilter::try_new(&level) {
        Ok(_) => Ok(level),
        Err(error) => Err(Refused {
            code: SETTING_INVALID.name,
            message: format!("`log.level` is `{level}`, which is no `tracing` filter: {error}"),
        }),
    }
}

/// The log file of `binary`'s `process` of `version` in `home`'s log
/// folder when `log.output` is `file` (one set per Daemon, one per process
/// otherwise), rotated as `log.file.*` say; none when it is `none`, or
/// `stdout` under `daemon run`.
///
/// # Errors
///
/// `setting_not_applicable` for `stdout` outside `daemon run`, whose
/// stdout is the protocol or the CLI's output, and when the file cannot be
/// opened, or no home holds the folder.
pub(super) fn file_of(
    binary: &str,
    process: Process,
    version: &ProductVersion,
    home: Option<&Home>,
    settings: Option<&Resolved>,
    clock: Clock,
) -> Result<Option<LogFile>, Refused> {
    let output = settings
        .and_then(|settings| settings.get::<Option<&str>>(&LOG_OUTPUT))
        .unwrap_or_else(|| process.default_output())
        .to_owned();
    match (output.as_str(), process) {
        ("file", _) => {}
        ("none", _) | ("stdout", Process::DaemonRun) => return Ok(None),
        ("stdout", _) => {
            return Err(Refused {
                code: SETTING_NOT_APPLICABLE.name,
                message: "`log.output` is `stdout`, which only `daemon run` writes: this process's stdout is the protocol or the CLI's output; set `file` or `none`".to_owned(),
            });
        }
        (other, _) => {
            return Err(Refused {
                code: SETTING_INVALID.name,
                message: format!("`log.output` is `{other}`: it is `stdout`, `file` or `none`"),
            });
        }
    }
    let rotation = Rotation {
        max_size: get_or_default(settings, &LOG_FILE_MAX_SIZE),
        max_files: get_or_default(settings, &LOG_FILE_MAX_FILES),
    };
    let home = home.ok_or_else(|| Refused {
        code: SETTING_NOT_APPLICABLE.name,
        message: "`log.output` is `file`, and no Goodfetch home holds a log folder".to_owned(),
    })?;
    let folder = home.logs();
    match process {
        Process::DaemonRun | Process::DaemonLaunched => LogFile::open(
            &folder,
            &format!("daemon-{}", version.line()),
            rotation,
            clock,
        ),
        Process::Other => {
            LogFile::open_of_process(&folder, binary, std::process::id(), rotation, clock)
        }
    }
    .map(Some)
    .map_err(|error| Refused {
        code: SETTING_NOT_APPLICABLE.name,
        message: format!(
            "the log file cannot be opened in `{}`: {error}",
            folder.display()
        ),
    })
}

/// A subscriber's writer whose every writer passes its lines through the
/// engine's masks as a kept output (spec #564): no value a Run resolved
/// reaches a log, unless its Run's `security.redaction` is off.
pub(super) struct Masked<M> {
    masks: Masks,
    inner: M,
}

impl<M> Masked<M> {
    /// `inner`'s writers, each masking with `masks`.
    pub(super) fn new(masks: Masks, inner: M) -> Self {
        Self { masks, inner }
    }
}

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for Masked<M> {
    type Writer = MaskedLine<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        MaskedLine {
            masks: self.masks.clone(),
            inner: self.inner.make_writer(),
            buffers: LINE_BUFFERS.try_with(Cell::take).unwrap_or_default(),
        }
    }
}

/// The largest buffer a thread keeps from one line to the next: a log line
/// is mostly under a kilobyte, and a thread that once logged a huge one
/// does not hold its memory for good.
const KEPT_CAPACITY: usize = 64 * 1024;

/// An event's line as written and as masked, in buffers a thread reuses
/// from line to line.
#[derive(Default)]
struct LineBuffers {
    held: Vec<u8>,
    masked: Vec<u8>,
}

thread_local! {
    /// This thread's [`LineBuffers`], which each [`MaskedLine`] it makes
    /// borrows and gives back, so logging line after line takes no new
    /// buffer for each; a writer made while another on the thread holds
    /// them starts with empty ones.
    static LINE_BUFFERS: Cell<LineBuffers> = const {
        Cell::new(LineBuffers {
            held: Vec::new(),
            masked: Vec::new(),
        })
    };
}

/// One `tracing` event's line on its way to its writer: held until the
/// event is written, then masked whole and handed over in one write. The
/// log file takes each write as a line of its own, and a line handed over
/// in pieces, as a streaming mask writes around each value it replaces,
/// would interleave with the lines other threads log at the same time.
pub(super) struct MaskedLine<W: Write> {
    masks: Masks,
    inner: W,
    buffers: LineBuffers,
}

impl<W: Write> MaskedLine<W> {
    /// Hands the line held so far, masked, to `inner` in one write.
    fn hand_over(&mut self) -> std::io::Result<()> {
        let LineBuffers { held, masked } = &mut self.buffers;
        if held.is_empty() {
            return Ok(());
        }
        masked.clear();
        self.masks.apply_bytes_into(held, Scope::KeptOutput, masked);
        held.clear();
        self.inner.write_all(masked)
    }
}

impl<W: Write> Write for MaskedLine<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.buffers.held.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.hand_over()?;
        self.inner.flush()
    }
}

impl<W: Write> Drop for MaskedLine<W> {
    fn drop(&mut self) {
        let _ = self.hand_over();
        let mut buffers = std::mem::take(&mut self.buffers);
        if buffers.held.capacity() <= KEPT_CAPACITY && buffers.masked.capacity() <= KEPT_CAPACITY {
            buffers.masked.clear();
            let _ = LINE_BUFFERS.try_with(|kept| kept.set(buffers));
        }
    }
}

/// What the subscriber is built from.
pub(super) struct Logs<'a> {
    /// `log.level`'s filter.
    pub(super) filter: String,
    /// The log file, if any.
    pub(super) file: Option<LogFile>,
    /// Where the logs go besides it.
    pub(super) to: LogsTo,
    /// The machine's settings, when they read.
    pub(super) settings: Option<&'a Resolved>,
    /// The log events the engine's Connections receive.
    pub(super) events: &'a LogEvents,
    /// Every live Run's resolved values, which each writer masks.
    pub(super) masks: &'a Masks,
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
/// when it is dropped, its log file, whose waiting lines are written then,
/// and the Audit log's OpenTelemetry events, also sent then.
pub(super) struct Installed {
    export: Option<SdkTracerProvider>,
    file: Option<LogFile>,
    audit: Option<audit_events::Export>,
}

impl Installed {
    /// What the engine sends each new Audit log entry to, when
    /// `audit.otlp` sends them anywhere.
    pub(super) fn audit_events(&self) -> Option<AuditEvents> {
        self.audit.as_ref().map(audit_events::Export::events)
    }
}

impl Drop for Installed {
    fn drop(&mut self) {
        if let Some(export) = self.export.take() {
            let _ = export.shutdown();
        }
        if let Some(file) = &self.file {
            file.flush();
        }
    }
}

/// Installs the process's `tracing` subscriber; an OTLP export that cannot
/// start is logged and left off.
pub(super) fn install(logs: Logs<'_>) -> Installed {
    let filter = || {
        tracing_subscriber::EnvFilter::try_new(&logs.filter)
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
    };
    let format = logs
        .settings
        .and_then(|settings| settings.effective(&LOG_FORMAT))
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
                    .with_writer(Masked::new(logs.masks.clone(), writer))
                    .with_filter(filter()),
            ),
        )
    } else {
        (
            Some(
                tracing_subscriber::fmt::layer()
                    .with_timer(timer)
                    .with_writer(Masked::new(logs.masks.clone(), writer))
                    .with_ansi(false)
                    .with_filter(filter()),
            ),
            None,
        )
    };
    let kept = logs.file.clone();
    let file = logs.file.map(|file| {
        tracing_subscriber::fmt::layer()
            .json()
            .with_timer(timer)
            .with_span_events(FmtSpan::CLOSE)
            .with_writer(Masked::new(logs.masks.clone(), file))
            .with_filter(filter())
    });
    let endpoint = logs.settings.and_then(|settings| {
        let endpoint = settings.effective(&OTLP_TRACES_ENDPOINT)?.value.as_str()?;
        Some((endpoint.to_owned(), settings))
    });
    let exported = endpoint.map(|(endpoint, settings)| {
        export(
            &endpoint,
            ToCollector::new(settings, clock),
            logs.product,
            trace_ids,
        )
    });
    let (export, failed) = match exported {
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
    let audit = audit_events::export(logs.settings, clock, logs.product);
    Installed {
        export,
        file: kept,
        audit,
    }
}

/// The export of every span to the OTLP/HTTP collector at `endpoint`
/// through `client`, its trace and span IDs drawn from `trace_ids` (ADR
/// 0132).
fn export(
    endpoint: &str,
    client: ToCollector,
    product: Product,
    trace_ids: Option<Entropy>,
) -> Result<SdkTracerProvider, String> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_http_client(client)
        .with_endpoint(endpoint)
        .with_timeout(EXPORT_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    let resource = opentelemetry_sdk::Resource::builder_empty()
        .with_service_name(product.executable_name())
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
