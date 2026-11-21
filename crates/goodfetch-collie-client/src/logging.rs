//! The crate's logs and the engine's (ADR 0078): the SDK emits `tracing`
//! events under the `goodfetch::…` targets this module names and installs
//! no subscriber, the program's own subscriber decides what is recorded.
//! The level an engine sends its logs at is the highest one that
//! subscriber records for [`ENGINE`], which [`engine_log_level`] reads,
//! and each `$/log` event an engine sends is re-emitted at its level under
//! [`ENGINE`] followed by the engine's own target past `goodfetch`, so the
//! subscriber filters the engine's areas by target as it filters the
//! program's. A linked engine sends no `$/log`: its events reach the
//! program's subscriber directly, under their own targets.

use std::collections::BTreeMap;
use std::sync::{OnceLock, PoisonError};

use goodfetch_protocol::log::{LogEvent, LogLevel};
use serde_json::Value;
use tokio::runtime::Handle;
use tokio_util::task::TaskTracker;
use tracing::instrument::WithSubscriber as _;
use tracing::level_filters::{LevelFilter, STATIC_MAX_LEVEL};
use tracing_core::callsite::{self, Callsite, Identifier};
use tracing_core::field::{FieldSet, Value as FieldValue, display};
use tracing_core::metadata::Kind;
use tracing_core::subscriber::Interest;
use tracing_core::{Dispatch, Event, Level, Metadata, dispatcher};

use crate::sync::Mutex;

/// The target of what the crate itself logs: a refused log level, a
/// handle dropped unclosed, a notification no call awaits.
pub const CLIENT: &str = "goodfetch::client";

/// The target of what the crate logs of a Run's start: one that resumed
/// its Run, or found it ended (ADR 0102).
pub const RUN: &str = "goodfetch::client::run";

/// The target the engine's `$/log` events are re-emitted under, each
/// followed by the engine's own target past `goodfetch`, such as
/// `goodfetch::engine::rpc::dispatch` for `goodfetch::rpc::dispatch`; the
/// level the program's subscriber records for it is the one the engine
/// is told.
pub const ENGINE: &str = "goodfetch::engine";

/// How many targets, by level, the engine's events are re-emitted under
/// their own name: each is registered with `tracing` once and kept for
/// the process, so an engine that sent ever new targets would grow it
/// without end; past this many, an event is re-emitted under [`ENGINE`]
/// alone, its own target still in its `target` field.
const TARGETS_KEPT: usize = 512;

/// The fields of a re-emitted event: its message, the engine's own
/// target, its fields as one JSON object when it has any, and how many
/// events the engine dropped before it when it dropped some.
static FIELDS: [&str; 4] = ["message", "target", "fields", "dropped"];

/// The highest level the program's subscriber records for [`ENGINE`] on
/// the calling thread, which the engine is told to send its logs at:
/// `off` when it records none of them, or when there is no subscriber.
///
/// # Examples
///
/// ```
/// use goodfetch_collie_client::logging::engine_log_level;
/// use goodfetch_protocol::log::LogLevel;
///
/// // With no subscriber, nothing is recorded, so the engine sends nothing.
/// assert_eq!(engine_log_level(), LogLevel::Off);
/// ```
#[must_use]
pub fn engine_log_level() -> LogLevel {
    static PROBES: OnceLock<Vec<(LogLevel, &'static Metadata<'static>)>> = OnceLock::new();
    let probes = PROBES.get_or_init(|| {
        [
            LogLevel::Trace,
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
        ]
        .into_iter()
        .filter_map(|told| {
            let level = level_of(told)?;
            (level <= STATIC_MAX_LEVEL).then(|| (told, relayed("", level).kept()))
        })
        .collect()
    });
    dispatcher::get_default(|dispatch| {
        probes
            .iter()
            .find(|&&(_, metadata)| records(dispatch, metadata))
            .map_or(LogLevel::Off, |&(told, _)| told)
    })
}

/// Whether `dispatch` records events of `metadata`: what its own interest
/// in the callsite says, and when that depends on each event, what it says
/// of one. Neither `tracing::enabled!`, whose callsite keeps the interest
/// every live subscriber settled together, nor [`Dispatch::enabled`] alone,
/// which a subscriber with per-layer filters answers `true` and filters at
/// the event, says it of this subscriber.
fn records(dispatch: &Dispatch, metadata: &'static Metadata<'static>) -> bool {
    let interest = dispatch.register_callsite(metadata);
    interest.is_always() || (interest.is_sometimes() && dispatch.enabled(metadata))
}

/// Spawns `task` on `runtime`, tracked by `tracker`, logging to the
/// subscriber of the program that opened the Connection, as the engine's
/// `$/log` events and an Attached engine's stderr do; with none set then,
/// to the global one, which a program may install later.
pub(crate) fn spawn_under_opener(
    task: impl Future<Output = ()> + Send + 'static,
    runtime: &Handle,
    tracker: &TaskTracker,
) {
    let opened_under = tracing::dispatcher::get_default(|dispatch| {
        (!dispatch.is::<tracing::subscriber::NoSubscriber>()).then(|| dispatch.clone())
    });
    match opened_under {
        Some(dispatch) => tracker.spawn_on(task.with_subscriber(dispatch), runtime),
        None => tracker.spawn_on(task, runtime),
    };
}

/// Re-emits the `$/log` event `params` hold to the subscriber of the
/// calling task, at its level, under [`ENGINE`] followed by its target,
/// when that subscriber records it.
/// Params that are not a log event are logged under [`CLIENT`] and
/// dropped.
pub(crate) fn relay(params: Option<Value>) {
    let event = match serde_json::from_value::<LogEvent>(params.unwrap_or(Value::Null)) {
        Ok(event) => event,
        Err(error) => {
            tracing::debug!(target: CLIENT, %error, "a `$/log` notification that holds no log event");
            return;
        }
    };
    let Some(level) = level_of(event.level) else {
        return;
    };
    if level > STATIC_MAX_LEVEL || level > LevelFilter::current() {
        return;
    }
    let metadata = relayed(&event.target, level).metadata();
    let names = metadata.fields();
    let [Some(message), Some(target), Some(fields), Some(dropped)] =
        FIELDS.map(|name| names.field(name))
    else {
        return;
    };
    let fields_shown = (!event.fields.is_empty()).then(|| display(Value::Object(event.fields)));
    let dropped_shown = (event.dropped > 0).then_some(event.dropped);
    let message_shown = display(event.message.as_str());
    let values: [(&_, Option<&dyn FieldValue>); 4] = [
        (&message, Some(&message_shown)),
        (&target, Some(&event.target.as_str())),
        (
            &fields,
            fields_shown.as_ref().map(|shown| shown as &dyn FieldValue),
        ),
        (
            &dropped,
            dropped_shown.as_ref().map(|shown| shown as &dyn FieldValue),
        ),
    ];
    let values = names.value_set(&values);
    dispatcher::get_default(|dispatch| {
        if dispatch.enabled(metadata) {
            dispatch.event(&Event::new(metadata, &values));
        }
    });
}

/// The `tracing` level of an engine's log level; `off` has none.
fn level_of(level: LogLevel) -> Option<Level> {
    match level {
        LogLevel::Off => None,
        LogLevel::Error => Some(Level::ERROR),
        LogLevel::Warn => Some(Level::WARN),
        LogLevel::Info => Some(Level::INFO),
        LogLevel::Debug => Some(Level::DEBUG),
        LogLevel::Trace => Some(Level::TRACE),
    }
}

/// The target an engine event of `target` is re-emitted under: [`ENGINE`]
/// followed by `target`'s segments, less a leading `goodfetch`, or
/// [`ENGINE`] alone when `target` is empty or a segment is not an
/// identifier.
pub(crate) fn engine_target(target: &str) -> String {
    let segments: Vec<&str> = target.split("::").collect();
    let own = match segments.split_first() {
        Some((&"goodfetch", rest)) => rest,
        _ => &segments[..],
    };
    if target.is_empty() || !own.iter().all(|segment| is_identifier(segment)) {
        return ENGINE.to_owned();
    }
    own.iter().fold(ENGINE.to_owned(), |mut path, segment| {
        path.push_str("::");
        path.push_str(segment);
        path
    })
}

/// Whether `segment` is a Rust identifier: a letter or `_`, then letters,
/// digits and `_`.
fn is_identifier(segment: &str) -> bool {
    let mut chars = segment.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|next| next.is_ascii_alphanumeric() || next == '_')
}

/// The callsite of the engine's events of the engine's own `target` at
/// `level`, re-emitted under [`engine_target`]'s, registered with `tracing`
/// the first time and kept for the process: `tracing`'s macros need a
/// target known when the program is compiled, an engine's is known only
/// when its event arrives. A kept callsite is found without allocating.
fn relayed(target: &str, level: Level) -> &'static Relayed {
    static KEPT: Mutex<BTreeMap<String, Vec<(Level, &'static Relayed)>>> =
        Mutex::new(BTreeMap::new());
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    let key = if kept.len() < TARGETS_KEPT || kept.contains_key(target) {
        target
    } else {
        ""
    };
    let found = kept
        .get(key)
        .and_then(|levels| levels.iter().find(|&&(at, _)| at == level));
    if let Some(&(_, found)) = found {
        return found;
    }
    let made: &'static Relayed = Box::leak(Box::new(Relayed {
        metadata: OnceLock::new(),
    }));
    let shown: &'static str = Box::leak(engine_target(key).into_boxed_str());
    let _set = made.metadata.set(Metadata::new(
        "$/log",
        shown,
        level,
        None,
        None,
        None,
        FieldSet::new(&FIELDS, Identifier(made)),
        Kind::EVENT,
    ));
    callsite::register(made);
    kept.entry(key.to_owned()).or_default().push((level, made));
    made
}

/// A callsite of the engine's events, made when the first event of its
/// target and level arrives.
struct Relayed {
    metadata: OnceLock<Metadata<'static>>,
}

impl Relayed {
    /// Its metadata, kept for the process.
    fn kept(&'static self) -> &'static Metadata<'static> {
        self.metadata
            .get()
            .expect("invariant: a relayed callsite's metadata is set before it is registered")
    }
}

impl Callsite for Relayed {
    /// Every event asks the subscriber whether it records it, so the
    /// interest is not kept.
    fn set_interest(&self, _interest: Interest) {}

    fn metadata(&self) -> &Metadata<'_> {
        self.metadata
            .get()
            .expect("invariant: a relayed callsite's metadata is set before it is registered")
    }
}

#[cfg(test)]
mod tests;
