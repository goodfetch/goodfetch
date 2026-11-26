//! A crawl's Sinks on its Run: [`Sinks`], the value Foxhound's composition
//! passes the crawl, refuses at the start a Sink this engine cannot deliver
//! to yet, and delivers each other Sink's Datasets from the run file while
//! the Run goes. A Sink reads only committed Records, so it waits for each
//! of the run file's commits, one per Run-state batch, and never forces
//! one while the Run goes (spec #2138); the Run's end writes the last
//! batch and waits for a last delivery.

use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::run_state::RunState;
use goodfetch_engine::runs::{RunKeys, RunParts};
use goodfetch_engine::secrets::Purpose;
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::{CallError, named};
use goodfetch_protocol::foxhound::sink::{DeliveredAs, SINKS, SinkSettings, SinkType, WriteMode};
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SettingsLayer,
};
use rusqlite::Connection;
use secrecy::ExposeSecret as _;
use tokio::sync::OnceCell;

use crate::contract::RunCopy;
use crate::driver::{Driver, Status};
use crate::sqlite::SqliteSink;
use crate::sync::{AtomicBool, Mutex};

/// The table a SQLite Sink writes when its `target` names none.
const SQLITE_TARGET: &str = "records";

/// Goodfetch-sink's composition value (ADR 0087): what the crawl's Run
/// asks of its Sinks, the catalog's in-binary destinations this engine
/// delivers to, SQLite so far.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sinks;

impl Sinks {
    /// The Sinks this engine delivers to.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Refuses `settings` when one of their `sinks` asks for what this
    /// engine does not deliver yet, rather than run with its Records going
    /// nowhere (ADR 0083).
    ///
    /// # Errors
    ///
    /// `capability_unavailable` naming the first such Sink's key: its
    /// `type` when it is not `sqlite` or is left out for a program's own
    /// handler, its `mode` when not `append`, its `deliver` when not
    /// `records`.
    pub fn check(self, settings: &Resolved) -> Result<(), CallError> {
        let layer = layer(settings);
        for (name, sink) in entries(settings) {
            let Some((key, kind)) = unavailable(&sink) else {
                continue;
            };
            let key = format!("{}.{name}.{key}", SINKS.name);
            return Err(CallError::new(
                &CAPABILITY_UNAVAILABLE,
                format!(
                    "`{}` asks for a Sink's `{kind}`, which this engine does not deliver yet: deliver to a `type: sqlite` Sink in `append` mode as `records`, or leave the Sink out.",
                    named::setting_in(&key, layer),
                ),
                &CapabilityUnavailableDetails {
                    key,
                    layer,
                    kind,
                    fix: "`type: sqlite`, `mode: append`, `deliver: records`".to_owned(),
                },
            ));
        }
        Ok(())
    }

    /// Starts delivering the Run `run`'s Datasets to each of the Sinks its
    /// `settings` name, as tasks of the Run, each waking at the run file's
    /// commits; the answer finishes the deliveries before the Run ends.
    /// `settings` passed [`Sinks::check`].
    #[must_use]
    pub fn deliver(self, run: &RunParts, settings: &Resolved) -> Delivery {
        let layer = layer(settings);
        let sinks: Vec<Arc<Sink>> = entries(settings)
            .map(|(name, settings)| {
                Arc::new(Sink {
                    name,
                    settings,
                    layer,
                    identity: run.identity.clone(),
                    state: run.state.clone(),
                    keys: run.keys.clone(),
                    driver: OnceCell::new(),
                    stopped: AtomicBool::new(false),
                })
            })
            .collect();
        for sink in &sinks {
            run.identity.spawn(keep_delivering(Arc::clone(sink)));
        }
        Delivery { sinks }
    }
}

/// The deliveries of one Run to its Sinks.
pub struct Delivery {
    sinks: Vec<Arc<Sink>>,
}

impl Delivery {
    /// Writes the run file's open batch, then delivers once more to each
    /// Sink still delivering what the run file has committed, then stops
    /// its deliveries: what the Run's end waits for, so a Sink holds the
    /// Records the Run committed before its end, the last batch's included.
    /// A Sink behind on its retries stays behind; a resumed copy of the
    /// Run delivers again.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the Sinks it had not reached stopped,
    /// with what they last delivered.
    pub async fn finish(&self) {
        let Some(first) = self.sinks.first() else {
            return;
        };
        if let Err(error) = first.state.seal().await {
            tracing::warn!(%error, "the run file's last batch did not commit before the Sinks' last delivery");
        }
        for sink in &self.sinks {
            if !sink.stopped.swap(true, Ordering::SeqCst) {
                sink.deliver().await;
            }
        }
    }
}

/// One Sink of a Run: its entry and what its delivery reads and writes.
struct Sink {
    name: String,
    settings: SinkSettings,
    layer: SettingsLayer,
    identity: RunIdentity,
    state: RunState,
    keys: RunKeys,
    /// Its delivery driver, once its destination opened; none when it did
    /// not.
    driver: OnceCell<Option<Arc<Mutex<Driver<SqliteSink>>>>>,
    /// Whether its delivery stopped: failed, superseded or finished.
    stopped: AtomicBool,
}

/// What a Sink's delivery does after an attempt.
enum Next {
    /// Waits for the run file's next commit.
    Wait,
    /// Tries again after the wait.
    Retry(Duration),
    /// Stops.
    Stop,
}

impl Sink {
    /// Delivers what the run file has committed and says what follows.
    async fn deliver(&self) -> Next {
        let Some(driver) = self.driver.get_or_init(|| self.open()).await.clone() else {
            self.stopped.store(true, Ordering::SeqCst);
            return Next::Stop;
        };
        let delivered = self
            .state
            .read(move |run_file| {
                driver
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .deliver(run_file)
            })
            .await;
        let next = match delivered {
            Ok(Status::Complete) => Next::Wait,
            Ok(Status::Behind { failure, retry_in }) => {
                tracing::info!(sink = %self.name, %failure, ?retry_in, "a Sink's delivery failed and is retried");
                Next::Retry(retry_in)
            }
            Ok(Status::Failed(failure)) => {
                tracing::warn!(sink = %self.name, %failure, "a Sink failed: it delivers nothing more");
                Next::Stop
            }
            Ok(Status::Superseded { run_epoch }) => {
                tracing::info!(sink = %self.name, run_epoch, "a later copy of the Run delivers to this Sink: this copy stops");
                Next::Stop
            }
            Err(error) => {
                tracing::warn!(sink = %self.name, %error, "a Sink's delivery could not read the run file: it stops");
                Next::Stop
            }
        };
        if matches!(next, Next::Stop) {
            self.stopped.store(true, Ordering::SeqCst);
        }
        next
    }

    /// Its delivery driver over its destination, resolved from its Secret
    /// reference and opened; none, logged, when either fails.
    async fn open(&self) -> Option<Arc<Mutex<Driver<SqliteSink>>>> {
        let reference = SecretReference::parse(
            self.settings.destination.as_deref().expect(
                "invariant: `SinkSettings::read` refuses a typed Sink with no `destination`",
            ),
        )
        .expect(
            "invariant: `SinkSettings::read` refuses a `destination` that is no Secret reference",
        );
        let placeholder = format!("{}.{}.destination", SINKS.name, self.name);
        let purpose = Purpose {
            placeholder: &placeholder,
            domain: None,
            domains: &[],
            layer: Some(self.layer),
            run_id: Some(self.identity.id()),
        };
        let path = match self.keys.secrets().resolve(&reference, &purpose).await {
            Ok(path) => path.expose_secret().to_owned(),
            Err(error) => {
                tracing::warn!(sink = %self.name, %error, "a Sink's destination did not resolve: it delivers nothing");
                return None;
            }
        };
        let opened = self
            .identity
            .owner()
            .blocking(move || Connection::open(path))
            .await;
        let connection = match opened {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(sink = %self.name, %error, "a Sink's SQLite database did not open: it delivers nothing");
                return None;
            }
        };
        let target = self.settings.target.as_deref().unwrap_or(SQLITE_TARGET);
        let run = RunCopy {
            run_id: self.identity.id().as_str().to_owned(),
            run_epoch: self.state.epoch(),
        };
        Some(Arc::new(Mutex::new(Driver::new(
            &self.name,
            SqliteSink::new(connection, target),
            run,
            self.settings.datasets.clone(),
        ))))
    }
}

/// Delivers to `sink` at each of the run file's commits until it stops,
/// or the run file's writer does.
async fn keep_delivering(sink: Arc<Sink>) {
    let mut commits = sink.state.commits();
    let clock = sink.identity.clock();
    while !sink.stopped.load(Ordering::SeqCst) {
        match sink.deliver().await {
            Next::Wait => {
                if commits.changed().await.is_err() {
                    return;
                }
            }
            Next::Retry(after) => clock.sleep(after).await,
            Next::Stop => return,
        }
    }
}

/// Each Sink `settings` name, with its entry.
fn entries(settings: &Resolved) -> impl Iterator<Item = (String, SinkSettings)> + '_ {
    settings
        .value(&SINKS)
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, entry)| {
            let entry = SinkSettings::read(entry)
                .expect("invariant: `sinks` resolved only from entries its shape's check read");
            (name.clone(), entry)
        })
}

/// The key of `sink`'s entry that asks for what this engine does not
/// deliver yet, with the feature it asks for; none when it delivers it.
fn unavailable(sink: &SinkSettings) -> Option<(&'static str, String)> {
    let Some(sink_type) = sink.sink_type else {
        return Some(("type", "program_sink".to_owned()));
    };
    if sink_type != SinkType::Sqlite {
        return Some(("type", wire_name(serde_json::to_value(sink_type))));
    }
    if let Some(mode) = sink.mode.filter(|mode| *mode != WriteMode::Append) {
        return Some(("mode", wire_name(serde_json::to_value(mode))));
    }
    sink.deliver
        .filter(|deliver| *deliver != DeliveredAs::Records)
        .map(|deliver| ("deliver", wire_name(serde_json::to_value(deliver))))
}

/// The layer that set `sinks`, the Run's when none did.
fn layer(settings: &Resolved) -> SettingsLayer {
    settings
        .effective(&SINKS)
        .map_or(SettingsLayer::Run, |effective| effective.layer)
}

/// The wire name of a value of one of the Sink entry's enums.
fn wire_name(value: serde_json::Result<serde_json::Value>) -> String {
    value
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("invariant: a Sink entry's enums encode as their snake_case names")
}
