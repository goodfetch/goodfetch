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
//! receives counts as `dropped`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::jsonrpc::{Notification, Version};
use goodfetch_protocol::log::{LOG, LogEvent, LogLevel};
use serde_json::{Map, Value};
use tokio::sync::{broadcast, mpsc, watch};
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

/// What the Connections' stream carries: the log events, in the order the
/// process emitted them, and each Connection's change of level among them,
/// so a Connection judges each event by the level it had when it was
/// emitted.
#[derive(Clone, Debug)]
enum Item {
    Event(Arc<LogEvent>),
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
        outgoing: mpsc::Sender<Vec<u8>>,
        work: CountedWork,
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
                forward(connection, events, level, outgoing, work).await;
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
    outgoing: mpsc::Sender<Vec<u8>>,
    work: CountedWork,
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
                Ok(Item::Event(event)) => {
                    if wanted == LogLevel::Off || event.level > wanted {
                        continue;
                    }
                    let event = LogEvent {
                        dropped: std::mem::take(&mut dropped),
                        ..LogEvent::clone(&event)
                    };
                    work.count(Row::MessageEncodes);
                    if outgoing.send(line(&event)).await.is_err() {
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

/// The `$/log` line of `event`.
fn line(event: &LogEvent) -> Vec<u8> {
    let mut line = serde_json::to_vec(&Notification {
        jsonrpc: Version,
        method: LOG.to_owned(),
        params: Some(serde_json::to_value(event).expect("invariant: a log event encodes")),
    })
    .expect("invariant: a notification of JSON values encodes");
    line.push(b'\n');
    line
}

/// The layer that makes each event a [`LogEvent`] for the Connections.
struct Forward {
    events: broadcast::Sender<Item>,
}

impl<S: Subscriber> Layer<S> for Forward {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        let mut fields = Fields::default();
        event.record(&mut fields);
        let _ = self.events.send(Item::Event(Arc::new(LogEvent {
            level: level_of(*metadata.level()),
            target: metadata.target().to_owned(),
            message: fields.message.unwrap_or_default(),
            fields: fields.fields,
            dropped: 0,
        })));
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

/// An event's message and its other fields, as JSON values.
#[derive(Default)]
struct Fields {
    message: Option<String>,
    fields: Map<String, Value>,
}

impl Fields {
    fn put(&mut self, field: &tracing::field::Field, value: Value) {
        if field.name() == "message" {
            self.message = Some(match value {
                Value::String(message) => message,
                other => other.to_string(),
            });
        } else {
            self.fields.insert(field.name().to_owned(), value);
        }
    }
}

impl tracing::field::Visit for Fields {
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.put(field, Value::Bool(value));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.put(field, Value::from(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.put(field, Value::from(value));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.put(field, Value::from(value));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.put(field, Value::String(value.to_owned()));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.put(field, Value::String(format!("{value:?}")));
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

#[cfg(test)]
mod tests;
