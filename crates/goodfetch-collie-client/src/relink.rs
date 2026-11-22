//! How a Connection reaches a remote Daemon again once its link dropped
//! (spec 606, ADR 0079): the backoff between attempts, each bounded by
//! `timeout.connect` counted from the drop; the same `hello`, at the log
//! level the engine was last told; and which calls in flight at the drop
//! are sent again: a read's or an idempotent write's, as it was, and a
//! stream's after the last item the Client read, while an Effect's never
//! is. TypeScript's `reconnect.ts` and `resumed-stream.ts` are its twins.

use std::future::Future;
use std::io;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use goodfetch_protocol::catalog::{OperationClass, OperationKind, Product};
use goodfetch_protocol::connection::{EngineInstance, HelloParams};
use goodfetch_protocol::entries;
use goodfetch_protocol::events::{EVENTS, EVENTS_SUBSCRIBE, LastRead, Position};
use goodfetch_protocol::log::LogLevel;
use serde_json::Value;
use tokio::runtime::Handle;
use tokio_util::task::TaskTracker;

use crate::sync::Mutex;
use crate::transport::{self, Endpoint, Link, Network};

/// The wait before the first attempt.
const FIRST_DELAY: Duration = Duration::from_millis(250);

/// Foxhound's stream of a Run's Records and the notification that carries
/// each, as the wire names them: outside `crates/foxhound/`, no code names
/// Foxhound's part of the protocol (ADR 0099), and the Records' resumption
/// lives here with the events'.
const RUN_RECORDS: &str = "run.records";
const RECORD: &str = "$/record";

/// The longest wait between two attempts.
const LONGEST_DELAY: Duration = Duration::from_secs(10);

/// How a Connection reaches its remote Daemon again. It holds `hello`'s
/// token, so it has no `Debug`.
pub(crate) struct Relinking {
    network: Network,
    endpoint: Endpoint,
    hello: HelloParams,
    /// The level the engine was last told to send its logs at.
    log_level: Arc<Mutex<LogLevel>>,
    within: Option<Duration>,
    runtime: Handle,
    tracker: TaskTracker,
}

impl Relinking {
    /// Reaches the engine at `endpoint` over `network` again, greeting it
    /// with `hello` at the level `log_level` holds, within `within`, the
    /// `timeout.connect` in force or `None` for no bound, each link's tasks
    /// on `runtime`, tracked by `tracker`.
    pub(crate) fn new(
        network: Network,
        endpoint: Endpoint,
        hello: HelloParams,
        log_level: Arc<Mutex<LogLevel>>,
        within: Option<Duration>,
        runtime: Handle,
        tracker: TaskTracker,
    ) -> Self {
        Self {
            network,
            endpoint,
            hello,
            log_level,
            within,
            runtime,
            tracker,
        }
    }

    /// How long reaching the engine again may take, from the drop.
    pub(crate) fn within(&self) -> Option<Duration> {
        self.within
    }

    /// Opens a new link to the engine.
    pub(crate) async fn reopen(&self) -> io::Result<Link> {
        Ok(transport::open(&self.network, &self.endpoint).await?.link)
    }

    /// The `hello` a new link is greeted with: the first one's, at the
    /// level the engine was last told when a start changed it.
    pub(crate) fn greeting(&self) -> HelloParams {
        let told = *self
            .log_level
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut hello = self.hello.clone();
        if told != hello.log_level.unwrap_or_default() {
            hello.log_level = Some(told);
        }
        hello
    }

    /// Runs a new link's `task` on the Connection's runtime.
    pub(crate) fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        self.tracker.spawn_on(task, &self.runtime);
    }

    /// Logs that the engine was reached again.
    pub(crate) fn reached(&self) {
        let at = match &self.endpoint {
            Endpoint::Exposed(exposed) => exposed.authority.as_str(),
            _ => "its endpoint",
        };
        tracing::info!(
            target: crate::logging::CLIENT,
            "The Connection reached the engine at {at} again after its link dropped."
        );
    }
}

/// The wait before attempt `attempt`, the first being 0: 250 ms, doubling
/// each time, at most 10 s.
pub(crate) fn delay(attempt: u32) -> Duration {
    1_u32
        .checked_shl(attempt)
        .and_then(|times| FIRST_DELAY.checked_mul(times))
        .map_or(LONGEST_DELAY, |delay| delay.min(LONGEST_DELAY))
}

/// What a call in flight at the drop is to a new link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resend {
    /// It fails with `connection_lost`: an Effect, a stream the Client
    /// cannot resume, or an operation the catalog does not know.
    Never,
    /// It is sent again as it was: a read or an idempotent write.
    AsItWas,
    /// It is sent again after the last item the Client read.
    Resumed(Stream),
}

/// What a call of `method` in flight at the drop is to a new link, as the
/// catalog classes it (ADR 0011). Foxhound's catalog holds Collie's
/// operations too, as `call`'s check of a call's kind reads it.
pub(crate) fn resend(method: &str) -> Resend {
    let Some(operation) =
        entries::operations_of(Product::Foxhound).find(|operation| operation.name == method)
    else {
        return Resend::Never;
    };
    match (operation.class, operation.kind) {
        (OperationClass::Effect, _) => Resend::Never,
        (_, OperationKind::Sequence) => Stream::of(method).map_or(Resend::Never, Resend::Resumed),
        _ => Resend::AsItWas,
    }
}

/// A stream a new link resumes after the last item the Client read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stream {
    /// `events.subscribe`, after the last event's position, as the engine
    /// instance that published it.
    Events,
    /// `run.records`, after the last Record's number.
    Records,
}

impl Stream {
    /// The stream `method` opens, if one resumes.
    fn of(method: &str) -> Option<Self> {
        match method {
            _ if method == EVENTS_SUBSCRIBE.name => Some(Self::Events),
            RUN_RECORDS => Some(Self::Records),
            _ => None,
        }
    }

    /// The items the stream's notification `method` carries, each of which
    /// used one credit, and where the last stands: an event's position or
    /// a Record's number; none for a notification that carries none.
    pub(crate) fn read(self, method: &str, params: &Value) -> Option<(u64, Option<u64>)> {
        match self {
            Self::Events if method == EVENTS => {
                let events = params.get("events")?.as_array()?;
                let last = events
                    .last()
                    .and_then(|event| event.get("position")?.as_u64());
                Some((u64::try_from(events.len()).unwrap_or(u64::MAX), last))
            }
            Self::Records if method == RECORD => {
                let number = params.get("record")?.get("number")?.as_u64();
                Some((1, number))
            }
            Self::Events | Self::Records => None,
        }
    }

    /// `params` resumed after `last`, where the last item read stands, an
    /// event's as `instance` published it; unchanged when an event's
    /// publisher is unknown, so the stream starts with the next one.
    pub(crate) fn resume(self, params: &mut Value, last: u64, instance: Option<&EngineInstance>) {
        let after = match self {
            Self::Events => instance.and_then(|instance| {
                serde_json::to_value(LastRead {
                    engine_instance: instance.clone(),
                    position: Position(last),
                })
                .ok()
            }),
            Self::Records => Some(Value::from(last)),
        };
        if let Some((params, after)) = params.as_object_mut().zip(after) {
            params.insert("after".to_owned(), after);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backoff_doubles_from_250_ms_to_at_most_10_s() {
        let delays: Vec<u128> = [0, 1, 2, 5, 6, 40].map(|n| delay(n).as_millis()).into();

        assert_eq!(delays, [250, 500, 1000, 8000, 10_000, 10_000]);
    }
}
