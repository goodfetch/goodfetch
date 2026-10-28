//! "Needs you" (ADRs 0031, 0087 and 0129; spec #592): one list over every
//! source of a person's work. Each source is a value a domain crate passes
//! in its contribution, so Foxhound adds its kinds without Collie naming
//! them; it fills only its primitives, its items as they are now and when
//! they may have changed. The driver here does the rest for all of them:
//! it merges their items into one list, the longest waiting first, keeps
//! each item's first `since`, publishes each change as the lossless
//! `needs_you.changed`, answers `needs_you.list` a page at a time, and
//! dismisses an item whose source allows it, keeping the dismissal in the
//! Run index so that every engine of the major hides it. The engine's own
//! sources are the storage entries of ADR 0122, in [`storage`], the Runs
//! stopped by a Limit a person may raise, in [`limits`], the consent
//! walls of ADR 0048, in [`consent`], the stops of ADR 0052's refetch
//! without the site's stored data, in [`refetch`], and the certificates
//! whose key changed of ADR 0042, in [`certificates`].

pub mod certificates;
pub mod consent;
pub mod limits;
pub mod refetch;
pub mod storage;

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::needs_you::{
    NEEDS_YOU_DISMISS, NeedsYouChange, NeedsYouItem, NeedsYouList, NeedsYouListParams,
    NeedsYouRemoval, PAGE,
};
use goodfetch_protocol::run::PointInTime;
use tokio::sync::{mpsc, oneshot, watch};

use crate::clock::Clock;
use crate::events::Events;
use crate::governor::StorageSample;
use crate::known_certificates::KnownCertificates;
use crate::ledger::Ledger;
use crate::run_index;
use crate::run_state::RunStateError;
use crate::settings::Settings;
use crate::tasks::TaskOwner;

/// How many commands wait for the driver before their senders wait.
const COMMANDS: usize = 256;

/// A boxed future of a source's answer.
pub type Answering<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a source fills (ADR 0129): its items as they are now, and when they
/// may have changed. The driver merges, orders, publishes and dismisses.
pub trait Primitives: Send + Sync {
    /// Whether a person may dismiss its items.
    fn dismissable(&self) -> bool;

    /// Its items now, each with an ID of its own that names the one
    /// occurrence for good.
    ///
    /// # Errors
    ///
    /// When it cannot read them: its items stay listed as they were, and
    /// the failure is logged.
    fn items<'a>(
        &'a self,
        context: &'a SourceContext,
    ) -> Answering<'a, Result<Vec<NeedsYouItem>, SourceError>>;

    /// When its items may have changed.
    fn changes(&self, context: &SourceContext) -> Changes;
}

/// When a source's items may have changed: a sequence that yields at each
/// change, and ends when they never change again.
pub struct Changes(BoxStream<'static, ()>);

impl Changes {
    /// At each change of the value `watched` sees, until its sender is
    /// gone.
    pub fn of<T: Send + Sync + 'static>(watched: watch::Receiver<T>) -> Self {
        Self(
            futures_util::stream::unfold(watched, |mut watched| async move {
                watched.changed().await.ok()?;
                Some(((), watched))
            })
            .boxed(),
        )
    }

    /// Never: the items stay as they are first read.
    pub fn none() -> Self {
        Self(futures_util::stream::empty().boxed())
    }

    /// Waits for the next change; `None` once there will be none.
    async fn next(&mut self) -> Option<()> {
        self.0.next().await
    }
}

/// One source of "Needs you" items, as a domain crate passes it in its
/// contribution.
#[derive(Clone)]
pub struct NeedsYouSource(Arc<dyn Primitives>);

impl NeedsYouSource {
    /// The source whose primitives are `primitives`.
    pub fn new(primitives: impl Primitives + 'static) -> Self {
        Self(Arc::new(primitives))
    }
}

impl std::fmt::Debug for NeedsYouSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NeedsYouSource")
            .finish_non_exhaustive()
    }
}

/// Why a source could not read its items.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SourceError(String);

impl SourceError {
    /// A failure that `message` describes.
    pub fn new(message: impl std::fmt::Display) -> Self {
        Self(message.to_string())
    }
}

/// What every source reads from: the engine's `Clock`, the Resource
/// governor's storage samples, its major's Run index, the user's Usage
/// ledger, the machine's settings, the user's known-certificates store and
/// an owner for blocking reads.
#[derive(Clone)]
pub struct SourceContext {
    pub(crate) clock: Clock,
    pub(crate) storage: watch::Receiver<Arc<StorageSample>>,
    pub(crate) run_index: PathBuf,
    pub(crate) version: ProductVersion,
    pub(crate) ledger: Ledger,
    pub(crate) settings: Settings,
    pub(crate) known_certificates: KnownCertificates,
    pub(crate) owner: Arc<TaskOwner>,
}

impl SourceContext {
    /// The engine's `Clock`.
    pub fn clock(&self) -> Clock {
        self.clock
    }

    /// The Resource governor's storage samples, which change at each
    /// sample (ADR 0122).
    pub fn storage(&self) -> watch::Receiver<Arc<StorageSample>> {
        self.storage.clone()
    }

    /// Runs `work`, which blocks, under the engine's blocking policy.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer: `work` runs to its end.
    pub async fn blocking<T, F>(&self, work: F) -> T
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        self.owner.blocking(work).await
    }

    /// The point in time it is now.
    pub fn now(&self) -> PointInTime {
        let wall = jiff::Timestamp::try_from(self.clock.wall())
            .expect("invariant: the clock reads a time jiff can hold");
        PointInTime::from_rfc3339(
            wall.round(jiff::Unit::Millisecond)
                .unwrap_or(wall)
                .to_string(),
        )
    }
}

/// Why "Needs you" did not answer.
#[derive(Debug, thiserror::Error)]
pub enum NeedsYouError {
    /// The driver has stopped with its engine.
    #[error("\"Needs you\" has stopped with its engine")]
    Stopped,
    /// A `needs_you.list` page's `after` is none a page gave.
    #[error("`after` is not the `next` of a page: {0}")]
    Cursor(String),
    /// The Run index did not keep a dismissal.
    #[error("the dismissal was not kept in the Run index: {0}")]
    Index(#[from] RunStateError),
}

/// The engine's "Needs you": a handle to its driver, which any task may
/// clone.
#[derive(Clone, Debug)]
pub struct NeedsYou {
    commands: mpsc::Sender<Command>,
}

enum Command {
    /// A source read its items, or failed to (`None`).
    Refreshed {
        source: usize,
        items: Option<Vec<NeedsYouItem>>,
    },
    List {
        params: NeedsYouListParams,
        answer: oneshot::Sender<Result<NeedsYouList, NeedsYouError>>,
    },
    Dismiss {
        item: String,
        answer: oneshot::Sender<Result<bool, NeedsYouError>>,
    },
}

impl NeedsYou {
    /// Starts the driver over `sources` as tasks of `owner`, publishing on
    /// `events`; each source reads `context`.
    pub(crate) fn start(
        sources: &[NeedsYouSource],
        context: SourceContext,
        events: Events,
        owner: &TaskOwner,
    ) -> Self {
        let (commands, mailbox) = mpsc::channel(COMMANDS);
        let token = owner.token().clone();
        for (index, source) in sources.iter().enumerate() {
            let source = Arc::clone(&source.0);
            let context = context.clone();
            let commands = commands.clone();
            let token = token.clone();
            owner.spawn(async move {
                tokio::select! {
                    () = watch_source(index, source.as_ref(), &context, &commands) => {}
                    () = token.cancelled() => {}
                }
            });
        }
        let driver = Driver {
            dismissable: sources
                .iter()
                .map(|source| source.0.dismissable())
                .collect(),
            held: vec![None; sources.len()],
            tried: vec![false; sources.len()],
            waiting: Vec::new(),
            listed: BTreeMap::new(),
            dismissed: BTreeSet::new(),
            context,
            events,
        };
        owner.spawn(async move {
            tokio::select! {
                () = driver.run(mailbox) => {}
                () = token.cancelled() => {}
            }
        });
        Self { commands }
    }

    /// A page of the list, as `needs_you.list` answers it.
    ///
    /// # Errors
    ///
    /// `Cursor` for an `after` no page gave, `Stopped` once the engine has
    /// stopped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn list(&self, params: NeedsYouListParams) -> Result<NeedsYouList, NeedsYouError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::List { params, answer })
            .await
            .map_err(|_| NeedsYouError::Stopped)?;
        answered.await.map_err(|_| NeedsYouError::Stopped)?
    }

    /// Dismisses the item `item` when the list holds it and its source
    /// allows it, answering whether it did.
    ///
    /// # Errors
    ///
    /// `Index` when the Run index did not keep the dismissal, which then
    /// did not happen; `Stopped` once the engine has stopped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after it was sent still dismisses the item.
    pub async fn dismiss(&self, item: String) -> Result<bool, NeedsYouError> {
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Dismiss { item, answer })
            .await
            .map_err(|_| NeedsYouError::Stopped)?;
        answered.await.map_err(|_| NeedsYouError::Stopped)?
    }
}

/// Reads the source `index`'s items now and after each change it yields,
/// handing each read to the driver.
async fn watch_source(
    index: usize,
    source: &dyn Primitives,
    context: &SourceContext,
    commands: &mpsc::Sender<Command>,
) {
    let mut changes = source.changes(context);
    loop {
        let items = match source.items(context).await {
            Ok(items) => Some(items),
            Err(error) => {
                tracing::warn!(%error, "a source of \"Needs you\" could not read its items: they stay as they were");
                None
            }
        };
        let refreshed = Command::Refreshed {
            source: index,
            items,
        };
        if commands.send(refreshed).await.is_err() {
            return;
        }
        if changes.next().await.is_none() {
            return;
        }
    }
}

/// Where an item stands in the list: the longest waiting first, by the
/// time its `since` names rather than its text, since sources write
/// fractions of a second and offsets differently; a `since` that does not
/// parse after every one that does; then by ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Waited<'a> {
    unknown: bool,
    since: Option<jiff::Timestamp>,
    id: &'a str,
}

impl<'a> Waited<'a> {
    /// The place of `item`.
    fn of(item: &'a NeedsYouItem) -> Self {
        Self::new(item.since.as_str(), &item.id)
    }

    /// The place of the item `id` waiting since `since`, as a cursor
    /// names it.
    fn new(since: &str, id: &'a str) -> Self {
        let since = since.parse::<jiff::Timestamp>().ok();
        Self {
            unknown: since.is_none(),
            since,
            id,
        }
    }
}

/// An item as the list holds it, with the source it came from.
#[derive(Clone)]
struct Listed {
    source: usize,
    item: NeedsYouItem,
}

/// The driver's state.
struct Driver {
    /// Whether each source lets a person dismiss its items.
    dismissable: Vec<bool>,
    /// Each source's last read, none before its first.
    held: Vec<Option<Vec<NeedsYouItem>>>,
    /// Whether each source has tried its first read.
    tried: Vec<bool>,
    /// The calls that wait for every source's first read.
    waiting: Vec<Command>,
    /// The items listed, by ID.
    listed: BTreeMap<String, Listed>,
    /// The IDs a person dismissed.
    dismissed: BTreeSet<String>,
    context: SourceContext,
    events: Events,
}

impl Driver {
    async fn run(mut self, mut mailbox: mpsc::Receiver<Command>) {
        self.dismissed = self.read_dismissed().await;
        while let Some(command) = mailbox.recv().await {
            match command {
                Command::Refreshed { source, items } => {
                    self.tried[source] = true;
                    if let Some(items) = items {
                        self.refresh(source, items).await;
                    }
                    if self.tried.iter().all(|tried| *tried) {
                        for call in std::mem::take(&mut self.waiting) {
                            self.answer(call).await;
                        }
                    }
                }
                // A call answers once every source has tried its first
                // read, so the first one after a start misses no item.
                call if !self.tried.iter().all(|tried| *tried) => self.waiting.push(call),
                call => self.answer(call).await,
            }
        }
    }

    async fn answer(&mut self, call: Command) {
        match call {
            Command::List { params, answer } => _ = answer.send(self.list(&params)),
            Command::Dismiss { item, answer } => {
                let dismissed = self.dismiss(item).await;
                let _ = answer.send(dismissed);
            }
            Command::Refreshed { .. } => {}
        }
    }

    async fn read_dismissed(&self) -> BTreeSet<String> {
        let path = self.context.run_index.clone();
        let version = self.context.version.clone();
        match self
            .context
            .blocking(move || run_index::dismissed(&path, &version))
            .await
        {
            Ok(dismissed) => dismissed,
            Err(error) => {
                tracing::warn!(%error, "the dismissed items of \"Needs you\" cannot be read: they are listed again");
                BTreeSet::new()
            }
        }
    }

    /// Takes the source `source`'s read `items`: each item it no longer
    /// holds leaves the list, each new or changed one is listed as it is
    /// now, keeping the `since` it first had.
    async fn refresh(&mut self, source: usize, items: Vec<NeedsYouItem>) {
        let mut read: BTreeMap<String, NeedsYouItem> = BTreeMap::new();
        for item in items {
            read.entry(item.id.clone()).or_insert(item);
        }
        let gone: Vec<String> = self
            .listed
            .iter()
            .filter(|(id, listed)| listed.source == source && !read.contains_key(*id))
            .map(|(id, _)| id.clone())
            .collect();
        for id in gone {
            self.listed.remove(&id);
            self.events
                .publish_needs_you(NeedsYouChange::Removed {
                    id,
                    reason: NeedsYouRemoval::Resolved,
                })
                .await;
        }
        for (id, mut item) in read.clone() {
            if self.dismissed.contains(&id) {
                continue;
            }
            if self.dismissable[source] {
                item.actions.push(NEEDS_YOU_DISMISS.name.to_owned());
            }
            if let Some(listed) = self.listed.get(&id) {
                item.since = listed.item.since.clone();
                if listed.item == item {
                    continue;
                }
            }
            self.listed.insert(
                id,
                Listed {
                    source,
                    item: item.clone(),
                },
            );
            self.events
                .publish_needs_you(NeedsYouChange::Listed { item })
                .await;
        }
        self.held[source] = Some(read.into_values().collect());
        self.forget_resolved_dismissals().await;
    }

    /// Once every source has read its items, forgets each dismissal of an
    /// item none of them holds any longer.
    async fn forget_resolved_dismissals(&mut self) {
        let Some(held) = self
            .held
            .iter()
            .map(Option::as_ref)
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        let holding: BTreeSet<&str> = held
            .iter()
            .flat_map(|items| items.iter().map(|item| item.id.as_str()))
            .collect();
        let resolved: Vec<String> = self
            .dismissed
            .iter()
            .filter(|id| !holding.contains(id.as_str()))
            .cloned()
            .collect();
        if resolved.is_empty() {
            return;
        }
        let path = self.context.run_index.clone();
        let version = self.context.version.clone();
        let forgotten = resolved.clone();
        match self
            .context
            .blocking(move || run_index::forget_dismissed(&path, &version, &forgotten))
            .await
        {
            Ok(()) => {
                for id in &resolved {
                    self.dismissed.remove(id);
                }
            }
            Err(error) => {
                tracing::warn!(%error, "the dismissals of resolved \"Needs you\" items were not forgotten");
            }
        }
    }

    fn list(&self, params: &NeedsYouListParams) -> Result<NeedsYouList, NeedsYouError> {
        let after = params
            .after
            .as_deref()
            .map(|cursor| {
                cursor
                    .split_once(' ')
                    .map(|(since, id)| Waited::new(since, id))
                    .ok_or_else(|| NeedsYouError::Cursor(cursor.to_owned()))
            })
            .transpose()?;
        let mut items: Vec<&NeedsYouItem> = self
            .listed
            .values()
            .map(|listed| &listed.item)
            .filter(|item| {
                params
                    .kinds
                    .as_ref()
                    .is_none_or(|kinds| kinds.contains(&item.kind))
            })
            .filter(|item| after.is_none_or(|after| Waited::of(item) > after))
            .collect();
        items.sort_by_key(|&item| Waited::of(item));
        let more = items.len() > PAGE;
        items.truncate(PAGE);
        let next = more
            .then(|| items.last())
            .flatten()
            .map(|last| format!("{} {}", last.since.as_str(), last.id));
        Ok(NeedsYouList {
            items: items.into_iter().cloned().collect(),
            next,
        })
    }

    async fn dismiss(&mut self, id: String) -> Result<bool, NeedsYouError> {
        let Some(listed) = self.listed.get(&id) else {
            return Ok(false);
        };
        if !self.dismissable[listed.source] {
            return Ok(false);
        }
        let path = self.context.run_index.clone();
        let version = self.context.version.clone();
        let at = self.context.now();
        let kept = id.clone();
        self.context
            .blocking(move || run_index::dismiss(&path, &version, &kept, &at))
            .await?;
        self.listed.remove(&id);
        self.dismissed.insert(id.clone());
        self.events
            .publish_needs_you(NeedsYouChange::Removed {
                id,
                reason: NeedsYouRemoval::Dismissed,
            })
            .await;
        Ok(true)
    }
}

#[cfg(test)]
mod tests;
