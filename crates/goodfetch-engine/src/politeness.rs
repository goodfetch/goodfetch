//! Politeness shared on one machine (ADRs 0053, 0060, 0086, 0088 and 0098):
//! the per-user Politeness file, outside the per-major folders, which every
//! engine of the user, of either Product, opens. An engine takes a Permit
//! for each request in flight under a key, such as a domain key, and gives
//! it back once the request ended, each in one SQLite transaction, so the
//! count in flight under a key is every sharing run's together. What a key
//! admits is a rule a Product's composition passes in, such as Foxhound's
//! per-domain cap, delay, backoff and 429 halving, kept in the tables it
//! adds to the file; a Collie engine adds none and touches none.
//!
//! An engine refused for want of room registers its wake endpoint (from
//! `transport`) as a waiter of the key; the engine that gives a Permit of
//! that key back wakes the waiter whose turn it is, whose room is kept for
//! it a moment ([`WAKE_GRACE`]) so a run that did not wait cannot take it
//! first; a waiter woken but still refused, by a stricter cap of its own,
//! passes the wake to the next, so waiters take turns and nothing polls. A wait for a deadline (a delay,
//! a `Retry-After`, a backoff) is a timer the caller sets.
//!
//! A Permit records its Run, run epoch and folder: an engine refused for
//! want of room first reclaims the Permits of each Run whose writer lock is
//! free, or held at a later epoch, as ADR 0041 reclaims slices.
//!
//! The file holds nothing to keep, so it skips `fsync`; it opens through a
//! VFS named by a value (ADR 0060). Its format is of the Usage ledger's
//! family (ADR 0066): its major never changes and newer minors only add. A
//! Run with `politeness.sharing = run` takes its Permits from tables of
//! its own in memory, without SQLite ([`MemoryTables`]), under the same
//! rules, its waiters woken within the engine: it opens nothing and runs
//! no transaction (ADR 0140's `E6.private_transactions`). A crawl's
//! admission holds that Politeness itself ([`OwnPoliteness`]): its takes
//! and give backs wait on no task, and the writes of its pages' tasks are
//! one-way messages its driver runs (#2171).
//!
//! Size: one file's protocol, its Permits, waiters, wakes and reclaim, each
//! in one transaction, and the Run's own Politeness held in memory under
//! the same rules, kept whole so the rules that guard the count in flight
//! under a key change together.

use std::borrow::Borrow;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::formats::{Compatibility, POLITENESS_FILE};
use rusqlite::{Connection, OptionalExtension as _};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::sqlite_store::{
    self, Access, Counting, Durability, File, Folder, FullFsync, OpenError, Opening, Schema,
    Synchronous, Wait, WalOnClose,
};
use crate::tasks::TaskOwner;
use crate::transport;

mod memory;

pub use memory::MemoryTables;

/// How many jobs wait for the task that holds a Politeness before their
/// callers wait.
const MAILBOX: usize = 256;

/// How many wakes a subscriber may fall behind; one further behind reads
/// as woken for every key.
const WAKES: usize = 256;

/// How long the room a give back freed is kept for the waiter it woke:
/// another run counts it as in flight meanwhile.
const WAKE_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// How long a statement waits while another engine's connection holds the
/// file, its switch to WAL included.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// How its commits reach the disk: never synced, as it holds nothing to
/// keep.
const DURABILITY: Durability = Durability {
    synchronous: Synchronous::Off,
    opening: Synchronous::Off,
    full_fsync: FullFsync::Off,
    wal_on_close: WalOnClose::Checkpointed,
};

/// How many prepared statements the file's connection keeps: more than
/// the engine and its tables' contributions prepare together, so the
/// connection prepares none twice.
const STATEMENTS: usize = 64;

/// The wake endpoint's name in its folder.
const WAKE_SOCKET: &str = "wake.sock";

/// The endpoint a Politeness file of one Run's own, in memory, names for
/// its waiters: every one is in this engine, woken without a socket.
const IN_MEMORY_ENDPOINT: &str = "in-memory";

/// The engine's own tables, which every Politeness file holds.
const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS permits (
        permit_id INTEGER NOT NULL PRIMARY KEY,
        key TEXT NOT NULL,
        run_id TEXT NOT NULL,
        epoch INTEGER NOT NULL,
        run_folder TEXT NOT NULL,
        engine TEXT NOT NULL,
        taken_at INTEGER NOT NULL
    ) STRICT",
    "CREATE INDEX IF NOT EXISTS permits_by_key ON permits (key)",
    "CREATE TABLE IF NOT EXISTS waiters (
        key TEXT NOT NULL,
        run_id TEXT NOT NULL,
        endpoint TEXT NOT NULL,
        turn INTEGER NOT NULL,
        woken INTEGER NOT NULL,
        woken_at INTEGER NOT NULL,
        PRIMARY KEY (key, run_id)
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS returns (key TEXT NOT NULL PRIMARY KEY, generation INTEGER NOT NULL) STRICT",
];

/// The turn after the last of the waiters of a key, `?1`.
macro_rules! back_of_turns {
    () => {
        "(SELECT coalesce(max(turn), 0) + 1 FROM waiters WHERE key = ?1)"
    };
}

// The engine's own statements, each run on the file's connection.
const STOP_WAITING: &str = "DELETE FROM waiters WHERE run_id = ?1 AND endpoint = ?2";
const GONE_WAITERS: &str = "DELETE FROM waiters WHERE endpoint = ?1";
const IN_FLIGHT: &str = "SELECT count(*) FROM permits WHERE key = ?1";
const GENERATION: &str = "SELECT generation FROM returns WHERE key = ?1";
const NEXT_WAITER: &str =
    "SELECT run_id, endpoint FROM waiters WHERE key = ?1 AND woken < ?2 ORDER BY turn LIMIT 1";
const WAKE_WAITER: &str =
    "UPDATE waiters SET woken = ?3, woken_at = ?4 WHERE key = ?1 AND run_id = ?2";
const KEPT_ROOM: &str = "SELECT count(*) FROM waiters
    WHERE key = ?1 AND run_id != ?2 AND woken > 0 AND woken = ?3 AND woken_at > ?4";
const OTHER_HOLDERS: &str =
    "SELECT DISTINCT run_id, epoch, run_folder FROM permits WHERE key = ?1 AND engine != ?2";
const RECLAIM: &str = "DELETE FROM permits WHERE run_id = ?1 AND epoch = ?2";
const TAKE: &str = "INSERT INTO permits (key, run_id, epoch, run_folder, engine, taken_at)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6)";
const LEAVE_TURNS: &str = "DELETE FROM waiters WHERE key = ?1 AND run_id = ?2";
const WOKEN: &str = "SELECT woken FROM waiters WHERE key = ?1 AND run_id = ?2";
const JOIN_TURNS: &str = concat!(
    "INSERT INTO waiters (key, run_id, endpoint, turn, woken, woken_at) VALUES (?1, ?2, ?3, ",
    back_of_turns!(),
    ", ?4, 0)"
);
const REJOIN_TURNS: &str = concat!(
    "UPDATE waiters SET turn = ",
    back_of_turns!(),
    ", endpoint = ?3, woken_at = 0 WHERE key = ?1 AND run_id = ?2"
);
const GIVE_BACK: &str = "DELETE FROM permits WHERE permit_id = ?1";
const NEXT_GENERATION: &str = "INSERT INTO returns (key, generation) VALUES (?1, 1)
    ON CONFLICT (key) DO UPDATE SET generation = generation + 1
    RETURNING generation";

/// The engine's own statements, prepared once when the file opens, so no
/// Permit prepares one (ADR 0140's `E6.opens`).
const PREPARED: &[&str] = &[
    STOP_WAITING,
    GONE_WAITERS,
    IN_FLIGHT,
    GENERATION,
    NEXT_WAITER,
    WAKE_WAITER,
    KEPT_ROOM,
    OTHER_HOLDERS,
    RECLAIM,
    TAKE,
    LEAVE_TURNS,
    WOKEN,
    JOIN_TURNS,
    REJOIN_TURNS,
    GIVE_BACK,
    NEXT_GENERATION,
];

/// The tables one contribution adds to the Politeness file: `CREATE TABLE
/// IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS` statements, run in order
/// when it opens.
#[derive(Clone, Copy, Debug)]
pub struct PolitenessTables {
    /// Who adds them.
    pub name: &'static str,
    /// Their statements.
    pub statements: &'static [&'static str],
    /// The statements its rule runs on the file, prepared once when the
    /// file opens, so no Permit prepares one (ADR 0140's `E6.opens`).
    pub prepared: &'static [&'static str],
}

/// How the Politeness file opens.
#[derive(Clone, Debug, Default)]
pub struct PolitenessOptions {
    /// The name of the SQLite VFS it opens through: none in production, the
    /// crash VFS in tests (ADR 0060).
    pub vfs: Option<String>,
    /// Where it counts its opens and transactions, and the robots.txt copy
    /// kept in it its reads and parses (ADR 0140's E6).
    pub work: CountedWork,
}

/// Whether a key admits one more request now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// It does.
    Go,
    /// Not before this moment, such as a delay's or a backoff's end, for
    /// the reason it gives.
    Until(SystemTime, Hold),
    /// Not before this moment, the time a site asked for, further away than
    /// the run waits: the request is not sent, and ends (ADR 0118).
    Refused(SystemTime),
    /// Not until a request in flight under it ends.
    Busy,
}

/// Why a key admits nothing before a moment (ADR 0102's wait reasons).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    /// Its last request started less than the delay between starts ago.
    Delay,
    /// It backs off, or waits for the time a site asked for.
    BackOff,
}

/// How a request that held a Permit ended, for its key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// Its key may go on.
    Done,
    /// Its response asked its key to back off, and to wait until the
    /// moment its `Retry-After` names, if any, when that is later.
    BackOff(Option<SystemTime>),
    /// Its response asked its key to wait until this moment, such as a
    /// redirect's `Retry-After`.
    Until(SystemTime),
    /// The site answered that too many requests came, a 429, with the
    /// moment its `Retry-After` names, if any.
    TooManyRequests(Option<SystemTime>),
}

/// The plain values a key keeps in the file, such as a domain key's limits
/// and pacing: what a rule reads and changes, which the storage loads
/// before and saves after, in the transaction that takes or gives back a
/// Permit, or that [`Politeness::change`] runs.
pub trait KeyState: Sized + Send + 'static {
    /// What it is kept under, such as a domain key.
    type Key: ?Sized + Send + Sync + 'static;
    /// Which of its values a change reads; the others load as if never
    /// written.
    type Reads: Copy + Send + 'static;
    /// Which of its values a change wrote, the only ones saved.
    type Writes: Send + 'static;

    /// The values `reads` names, kept under `key` in `connection`.
    ///
    /// # Errors
    ///
    /// When its tables cannot be read.
    fn load(connection: &Connection, key: &Self::Key, reads: Self::Reads)
    -> rusqlite::Result<Self>;

    /// Saves under `key` in `connection` the values `writes` names.
    ///
    /// # Errors
    ///
    /// When its tables cannot be written.
    fn save(
        &self,
        connection: &Connection,
        key: &Self::Key,
        writes: &Self::Writes,
    ) -> rusqlite::Result<()>;

    /// The values `reads` names, kept under `key` in `tables`, the tables
    /// of a Run's own Politeness: as [`load`](KeyState::load) reads them
    /// from a file.
    fn load_from(tables: &MemoryTables, key: &Self::Key, reads: Self::Reads) -> Self;

    /// Saves under `key` in `tables` the values `writes` names, as
    /// [`save`](KeyState::save) writes them to a file.
    fn save_to(&self, tables: &mut MemoryTables, key: &Self::Key, writes: &Self::Writes);
}

/// Loads the state kept under `key` in `connection` as `reads` says, runs
/// `change` on it and saves what it wrote: the work of one
/// [`Politeness::change`], in the caller's transaction.
fn changed<S: KeyState, T>(
    connection: &Connection,
    key: &S::Key,
    reads: S::Reads,
    change: impl FnOnce(&mut S) -> (T, S::Writes),
) -> rusqlite::Result<T> {
    let mut state = S::load(connection, key, reads)?;
    let (answer, writes) = change(&mut state);
    state.save(connection, key, &writes)?;
    Ok(answer)
}

/// The step of a Permit's life a rule reads its key's state for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Its take: the admission, again after a reclaim, and the start.
    Take,
    /// Its give back, after its request ended.
    GiveBack,
}

/// What a key admits, which a Product's composition passes in: a pure
/// function of its key's state, which the storage loads before and saves
/// after, in the transaction that takes or gives back a Permit.
pub trait PermitRule: Send + Sync + 'static {
    /// The values it keeps under each key.
    type State: KeyState<Key = str>;

    /// Which of its key's values `step` reads.
    fn reads(&self, step: Step) -> <Self::State as KeyState>::Reads;

    /// Whether a key whose state is `state`, with `in_flight` requests
    /// under it across every sharing run, admits one more at `now`.
    fn admit(&self, state: &Self::State, in_flight: u64, now: SystemTime) -> Verdict;

    /// A request under the key started at `now`; answers what it wrote.
    fn started(
        &self,
        state: &mut Self::State,
        now: SystemTime,
    ) -> <Self::State as KeyState>::Writes;

    /// A request under the key ended at `now` as `ending` says, `in_flight`
    /// others still in flight; answers when the key may go again, if later
    /// than `now` and the run waits for it (`None` also when its requests
    /// are refused until then, [`Verdict::Refused`]), and what it wrote.
    fn ended(
        &self,
        state: &mut Self::State,
        in_flight: u64,
        ending: Ending,
        now: SystemTime,
    ) -> (Option<SystemTime>, <Self::State as KeyState>::Writes);

    /// The moment a crawl request granted at `now` is to be sent, on a
    /// Politeness held in memory (#2171): its send's own waits are kept
    /// and taken in `state` as if it went then, and what that wrote is
    /// answered beside it. Its page's task waits until that moment, sends,
    /// and settles the true instant without waiting again; the Politeness
    /// keeps the later of the two. `None` by default: the rule reserves
    /// nothing, and the send asks the Politeness as it does the file.
    fn reserved(
        &self,
        _state: &mut Self::State,
        _now: SystemTime,
    ) -> Option<(SystemTime, <Self::State as KeyState>::Writes)> {
        None
    }

    /// Whether it may hold a request back on its own settings: `false`
    /// for a rule that neither caps, spaces nor rates its keys, under
    /// which a Run consults no Politeness store and never opens one for
    /// its admission, its gate backing its keys off itself (ADR 0138).
    fn paces(&self) -> bool {
        true
    }
}

/// A [`PermitRule`] any task may clone and hand to a take or a give back,
/// whatever its state.
#[derive(Clone)]
pub struct SharedRule(Arc<dyn Erased>);

impl SharedRule {
    /// Shares `rule`.
    pub fn new(rule: impl PermitRule) -> Self {
        Self(Arc::new(rule))
    }

    /// [`PermitRule::paces`].
    pub fn paces(&self) -> bool {
        self.0.paces()
    }
}

impl fmt::Debug for SharedRule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedRule")
            .field("paces", &self.paces())
            .finish_non_exhaustive()
    }
}

/// A [`PermitRule`] with its state's type erased, run in the transaction
/// that takes or gives back a Permit.
trait Erased: Send + Sync {
    fn paces(&self) -> bool;

    fn taken(
        &self,
        transaction: &Connection,
        context: &Context,
        key: &str,
        holder: &Holder,
        now: SystemTime,
        in_turn: bool,
    ) -> rusqlite::Result<(Taken, Option<Turn>)>;

    fn given_back(
        &self,
        transaction: &Connection,
        permit: &Permit,
        ending: Ending,
        now: SystemTime,
    ) -> rusqlite::Result<(Option<SystemTime>, Option<Turn>)>;

    fn taken_in_memory(
        &self,
        store: &mut memory::Store,
        key: &str,
        holder: &Holder,
        now: SystemTime,
        in_turn: bool,
    ) -> (Taken, Option<Turn>);

    fn given_back_in_memory(
        &self,
        store: &mut memory::Store,
        permit: &Permit,
        ending: Ending,
        now: SystemTime,
    ) -> (Option<SystemTime>, Option<Turn>);
}

impl<R: PermitRule> Erased for R {
    fn paces(&self) -> bool {
        PermitRule::paces(self)
    }

    fn taken(
        &self,
        transaction: &Connection,
        context: &Context,
        key: &str,
        holder: &Holder,
        now: SystemTime,
        in_turn: bool,
    ) -> rusqlite::Result<(Taken, Option<Turn>)> {
        taken_in(transaction, context, key, holder, self, now, in_turn)
    }

    fn given_back(
        &self,
        transaction: &Connection,
        permit: &Permit,
        ending: Ending,
        now: SystemTime,
    ) -> rusqlite::Result<(Option<SystemTime>, Option<Turn>)> {
        given_back_in(transaction, permit, self, ending, now)
    }

    fn taken_in_memory(
        &self,
        store: &mut memory::Store,
        key: &str,
        holder: &Holder,
        now: SystemTime,
        in_turn: bool,
    ) -> (Taken, Option<Turn>) {
        store.taken(key, holder, self, now, in_turn)
    }

    fn given_back_in_memory(
        &self,
        store: &mut memory::Store,
        permit: &Permit,
        ending: Ending,
        now: SystemTime,
    ) -> (Option<SystemTime>, Option<Turn>) {
        store.given_back(permit, self, ending, now)
    }
}

/// The Run that holds a Permit, under its run epoch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    /// Its Run ID.
    pub run_id: String,
    /// The run epoch it writes under.
    pub epoch: u64,
    /// Its folder, whose writer lock tells whether it is live.
    pub run_folder: PathBuf,
}

/// A request's Permit, held while it is in flight.
#[derive(Debug, PartialEq, Eq)]
pub struct Permit {
    id: i64,
    key: String,
    /// The moment its request is to be sent, which its grant reserved
    /// ([`PermitRule::reserved`]).
    send_at: Option<SystemTime>,
}

impl Permit {
    /// The key it counts under.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The moment its request is to be sent, when its grant reserved one,
    /// on a Politeness held in memory ([`PermitRule::reserved`]).
    pub fn send_at(&self) -> Option<SystemTime> {
        self.send_at
    }
}

/// What a take answered.
#[derive(Debug, PartialEq, Eq)]
pub enum Taken {
    /// The request may go, holding this Permit.
    Granted(Permit),
    /// Not now: try again at this moment, for the reason given, or, when
    /// `None`, once woken, since requests under the key are in flight.
    Wait(Option<(SystemTime, Hold)>),
    /// Not before this moment, further away than the run waits: the request
    /// is not sent (see [`Verdict::Refused`]).
    Refused(SystemTime),
}

/// A wake a waiter of this engine received: its Run may try `key` again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wake {
    /// The waiting Run's ID.
    pub run_id: String,
    /// The key it waits under.
    pub key: String,
}

/// Whether the Run in `folder` that took Permits under `epoch` is gone: its
/// writer lock free, or held at a later epoch. It blocks.
pub type HolderGone = fn(&Path, u64) -> bool;

/// Why the Politeness file did not open, or a take or a give back failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PolitenessError {
    /// SQLite failed; the transaction changed nothing.
    #[error("the Politeness file at `{path}`: {message}")]
    Sqlite {
        /// The file.
        path: PathBuf,
        /// SQLite's message.
        message: String,
    },
    /// The file is of a format version this engine cannot read (ADR 0066).
    #[error("the Politeness file is politeness-file {found}, which this engine cannot read")]
    Format {
        /// The file's version.
        found: String,
        /// What this engine makes of it.
        compatibility: Compatibility,
    },
    /// The OS refused the wake endpoint or its folder.
    #[error("the Politeness file's wake endpoint: {0}")]
    WakeEndpoint(String),
    /// The disk was full or failed an I/O at a transaction (ADR 0122): it
    /// was rolled back, and nothing of it is acknowledged.
    #[error("the Politeness file at `{path}` could not be written to its storage: {message}")]
    Storage {
        /// The file.
        path: PathBuf,
        /// SQLite's message.
        message: String,
    },
    /// The task that holds the Politeness, the file's actor or a Run's
    /// admission, has stopped.
    #[error("the task that holds the Politeness has stopped")]
    Closed,
}

/// What a job sees beside the file.
struct Context {
    path: PathBuf,
    engine: String,
    endpoint: Option<String>,
    holder_gone: HolderGone,
    /// Where a Run's own Politeness wakes its waiters, all within the
    /// engine; none for the file, whose waiters [`Politeness::wake_turn`]
    /// wakes at their endpoints.
    wakes: Option<broadcast::Sender<Wake>>,
}

impl Context {
    /// Wakes the waiter whose turn it is, of a Run's own Politeness.
    fn wake(&self, turn: Option<Turn>) {
        if let (Some(turn), Some(wakes)) = (turn, &self.wakes) {
            let _ = wakes.send(turn.wake);
        }
    }
}

/// What a Politeness keeps its Permits, waiters and contributions' values
/// in: the per-user file, or a Run's own store in memory.
enum Backend {
    File(Connection),
    Memory(memory::Store),
}

type Job = Box<dyn FnOnce(&mut Backend, &Context) + Send>;

/// A waiter whose turn it is, to wake.
#[derive(Debug)]
struct Turn {
    endpoint: String,
    wake: Wake,
    generation: i64,
    now: SystemTime,
}

/// The engine's handle on a Politeness, a file or a Run's own held in
/// memory: a handle to the task that holds it, which any task may clone.
#[derive(Clone)]
pub struct Politeness(Arc<Inner>);

struct Inner {
    mailbox: mpsc::Sender<Job>,
    wakes: broadcast::Sender<Wake>,
    endpoint: Option<String>,
    _wake_folder: Option<tempfile::TempDir>,
    /// The first transaction that met a full disk or an I/O error, as
    /// SQLite named it, once one did (ADR 0122).
    failures: watch::Sender<Option<String>>,
    work: CountedWork,
    /// Whether it is a Politeness of one Run's own, held in memory.
    in_memory: bool,
}

impl fmt::Debug for Politeness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Politeness")
            .field("endpoint", &self.0.endpoint)
            .finish_non_exhaustive()
    }
}

impl Politeness {
    /// Opens the per-user Politeness file at `path`, creating it and the
    /// tables of `tables` when it does not exist, as the engine `engine`
    /// (an ID no other engine of the user holds), with a wake endpoint of
    /// its own; its actor and the endpoint's listener are `owner`'s, and
    /// stop once every handle is dropped and `owner` is cancelled.
    /// `holder_gone` tells a Run whose Permits may be reclaimed.
    ///
    /// # Errors
    ///
    /// `Sqlite` when the file cannot be opened, its tables created or its
    /// statements prepared,
    /// `Format` when it is of a format this engine cannot read,
    /// `WakeEndpoint` when the OS refuses the endpoint.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the file opened closes it again.
    pub async fn open(
        path: PathBuf,
        options: &PolitenessOptions,
        tables: Vec<PolitenessTables>,
        holder_gone: HolderGone,
        engine: String,
        owner: &TaskOwner,
    ) -> Result<Self, PolitenessError> {
        let vfs = options.vfs.clone();
        let opened = path.clone();
        let work = options.work.clone();
        let (connection, folder, (mut listener, endpoint)) = owner
            .blocking(move || -> Result<_, PolitenessError> {
                let connection = open_file(&opened, vfs.as_deref(), &work, &tables)?;
                let endpoint =
                    |error: std::io::Error| PolitenessError::WakeEndpoint(error.to_string());
                let folder =
                    transport::socket_folder("goodfetch-wake-", WAKE_SOCKET).map_err(endpoint)?;
                let listening =
                    transport::listen_wakes(&folder.path().join(WAKE_SOCKET)).map_err(endpoint)?;
                Ok((connection, folder, listening))
            })
            .await?;
        let (wakes, _) = broadcast::channel(WAKES);
        let woken = wakes.clone();
        let cancelled = owner.token().clone();
        owner.spawn(async move {
            loop {
                tokio::select! {
                    () = cancelled.cancelled() => break,
                    message = listener.accept() => match message {
                        Ok(message) => {
                            if let Ok(wake) = serde_json::from_slice::<Wake>(&message) {
                                let _ = woken.send(wake);
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "the Politeness file's wake endpoint stopped; waiters fall back on their deadlines");
                            break;
                        }
                    },
                }
            }
        });
        let context = Context {
            path,
            engine,
            endpoint: Some(endpoint.clone()),
            holder_gone,
            wakes: None,
        };
        let (mailbox, jobs) = mpsc::channel::<Job>(MAILBOX);
        serve_on(owner, Backend::File(connection), context, jobs)?;
        Ok(Self::handle(
            mailbox,
            wakes,
            (Some(endpoint), Some(folder)),
            options.work.clone(),
            false,
        ))
    }

    /// A Politeness of one Run's own, in memory, for a Run with
    /// `politeness.sharing = run`: the same rules, shared with no other
    /// Run, kept as plain values without SQLite, so it opens nothing and
    /// runs no transaction; `options`' VFS is not used. It is held by the
    /// one task that serves it, [`OwnPoliteness`], with no actor of its
    /// own: the handle any task may clone sends that task its jobs (#2171).
    pub fn own(options: &PolitenessOptions) -> (Self, OwnPoliteness) {
        let (wakes, _) = broadcast::channel(WAKES);
        let context = Context {
            path: PathBuf::from(":memory:"),
            engine: String::new(),
            endpoint: Some(IN_MEMORY_ENDPOINT.to_owned()),
            holder_gone: |_, _| false,
            wakes: Some(wakes.clone()),
        };
        let (mailbox, jobs) = mpsc::channel::<Job>(MAILBOX);
        let handle = Self::handle(
            mailbox,
            wakes,
            (Some(IN_MEMORY_ENDPOINT.to_owned()), None),
            options.work.clone(),
            true,
        );
        let held = OwnPoliteness {
            backend: Backend::Memory(memory::Store::default()),
            context,
            jobs,
        };
        (handle, held)
    }

    /// A Politeness of one Run's own, in memory, as [`Politeness::own`]
    /// makes it, served by an actor of `owner`'s, for a Run whose driver
    /// holds no admission that would serve it.
    ///
    /// # Errors
    ///
    /// `Sqlite` when `owner` no longer starts tasks.
    pub fn in_memory(
        options: &PolitenessOptions,
        owner: &TaskOwner,
    ) -> Result<Self, PolitenessError> {
        let (politeness, held) = Self::own(options);
        held.serve_on(owner)?;
        Ok(politeness)
    }

    fn handle(
        mailbox: mpsc::Sender<Job>,
        wakes: broadcast::Sender<Wake>,
        (endpoint, wake_folder): (Option<String>, Option<tempfile::TempDir>),
        work: CountedWork,
        in_memory: bool,
    ) -> Self {
        Self(Arc::new(Inner {
            mailbox,
            wakes,
            endpoint,
            _wake_folder: wake_folder,
            failures: watch::Sender::new(None),
            work,
            in_memory,
        }))
    }

    /// Whether it is a Run's own, held in memory, whose
    /// [`notes`](Self::note) its callers do not wait for (#2171); on the
    /// file a note is a transaction its caller waits for.
    pub fn held_in_memory(&self) -> bool {
        self.0.in_memory
    }

    /// Where the file counts its work, and what keeps its tables counts
    /// theirs (ADR 0140's E6).
    pub fn work(&self) -> &CountedWork {
        &self.0.work
    }

    /// The first transaction of the file that met a full disk or an I/O
    /// error, once one did (ADR 0122): the Runs that take its Permits stop
    /// on it.
    pub fn storage_failures(&self) -> watch::Receiver<Option<String>> {
        self.0.failures.subscribe()
    }

    /// The wakes this engine's waiters receive, from the moment it is
    /// called.
    pub fn wakes(&self) -> broadcast::Receiver<Wake> {
        self.0.wakes.subscribe()
    }

    /// Takes a Permit under `key` for `holder` at `now`, in one
    /// transaction, if `rule` admits one more request with the count in
    /// flight under `key` across every sharing run; else answers when to
    /// try again, registering `holder` as a waiter of `key` when only a
    /// request's end can make room. A refusal for want of room first
    /// reclaims the Permits of Runs that are gone. A waiter woken but still
    /// refused passes the wake to the next waiter.
    ///
    /// # Errors
    ///
    /// `Sqlite` when the transaction failed, `Closed` when the task that
    /// holds it stopped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future does not withdraw a take already queued: it may
    /// still take a Permit, which the holder's Run gives back when it ends,
    /// or another engine reclaims.
    pub async fn take(
        &self,
        key: &str,
        holder: &Holder,
        rule: SharedRule,
        now: SystemTime,
    ) -> Result<Taken, PolitenessError> {
        let key = key.to_owned();
        let holder = holder.clone();
        let (taken, turn) = self
            .run(move |backend, context| take_in(backend, context, &key, &holder, &rule, now, true))
            .await?;
        if let Some(turn) = turn {
            self.wake_turn(turn).await;
        }
        Ok(taken)
    }

    /// Takes a Permit under `key` for `holder` at `now` as [`take`] does,
    /// for a request beside one `holder` already sent, such as a hedge's
    /// second copy (ADRs 0040 and 0086): it never waits in turn, so a
    /// refusal for want of room registers no waiter, and a grant leaves
    /// `holder`'s place among the key's waiters as it was.
    ///
    /// [`take`]: Politeness::take
    ///
    /// # Errors
    ///
    /// As [`Politeness::take`].
    ///
    /// # Cancel safety
    ///
    /// As [`Politeness::take`].
    pub async fn take_beside(
        &self,
        key: &str,
        holder: &Holder,
        rule: SharedRule,
        now: SystemTime,
    ) -> Result<Taken, PolitenessError> {
        let key = key.to_owned();
        let holder = holder.clone();
        // Its round trip counts on `F3.second_copy_round_trips`, where the
        // second copy takes it.
        let (taken, _) = self
            .answered(move |backend, context| {
                take_in(backend, context, &key, &holder, &rule, now, false)
            })
            .await?;
        Ok(taken)
    }

    /// Gives `permit` back at `now` after its request ended as `ending`,
    /// in one transaction with what `rule` makes of that ending, then
    /// wakes the waiter of its key whose turn it is. Answers when its key
    /// may go again, if later than `now`.
    ///
    /// # Errors
    ///
    /// `Sqlite` when the transaction failed, `Closed` when the task that
    /// holds it stopped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future does not withdraw a give back already queued;
    /// a waiter it would have woken waits for its own deadline.
    pub async fn give_back(
        &self,
        permit: Permit,
        rule: SharedRule,
        ending: Ending,
        now: SystemTime,
    ) -> Result<Option<SystemTime>, PolitenessError> {
        let (ready_at, turn) = self
            .run(move |backend, context| {
                give_back_in(backend, context, &permit, &rule, ending, now)
            })
            .await?;
        if let Some(turn) = turn {
            self.wake_turn(turn).await;
        }
        Ok(ready_at)
    }

    /// Withdraws `holder` as a waiter of every key, as its Run stops.
    ///
    /// # Errors
    ///
    /// As [`Politeness::take`].
    ///
    /// # Cancel safety
    ///
    /// Dropping the future does not withdraw it once queued.
    pub async fn stop_waiting(&self, holder: &Holder) -> Result<(), PolitenessError> {
        let run_id = holder.run_id.clone();
        self.run(move |backend, context| match backend {
            Backend::File(connection) => connection
                .prepare_cached(STOP_WAITING)
                .and_then(|mut cached| {
                    cached.execute((&run_id, context.endpoint.as_deref().unwrap_or_default()))
                })
                .map(drop)
                .map_err(|error| sqlite_error(&context.path, &error)),
            Backend::Memory(store) => {
                store.stop_waiting(&run_id);
                Ok(())
            }
        })
        .await
    }

    /// Changes the state `S` keeps under `key` in one transaction,
    /// committed when it succeeds and rolled back otherwise: loads what
    /// `reads` names, runs `change` on it and saves what it answers it
    /// wrote, such as a domain key's pacing or Foxhound's
    /// shared robots.txt copy (ADR 0086). Answers what `change` answered.
    ///
    /// # Errors
    ///
    /// `Sqlite` or `Storage` when the transaction failed, `Closed` when the
    /// task that holds it stopped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future does not withdraw a transaction already queued.
    pub async fn change<S, K, T, F>(
        &self,
        key: K,
        reads: S::Reads,
        change: F,
    ) -> Result<T, PolitenessError>
    where
        S: KeyState,
        K: Borrow<S::Key> + Send + 'static,
        T: Send + 'static,
        F: FnOnce(&mut S) -> (T, S::Writes) + Send + 'static,
    {
        self.run(change_job(key, reads, change)).await
    }

    /// Changes the state `S` keeps under `key` as [`Politeness::change`]
    /// does, for a read its caller counts on a row of its own, such as a
    /// robots.txt copy's on `E6.robots_reads`: on a Run's own Politeness
    /// its round trip does not count on `F3.politeness_read_round_trips`
    /// (#2171).
    ///
    /// # Errors
    ///
    /// As [`Politeness::change`].
    ///
    /// # Cancel safety
    ///
    /// As [`Politeness::change`].
    pub async fn change_counted_apart<S, K, T, F>(
        &self,
        key: K,
        reads: S::Reads,
        change: F,
    ) -> Result<T, PolitenessError>
    where
        S: KeyState,
        K: Borrow<S::Key> + Send + 'static,
        T: Send + 'static,
        F: FnOnce(&mut S) -> (T, S::Writes) + Send + 'static,
    {
        self.answered(change_job(key, reads, change)).await
    }

    /// Changes the state `S` keeps under `key` as [`Politeness::change`]
    /// does, for a write whose caller needs no answer: on a Run's own
    /// Politeness it is a one-way message, queued ahead of any take or
    /// change queued after it and run by the task that holds it, where it
    /// cannot fail once queued, so the caller never waits on that task
    /// (#2171); on the file it is [`Politeness::change`], awaited.
    ///
    /// # Errors
    ///
    /// `Closed` when the task that holds it stopped, so the change was not
    /// queued; on the file, as
    /// [`Politeness::change`].
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before the change is queued drops the change.
    pub async fn note<S, K, F>(
        &self,
        key: K,
        reads: S::Reads,
        change: F,
    ) -> Result<(), PolitenessError>
    where
        S: KeyState,
        K: Borrow<S::Key> + Send + 'static,
        F: FnOnce(&mut S) -> S::Writes + Send + 'static,
    {
        let change = move |state: &mut S| ((), change(state));
        if !self.0.in_memory {
            return self.change(key, reads, change).await;
        }
        self.send(Box::new(move |backend, _| {
            memory_of(backend).change(key.borrow(), reads, change);
        }))
        .await
    }

    /// Gives `permit` back at `now` as [`Politeness::give_back`] does, for
    /// a caller that needs no answer, such as a second copy's: on a Run's
    /// own Politeness a one-way message that wakes the waiter whose turn it
    /// is where it runs, on the file [`Politeness::give_back`], awaited.
    ///
    /// # Errors
    ///
    /// As [`Politeness::note`].
    ///
    /// # Cancel safety
    ///
    /// As [`Politeness::note`].
    pub async fn note_given_back(
        &self,
        permit: Permit,
        rule: SharedRule,
        ending: Ending,
        now: SystemTime,
    ) -> Result<(), PolitenessError> {
        if !self.0.in_memory {
            return self.give_back(permit, rule, ending, now).await.map(drop);
        }
        self.send(Box::new(move |backend, context| {
            let (_, turn) = rule
                .0
                .given_back_in_memory(memory_of(backend), &permit, ending, now);
            context.wake(turn);
        }))
        .await
    }

    /// Queues `job`, answering nothing.
    async fn send(&self, job: Job) -> Result<(), PolitenessError> {
        self.0
            .mailbox
            .send(job)
            .await
            .map_err(|_| PolitenessError::Closed)
    }

    /// Runs `work` on the task that holds the Politeness and answers what
    /// it returned: on a Run's own, a round trip that counts on
    /// `F3.politeness_read_round_trips` (#2171).
    async fn run<T, F>(&self, work: F) -> Result<T, PolitenessError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Backend, &Context) -> Result<T, PolitenessError> + Send + 'static,
    {
        if self.0.in_memory {
            self.0.work.count(Row::PolitenessReadRoundTrips);
        }
        self.answered(work).await
    }

    /// Runs `work` on the task that holds the Politeness and answers what
    /// it returned, its round trip counted by its caller.
    async fn answered<T, F>(&self, work: F) -> Result<T, PolitenessError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Backend, &Context) -> Result<T, PolitenessError> + Send + 'static,
    {
        let (answer, answered) = oneshot::channel();
        let job: Job = Box::new(move |backend, context| {
            let _ = answer.send(work(backend, context));
        });
        self.send(job).await?;
        let answer = answered.await.map_err(|_| PolitenessError::Closed)?;
        if let Err(PolitenessError::Storage { message, .. }) = &answer {
            tracing::warn!(%message, "a transaction of the Politeness file met a full disk or an I/O error and was rolled back");
            self.0.failures.send_if_modified(|failure| {
                let first = failure.is_none();
                if first {
                    *failure = Some(message.clone());
                }
                first
            });
        }
        answer
    }

    /// Wakes the waiter whose turn it is; one whose endpoint is gone loses
    /// its rows, and the next waiter's turn comes.
    async fn wake_turn(&self, mut turn: Turn) {
        loop {
            if self.0.endpoint.as_deref() == Some(turn.endpoint.as_str()) {
                let _ = self.0.wakes.send(turn.wake);
                return;
            }
            let message = serde_json::to_vec(&turn.wake).expect("invariant: a wake encodes");
            match transport::wake(&turn.endpoint, &message).await {
                Ok(()) => return,
                Err(error) if transport::is_gone(&error) => {
                    let gone = turn.endpoint.clone();
                    let key = turn.wake.key.clone();
                    let generation = turn.generation;
                    let now = turn.now;
                    let next = self
                        .run(move |backend, context| {
                            // A Run's own store wakes its waiters within the
                            // engine: none of its endpoints is ever gone.
                            let Backend::File(connection) = backend else {
                                return Ok(None);
                            };
                            connection
                                .prepare_cached(GONE_WAITERS)
                                .and_then(|mut cached| cached.execute([&gone]))
                                .map_err(|error| sqlite_error(&context.path, &error))?;
                            next_turn(connection, &key, generation, now)
                                .map_err(|error| sqlite_error(&context.path, &error))
                        })
                        .await;
                    match next {
                        Ok(Some(next)) => turn = next,
                        Ok(None) => return,
                        Err(error) => {
                            tracing::warn!(%error, "a gone waiter's rows were not removed");
                            return;
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "a waiter of the Politeness file was not woken; it waits for its own deadline");
                    return;
                }
            }
        }
    }
}

/// The job that changes the state `S` keeps under `key`, in one
/// transaction on the file ([`Politeness::change`]).
fn change_job<S, K, T, F>(
    key: K,
    reads: S::Reads,
    change: F,
) -> impl FnOnce(&mut Backend, &Context) -> Result<T, PolitenessError> + Send + 'static
where
    S: KeyState,
    K: Borrow<S::Key> + Send + 'static,
    T: Send + 'static,
    F: FnOnce(&mut S) -> (T, S::Writes) + Send + 'static,
{
    move |backend, context| match backend {
        Backend::File(connection) => sqlite_store::immediate(connection, |transaction| {
            changed(transaction, key.borrow(), reads, change)
        })
        .map_err(|error| sqlite_error(&context.path, &error)),
        Backend::Memory(store) => Ok(store.change(key.borrow(), reads, change)),
    }
}

/// Milliseconds of `wall` since the Unix epoch.
pub fn milliseconds(wall: SystemTime) -> i64 {
    wall.duration_since(UNIX_EPOCH).map_or(0, |since| {
        i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
    })
}

/// The moment `milliseconds` after the Unix epoch.
pub fn moment(milliseconds: i64) -> SystemTime {
    UNIX_EPOCH + std::time::Duration::from_millis(u64::try_from(milliseconds).unwrap_or_default())
}

/// The failure of a transaction of the file at `path` on `error`: a full
/// disk or an I/O error as such, after which SQLite may have rolled the
/// whole transaction back, and [`sqlite_store::immediate`] rolls back the
/// rest (ADR 0122).
fn sqlite_error(path: &Path, error: &(dyn std::error::Error + 'static)) -> PolitenessError {
    let storage = error
        .downcast_ref::<rusqlite::Error>()
        .is_some_and(sqlite_store::is_storage);
    if storage {
        PolitenessError::Storage {
            path: path.to_owned(),
            message: error.to_string(),
        }
    } else {
        PolitenessError::Sqlite {
            path: path.to_owned(),
            message: error.to_string(),
        }
    }
}

/// Opens the file, skips `fsync`, creates its tables and prepares its
/// statements.
fn open_file(
    path: &Path,
    vfs: Option<&str>,
    work: &CountedWork,
    tables: &[PolitenessTables],
) -> Result<Connection, PolitenessError> {
    let statements = statements(tables);
    File::new(path, vfs)
        .open(&Opening {
            folder: Folder::Make,
            access: Access::Create,
            wait: Wait::Busy(BUSY_TIMEOUT),
            durability: DURABILITY,
            counting: Some(Counting {
                work,
                opens: Some(Row::PolitenessOpens),
                transactions: Row::PolitenessTransactions,
                counts_switch: true,
            }),
            schema: Some(Schema {
                format: &POLITENESS_FILE,
                statements: &statements,
            }),
        })
        .map_err(|error| open_failed(path, error))
        .and_then(|connection| {
            prepare_statements(&connection, tables)
                .map(|()| connection)
                .map_err(|error| sqlite_error(path, &error))
        })
}

/// Prepares, in `connection`'s cache, the engine's statements and those
/// `tables` name, so no Permit prepares one (ADR 0140's `E6.opens`).
fn prepare_statements(
    connection: &Connection,
    tables: &[PolitenessTables],
) -> rusqlite::Result<()> {
    connection.set_prepared_statement_cache_capacity(STATEMENTS);
    let prepared: Vec<&str> = PREPARED
        .iter()
        .chain(tables.iter().flat_map(|table| table.prepared))
        .copied()
        .collect();
    sqlite_store::prepare_immediate(connection, &prepared)
}

/// The engine's own tables' statements, then each contribution's.
fn statements(tables: &[PolitenessTables]) -> Vec<&[&str]> {
    std::iter::once(SCHEMA)
        .chain(tables.iter().map(|table| table.statements))
        .collect()
}

/// The failure of opening the file at `path` on `error`.
fn open_failed(path: &Path, error: OpenError) -> PolitenessError {
    match error {
        OpenError::Format(refusal) => PolitenessError::Format {
            found: refusal.found,
            compatibility: refusal.compatibility,
        },
        OpenError::Unopened(error) | OpenError::Sqlite(error) => sqlite_error(path, &error),
        OpenError::Folder(error) => sqlite_error(path, &error),
    }
}

/// How many Permits are held under `key`.
fn in_flight(connection: &Connection, key: &str) -> rusqlite::Result<u64> {
    let count: i64 = connection
        .prepare_cached(IN_FLIGHT)?
        .query_row([key], |row| row.get(0))?;
    Ok(u64::try_from(count).unwrap_or_default())
}

/// How many Permits under `key` were given back so far: each give back
/// starts a new generation of wakes.
fn generation(connection: &Connection, key: &str) -> rusqlite::Result<i64> {
    Ok(connection
        .prepare_cached(GENERATION)?
        .query_row([key], |row| row.get(0))
        .optional()?
        .unwrap_or_default())
}

/// The first waiter of `key` in turn not yet woken for `generation`,
/// marked woken for it at `now`.
fn next_turn(
    connection: &Connection,
    key: &str,
    generation: i64,
    now: SystemTime,
) -> rusqlite::Result<Option<Turn>> {
    let next: Option<(String, String)> = connection
        .prepare_cached(NEXT_WAITER)?
        .query_row((key, generation), |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?;
    let Some((run_id, endpoint)) = next else {
        return Ok(None);
    };
    connection.prepare_cached(WAKE_WAITER)?.execute((
        key,
        &run_id,
        generation,
        milliseconds(now),
    ))?;
    Ok(Some(Turn {
        endpoint,
        wake: Wake {
            run_id,
            key: key.to_owned(),
        },
        generation,
        now,
    }))
}

/// How many requests `run_id` counts as in flight under `key` at `now`:
/// the Permits held, and the room kept for another run's waiter woken
/// within [`WAKE_GRACE`] for the latest give back.
fn taken_room(
    connection: &Connection,
    key: &str,
    run_id: &str,
    now: SystemTime,
) -> rusqlite::Result<u64> {
    let generation = generation(connection, key)?;
    let kept: i64 = connection.prepare_cached(KEPT_ROOM)?.query_row(
        (
            key,
            run_id,
            generation,
            milliseconds(now).saturating_sub(i64::try_from(WAKE_GRACE.as_millis()).unwrap_or(0)),
        ),
        |row| row.get(0),
    )?;
    Ok(in_flight(connection, key)? + u64::try_from(kept).unwrap_or_default())
}

/// Deletes the Permits of every other engine's Run that is gone, and
/// answers how many.
fn reclaim(connection: &Connection, context: &Context, key: &str) -> rusqlite::Result<usize> {
    let holders: Vec<(String, i64, String)> = connection
        .prepare_cached(OTHER_HOLDERS)?
        .query_map((key, &context.engine), |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut reclaimed = 0;
    for (run_id, epoch, folder) in holders {
        if (context.holder_gone)(Path::new(&folder), u64::try_from(epoch).unwrap_or_default()) {
            tracing::debug!(%run_id, epoch, "the Permits of a gone Run are reclaimed");
            reclaimed += connection
                .prepare_cached(RECLAIM)?
                .execute((&run_id, epoch))?;
        }
    }
    Ok(reclaimed)
}

/// A Politeness of one Run's own, in memory ([`Politeness::own`]), held
/// by the one task that serves it, such as its crawl's loop through the
/// Run's admission: that task takes and gives back its Permits with no
/// message, and runs the jobs the Run's other tasks send it through their
/// handles ([`OwnPoliteness::serve`]), so the Run's Politeness has no actor
/// of its own (#2171).
pub struct OwnPoliteness {
    backend: Backend,
    context: Context,
    jobs: mpsc::Receiver<Job>,
}

impl fmt::Debug for OwnPoliteness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnPoliteness")
            .finish_non_exhaustive()
    }
}

impl OwnPoliteness {
    /// Takes a Permit under `key` for `holder` at `now` as
    /// [`Politeness::take`] does, after every job sent before it, and
    /// wakes the waiter whose turn it is.
    pub fn take(
        &mut self,
        key: &str,
        holder: &Holder,
        rule: &SharedRule,
        now: SystemTime,
    ) -> Taken {
        self.drain();
        let (taken, turn) =
            rule.0
                .taken_in_memory(memory_of(&mut self.backend), key, holder, now, true);
        self.context.wake(turn);
        taken
    }

    /// Gives `permit` back at `now` as [`Politeness::give_back`] does,
    /// after every job sent before it; answers when its key may go again,
    /// if later than `now`.
    pub fn give_back(
        &mut self,
        permit: &Permit,
        rule: &SharedRule,
        ending: Ending,
        now: SystemTime,
    ) -> Option<SystemTime> {
        self.drain();
        let (ready_at, turn) =
            rule.0
                .given_back_in_memory(memory_of(&mut self.backend), permit, ending, now);
        self.context.wake(turn);
        ready_at
    }

    /// Withdraws `holder` as a waiter of every key, as its Run stops.
    pub fn stop_waiting(&mut self, holder: &Holder) {
        self.drain();
        memory_of(&mut self.backend).stop_waiting(&holder.run_id);
    }

    /// Runs the next job a handle sent, and every one queued behind it;
    /// pending for ever once no handle is left.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no job.
    pub async fn serve(&mut self) {
        match self.jobs.recv().await {
            Some(job) => {
                job(&mut self.backend, &self.context);
                self.drain();
            }
            None => std::future::pending().await,
        }
    }

    /// Runs every job queued now, in the order they were sent.
    fn drain(&mut self) {
        while let Ok(job) = self.jobs.try_recv() {
            job(&mut self.backend, &self.context);
        }
    }

    /// Serves it from an actor of `owner`'s.
    ///
    /// # Errors
    ///
    /// `Sqlite` when `owner` no longer starts tasks.
    pub fn serve_on(self, owner: &TaskOwner) -> Result<(), PolitenessError> {
        serve_on(owner, self.backend, self.context, self.jobs)
    }
}

/// Serves `backend`'s jobs from an actor of `owner`'s.
fn serve_on(
    owner: &TaskOwner,
    mut backend: Backend,
    context: Context,
    jobs: mpsc::Receiver<Job>,
) -> Result<(), PolitenessError> {
    let failed = context.path.clone();
    owner
        .actor("politeness-file", jobs, move |group: Vec<Job>| {
            for job in group {
                job(&mut backend, &context);
            }
        })
        .map_err(|error| sqlite_error(&failed, &error))
}

/// The store of a Run's own Politeness, which only such a Politeness
/// sends jobs that read it.
fn memory_of(backend: &mut Backend) -> &mut memory::Store {
    match backend {
        Backend::Memory(store) => store,
        Backend::File(_) => {
            unreachable!("invariant: only a Run's own Politeness is held in memory")
        }
    }
}

fn take_in(
    backend: &mut Backend,
    context: &Context,
    key: &str,
    holder: &Holder,
    rule: &SharedRule,
    now: SystemTime,
    in_turn: bool,
) -> Result<(Taken, Option<Turn>), PolitenessError> {
    match backend {
        Backend::File(connection) => sqlite_store::immediate(connection, |transaction| {
            rule.0
                .taken(transaction, context, key, holder, now, in_turn)
        })
        .map_err(|error| sqlite_error(&context.path, &error)),
        Backend::Memory(store) => Ok(rule.0.taken_in_memory(store, key, holder, now, in_turn)),
    }
}

/// [`take_in`]'s work, in its transaction: the key's state is loaded once,
/// for both admissions and the start, and only what the start wrote is
/// saved.
fn taken_in<R: PermitRule>(
    transaction: &Connection,
    context: &Context,
    key: &str,
    holder: &Holder,
    rule: &R,
    now: SystemTime,
    in_turn: bool,
) -> rusqlite::Result<(Taken, Option<Turn>)> {
    let taken = |transaction: &Connection| taken_room(transaction, key, &holder.run_id, now);
    let in_flight = taken(transaction)?;
    let mut state = R::State::load(transaction, key, rule.reads(Step::Take))?;
    let mut verdict = rule.admit(&state, in_flight, now);
    if verdict == Verdict::Busy && reclaim(transaction, context, key)? > 0 {
        verdict = rule.admit(&state, taken(transaction)?, now);
    }
    let mut turn = None;
    let taken = match verdict {
        Verdict::Go => {
            transaction.prepare_cached(TAKE).and_then(|mut cached| {
                cached.execute((
                    key,
                    &holder.run_id,
                    i64::try_from(holder.epoch).unwrap_or(i64::MAX),
                    holder.run_folder.to_string_lossy(),
                    &context.engine,
                    milliseconds(now),
                ))
            })?;
            let id = transaction.last_insert_rowid();
            let writes = rule.started(&mut state, now);
            state.save(transaction, key, &writes)?;
            if in_turn {
                transaction
                    .prepare_cached(LEAVE_TURNS)
                    .and_then(|mut cached| cached.execute((key, &holder.run_id)))?;
            }
            Taken::Granted(Permit {
                id,
                key: key.to_owned(),
                send_at: None,
            })
        }
        Verdict::Until(until, hold) => Taken::Wait(Some((until, hold))),
        Verdict::Refused(until) => Taken::Refused(until),
        Verdict::Busy => {
            if in_turn && let Some(endpoint) = &context.endpoint {
                turn = wait_in_turn(transaction, key, &holder.run_id, endpoint, now)?;
            }
            Taken::Wait(None)
        }
    };
    Ok((taken, turn))
}

/// Registers `run_id` as a waiter of `key` at `endpoint`. A waiter already
/// woken for the latest give back, and still refused, goes to the back of
/// the turns and passes the wake to the next waiter, whose turn it answers.
fn wait_in_turn(
    connection: &Connection,
    key: &str,
    run_id: &str,
    endpoint: &str,
    now: SystemTime,
) -> rusqlite::Result<Option<Turn>> {
    let generation = generation(connection, key)?;
    let woken: Option<i64> = connection
        .prepare_cached(WOKEN)?
        .query_row((key, run_id), |row| row.get(0))
        .optional()?;
    match woken {
        None => {
            connection
                .prepare_cached(JOIN_TURNS)?
                .execute((key, run_id, endpoint, generation))?;
            Ok(None)
        }
        Some(woken) if woken == generation && generation > 0 => {
            connection
                .prepare_cached(REJOIN_TURNS)?
                .execute((key, run_id, endpoint))?;
            next_turn(connection, key, generation, now)
        }
        Some(_) => Ok(None),
    }
}

fn give_back_in(
    backend: &mut Backend,
    context: &Context,
    permit: &Permit,
    rule: &SharedRule,
    ending: Ending,
    now: SystemTime,
) -> Result<(Option<SystemTime>, Option<Turn>), PolitenessError> {
    match backend {
        Backend::File(connection) => sqlite_store::immediate(connection, |transaction| {
            rule.0.given_back(transaction, permit, ending, now)
        })
        .map_err(|error| sqlite_error(&context.path, &error)),
        Backend::Memory(store) => Ok(rule.0.given_back_in_memory(store, permit, ending, now)),
    }
}

/// [`give_back_in`]'s work, in its transaction.
fn given_back_in<R: PermitRule>(
    transaction: &Connection,
    permit: &Permit,
    rule: &R,
    ending: Ending,
    now: SystemTime,
) -> rusqlite::Result<(Option<SystemTime>, Option<Turn>)> {
    transaction
        .prepare_cached(GIVE_BACK)
        .and_then(|mut cached| cached.execute([permit.id]))?;
    let others = in_flight(transaction, &permit.key)?;
    let mut state = R::State::load(transaction, &permit.key, rule.reads(Step::GiveBack))?;
    let (ready_at, writes) = rule.ended(&mut state, others, ending, now);
    state.save(transaction, &permit.key, &writes)?;
    let generation: i64 = transaction
        .prepare_cached(NEXT_GENERATION)
        .and_then(|mut cached| cached.query_row([&permit.key], |row| row.get(0)))?;
    let turn = next_turn(transaction, &permit.key, generation, now)?;
    Ok((ready_at, turn))
}

#[cfg(test)]
mod tests;
