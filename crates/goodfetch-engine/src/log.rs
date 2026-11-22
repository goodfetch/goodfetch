//! The engine's side of its logs (ADRs 0059, 0078, 0122 and 0132; RT-17):
//! the log events each Connection receives as `$/log` notifications at the
//! level its `hello` or `connection.set_log_level` names, the `tracing`
//! timer that reads the `Clock`, and the origin a span records of a URL.
//!
//! [`LogEvents`] is one value the executable's `main`, or a test, makes:
//! its [`LogEvents::layer`] joins the process's `tracing` subscriber, and
//! the engine it is given ([`crate::Engine::with_log_events`]) hands each
//! Connection its own [`ConnectionLog`]. An engine given none, whose
//! Connections no layer could feed, keeps no buffer of them. The layer
//! listens only at the most verbose level a Connection asks for, so with
//! none listening it costs nothing. Log events are telemetry: the layer never waits, and a
//! Connection too slow to read them loses the oldest, which the next one it
//! receives counts as `dropped`. Each event's `$/log` line is encoded once,
//! as it is emitted, its message and fields written straight into it, and
//! each Connection at its level sends a copy (ADR 0140's E1).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use goodfetch_protocol::log::{LOG, LogLevel};
use serde::ser::{SerializeMap, SerializeStruct as _};
use serde::{Serialize, Serializer};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, watch};
use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::subscriber::Interest;
use tracing::{Metadata, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::layer::{Context, Filter};

use crate::clock::Clock;

/// How many log events wait for the slowest Connection before it loses the
/// oldest.
const BUFFERED: usize = 1024;

/// The log events of one process, which its Connections receive.
#[derive(Clone, Debug)]
pub struct LogEvents {
    levels: Arc<watch::Sender<Levels>>,
    events: broadcast::Sender<Item>,
}

/// The name of the field that holds an event's message.
const MESSAGE: &str = "message";

/// What a `$/log` line's buffer starts with room for: a line with a short
/// message and a few fields fits it whole.
const LINE_CAPACITY: usize = 256;

/// What the Connections' stream carries: the log events, each at its level
/// and as its `$/log` line, in the order the process emitted them, and each
/// Connection's change of level among them, so a Connection judges each
/// event by the level it had when it was emitted.
#[derive(Clone, Debug)]
enum Item {
    Event { level: LogLevel, line: Vec<u8> },
    Level { connection: u64, level: LogLevel },
}

/// Each Connection's level, and the most verbose of them.
#[derive(Debug, Default)]
struct Levels {
    of: BTreeMap<u64, LogLevel>,
    next: u64,
    most: LogLevel,
}

impl Levels {
    fn settle(&mut self) -> bool {
        let most = self.of.values().copied().max().unwrap_or_default();
        let changed = most != self.most;
        self.most = most;
        changed
    }
}

impl Default for LogEvents {
    fn default() -> Self {
        Self::new()
    }
}

impl LogEvents {
    /// Log events no Connection receives yet.
    pub fn new() -> Self {
        Self {
            levels: Arc::new(watch::Sender::new(Levels::default())),
            events: broadcast::channel(BUFFERED).0,
        }
    }

    /// The layer that hands the subscriber's events to the Connections, at
    /// the level each asks for.
    pub fn layer<S>(&self) -> impl Layer<S> + Send + Sync + 'static
    where
        S: Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
    {
        Forward {
            events: self.events.clone(),
        }
        .with_filter(MostVerbose {
            levels: Arc::clone(&self.levels),
        })
    }

    /// A new Connection's log, which receives nothing until its level is
    /// set.
    pub(crate) fn connection(&self) -> ConnectionLog {
        let mut id = 0;
        self.levels.send_modify(|levels| {
            id = levels.next;
            levels.next += 1;
            levels.of.insert(id, LogLevel::Off);
        });
        ConnectionLog(Some(Listening {
            id,
            events: self.clone(),
            level: watch::Sender::new(LogLevel::Off),
        }))
    }

    fn set(&self, id: u64, level: Option<LogLevel>) {
        let mut changed = false;
        self.levels.send_modify(|levels| {
            match level {
                Some(level) => levels.of.insert(id, level),
                None => levels.of.remove(&id),
            };
            changed = levels.settle();
        });
        // Each callsite's interest is cached: asked again, the layer
        // listens where it now must, and nowhere else.
        if changed {
            tracing::callsite::rebuild_interest_cache();
        }
    }
}

/// One Connection's log events: its level, and what sends them; nothing on
/// an engine given no [`LogEvents`], whose Connections receive none.
#[derive(Debug)]
pub(crate) struct ConnectionLog(Option<Listening>);

/// The log of a Connection to an engine given [`LogEvents`].
#[derive(Debug)]
struct Listening {
    id: u64,
    events: LogEvents,
    level: watch::Sender<LogLevel>,
}

impl ConnectionLog {
    /// The log of a Connection to an engine given no [`LogEvents`]: no
    /// layer emits any, so it receives none at any level.
    pub(crate) fn none() -> Self {
        Self(None)
    }

    /// Sets the level the Connection receives from the next log event on.
    pub(crate) fn set(&self, level: LogLevel) {
        let Some(listening) = &self.0 else {
            return;
        };
        listening.level.send_replace(level);
        listening.events.set(listening.id, Some(level));
        let _ = listening.events.events.send(Item::Level {
            connection: listening.id,
            level,
        });
    }

    /// Sends the Connection its log events as `$/log` lines on `outgoing`,
    /// until this log is dropped; ends at once when it has none.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the log events it had not sent.
    pub(crate) fn forward(
        &self,
        outgoing: mpsc::Sender<crate::wire::Line>,
    ) -> impl Future<Output = ()> + Send + 'static {
        let receivers = self.0.as_ref().map(|listening| {
            (
                listening.id,
                listening.events.events.subscribe(),
                listening.level.subscribe(),
            )
        });
        async move {
            if let Some((connection, events, level)) = receivers {
                forward(connection, events, level, outgoing).await;
            }
        }
    }
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.events.set(self.id, None);
    }
}

/// Sends Connection `connection` the log events of `events` at its level,
/// which the stream's own changes of it set, and `level` once the stream
/// lost them, until `level`'s sender is dropped.
async fn forward(
    connection: u64,
    mut events: broadcast::Receiver<Item>,
    mut level: watch::Receiver<LogLevel>,
    outgoing: mpsc::Sender<crate::wire::Line>,
) {
    let mut dropped: u64 = 0;
    let mut wanted = *level.borrow_and_update();
    loop {
        tokio::select! {
            biased;
            received = events.recv() => match received {
                Ok(Item::Level { connection: changed, level }) => {
                    if changed == connection {
                        wanted = level;
                    }
                }
                Ok(Item::Event { level: emitted, line }) => {
                    if wanted == LogLevel::Off || emitted > wanted {
                        continue;
                    }
                    let line = match std::mem::take(&mut dropped) {
                        0 => line,
                        dropped => telling_dropped(line, dropped),
                    };
                    if outgoing.send(line.into()).await.is_err() {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(lost)) => {
                    dropped += lost;
                    wanted = *level.borrow();
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            changed = level.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
    }
}

/// The `$/log` line of `event` at `level`, encoded once, its message and
/// fields written straight into it: the [`goodfetch_protocol::log::LogEvent`]
/// its Connections receive, its fields in the order the event records them,
/// or the error of a value whose `Debug` fails.
fn encode(level: LogLevel, event: &tracing::Event<'_>) -> Result<Vec<u8>, serde_json::Error> {
    crate::wire::try_notification_line(LOG, &Params { level, event }, LINE_CAPACITY)
}

/// `line`, a `$/log` line as [`encode`] writes it, telling the `dropped` log
/// events lost before it: `dropped` is the last field of
/// [`goodfetch_protocol::log::LogEvent`], so it goes after the params'
/// others, before their closing brace and the message's.
fn telling_dropped(mut line: Vec<u8>, dropped: u64) -> Vec<u8> {
    const END: &[u8] = b"}}\n";
    debug_assert!(
        line.ends_with(END),
        "a `$/log` line ends its params and itself"
    );
    line.truncate(line.len().saturating_sub(END.len()));
    line.extend_from_slice(b",\"dropped\":");
    line.extend_from_slice(dropped.to_string().as_bytes());
    line.extend_from_slice(END);
    line
}

/// An event's `$/log` params as its [`goodfetch_protocol::log::LogEvent`]
/// has them, with no `dropped`.
struct Params<'a, 'e> {
    level: LogLevel,
    event: &'a tracing::Event<'e>,
}

impl Serialize for Params<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut others = Others(false);
        self.event.record(&mut others);
        let mut params = serializer.serialize_struct("LogEvent", 3 + usize::from(others.0))?;
        params.serialize_field("level", &self.level)?;
        params.serialize_field("target", self.event.metadata().target())?;
        params.serialize_field("message", &MessageOf(self.event))?;
        if others.0 {
            params.serialize_field("fields", &FieldsOf(self.event))?;
        }
        params.end()
    }
}

/// Whether an event records a field other than its message.
struct Others(bool);

impl Visit for Others {
    fn record_debug(&mut self, field: &Field, _value: &dyn fmt::Debug) {
        self.0 |= field.name() != MESSAGE;
    }
}

/// An event's message: the text of its first `message` field, empty when it
/// records none.
struct MessageOf<'a, 'e>(&'a tracing::Event<'e>);

impl Serialize for MessageOf<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut message = WriteMessage {
            serializer: Some(serializer),
            written: None,
        };
        self.0.record(&mut message);
        match message.written {
            Some(written) => written,
            None => message
                .serializer
                .expect("invariant: the serializer is taken only to write the message")
                .serialize_str(""),
        }
    }
}

/// Writes the first `message` field it visits as a JSON string.
struct WriteMessage<S: Serializer> {
    serializer: Option<S>,
    written: Option<Result<S::Ok, S::Error>>,
}

impl<S: Serializer> WriteMessage<S> {
    fn write(&mut self, field: &Field, write: impl FnOnce(S) -> Result<S::Ok, S::Error>) {
        if field.name() != MESSAGE {
            return;
        }
        if let Some(serializer) = self.serializer.take() {
            self.written = Some(write(serializer));
        }
    }
}

impl<S: Serializer> Visit for WriteMessage<S> {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.write(field, |serializer| {
            serializer.collect_str(&Value::Bool(value))
        });
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.write(field, |serializer| {
            serializer.collect_str(&Value::from(value))
        });
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.write(field, |serializer| {
            serializer.collect_str(&Value::from(value))
        });
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.write(field, |serializer| {
            serializer.collect_str(&Value::from(value))
        });
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.write(field, |serializer| serializer.serialize_str(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.write(field, |serializer| serializer.collect_str(&Shown(value)));
    }
}

/// An event's fields other than its message, as a JSON object.
struct FieldsOf<'a, 'e>(&'a tracing::Event<'e>);

impl Serialize for FieldsOf<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut fields = WriteFields {
            map: serializer.serialize_map(None)?,
            failed: None,
        };
        self.0.record(&mut fields);
        match fields.failed {
            Some(error) => Err(error),
            None => fields.map.end(),
        }
    }
}

/// Writes each field other than the message it visits as an entry, keeping
/// the first error.
struct WriteFields<M: SerializeMap> {
    map: M,
    failed: Option<M::Error>,
}

impl<M: SerializeMap> WriteFields<M> {
    fn write(&mut self, field: &Field, value: &(impl Serialize + ?Sized)) {
        if field.name() == MESSAGE || self.failed.is_some() {
            return;
        }
        if let Err(error) = self.map.serialize_entry(field.name(), value) {
            self.failed = Some(error);
        }
    }
}

impl<M: SerializeMap> Visit for WriteFields<M> {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.write(field, &value);
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.write(field, &value);
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.write(field, &value);
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.write(field, &Value::from(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.write(field, value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.write(field, &AsString(value));
    }
}

/// A value's `Debug`, shown as text.
struct Shown<'a>(&'a dyn fmt::Debug);

impl fmt::Display for Shown<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.0, formatter)
    }
}

/// A value's `Debug`, as a JSON string.
struct AsString<'a>(&'a dyn fmt::Debug);

impl Serialize for AsString<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&Shown(self.0))
    }
}

/// The layer that encodes each event's `$/log` line for the Connections.
struct Forward {
    events: broadcast::Sender<Item>,
}

impl<S: Subscriber> Layer<S> for Forward {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        let level = level_of(*event.metadata().level());
        // An event one of whose values fails to format is lost, as one no
        // Connection had room for.
        if let Ok(line) = encode(level, event) {
            let _ = self.events.send(Item::Event { level, line });
        }
    }
}

/// The filter that lets through only the events some Connection asks for.
struct MostVerbose {
    levels: Arc<watch::Sender<Levels>>,
}

impl MostVerbose {
    fn most(&self) -> LevelFilter {
        level_filter(self.levels.borrow().most)
    }
}

impl<S> Filter<S> for MostVerbose {
    fn enabled(&self, metadata: &Metadata<'_>, _context: &Context<'_, S>) -> bool {
        metadata.is_event() && self.most() >= *metadata.level()
    }

    fn callsite_enabled(&self, metadata: &'static Metadata<'static>) -> Interest {
        if metadata.is_event() && self.most() >= *metadata.level() {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(self.most())
    }
}

fn level_of(level: tracing::Level) -> LogLevel {
    match level {
        tracing::Level::ERROR => LogLevel::Error,
        tracing::Level::WARN => LogLevel::Warn,
        tracing::Level::INFO => LogLevel::Info,
        tracing::Level::DEBUG => LogLevel::Debug,
        tracing::Level::TRACE => LogLevel::Trace,
    }
}

fn level_filter(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Off => LevelFilter::OFF,
        LogLevel::Error => LevelFilter::ERROR,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Trace => LevelFilter::TRACE,
    }
}

/// The `tracing` timer that reads the `Clock`: RFC 3339 in UTC, so an
/// in-process test's traces carry its simulated time (ADR 0059).
#[derive(Clone, Copy, Debug)]
pub struct ClockTime(pub Clock);

impl FormatTime for ClockTime {
    fn format_time(&self, writer: &mut Writer<'_>) -> fmt::Result {
        write!(writer, "{}", self.0.timestamp())
    }
}

/// What a span or a log line records of `url`: its origin, the scheme, host
/// and port, never its user, password, path, query or fragment (ADR 0132);
/// `null` for a URL with no such origin, or none at all.
pub fn origin(url: &str) -> String {
    url::Url::parse(url).map_or_else(
        |_| "null".to_owned(),
        |url| url.origin().ascii_serialization(),
    )
}

/// The targets under which a WebSocket crate logs each frame's payload at
/// `trace`, through the `log` crate: an Exposed Daemon's `hello` and its
/// token, and a Borrowed browser's CDP traffic at its `cdp_url`.
pub const FRAME_PAYLOAD_TARGETS: [&str; 2] = ["tungstenite", "tokio_tungstenite"];

/// The process subscriber's global filter that keeps every WebSocket
/// frame's payload out of its logs, since secrets never reach logs (ADR
/// 0067): it caps [`FRAME_PAYLOAD_TARGETS`] at `debug` and lets every other
/// target through, at any level.
///
/// Joined to the subscriber before its layers, outside their own filters,
/// it decides first for every record, a `log` record a bridge hands to
/// `tracing` under its own target as much as a `tracing` event: no
/// `log.level`, `RUST_LOG` directive or later bridge lifts it.
pub fn frame_payloads_unlogged() -> tracing_subscriber::filter::Targets {
    FRAME_PAYLOAD_TARGETS.into_iter().fold(
        tracing_subscriber::filter::Targets::new().with_default(LevelFilter::TRACE),
        |targets, target| targets.with_target(target, LevelFilter::DEBUG),
    )
}

#[cfg(test)]
mod tests;
