//! The model-based test of the crawl's Run state (ADR 0142, seam 2 of spec
//! #2138; ADR 0060; TESTING.md, Model-based tests and crashes in one
//! process): a crawl's work items in a run file on real SQLite through the
//! crash VFS, given work, leased and ended through the Run state as the
//! generated steps say. Seeds are added to the running crawl and the
//! failures re-admitted over the loop's mailbox, as another task asks
//! them; an attempt enqueues pages, which wait for its commit; the loop
//! keys the seeds its start wrote, a few at a time, in a sealed batch,
//! claims the next work items, refuses one its site asked not to send,
//! and asks once idle whether the crawl drained, which closes its intake.
//! An attempt commits, fails, or panics inside the engine, as a bug would
//! (#2285): what it staged goes with it, and it ends as a failed attempt
//! does, tried again while `crawl.retry.max` leaves it a retry, then
//! `failed` with `engine_panicked`. The crawl may stop cleanly, crash or
//! lose power, and then resumes. It crashes at each of the file-backed
//! Run state's crash points (ADR 0142): between its writes, inside a write
//! not yet synced, which a power cut tears, and inside a clean stop, once
//! its last write, the release of its leases, was handed to the file.
//!
//! A reference model of the work items says what each step answers and
//! leaves. After every step the run file holds the model's work items,
//! each page once whatever gave it again (the exact duplicate check), and
//! the requests by state the Run state counted in memory are the run
//! file's and the model's. A clean stop leaves exactly the model; after a
//! process crash the resumed crawl goes on from every write, each attempt
//! in flight counted as `handler_lost`; after a power cut the file the
//! next process opens passes `PRAGMA integrity_check` (in `after_crash`)
//! and holds the model as one of its writes left it, from the last synced
//! one on, and the resumed crawl, given every page again, adds each page
//! it does not hold once. Each case runs at a level it draws: `full`, each
//! write in the file as it is answered, or `batch` with no threshold, where
//! a claim, a key and an attempt's end ride the open batch, which a reader
//! sees only once a barrier, a seal or the writer's end writes it, and
//! which a crash takes whole; the loop's reads leave out what it claimed
//! in a batch they do not see. 16 cases in the Quick suite and 256 in the
//! Pre-merge suite (`proptest_config`).
#![expect(
    clippy::disallowed_methods,
    reason = "the model reads the test Clock and the seeded Entropy (ADR 0059)"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_counted_work::CountedWork;
use goodfetch_engine::admission::Keys;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::mask::Masks;
use goodfetch_engine::run_state::{Durability, RunStateOptions, RunStateTables, Syncing};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::foxhound::crawl::{RequestSeed, Seed};
use goodfetch_protocol::foxhound::frontier::{
    CrawlRequestEnqueueParams, Enqueued, HttpMethod, NotAddedReason,
};
use goodfetch_protocol::foxhound::outcomes::OutcomeKind;
use goodfetch_protocol::run::durability::BatchThreshold;
use goodfetch_protocol::run::{PointInTime, RunId};
use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv, after_crash};
use proptest::prelude::*;
use proptest::sample::Index;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use rusqlite::Connection;
use tokio::runtime::Runtime;

use crate::attempt_outcome::{Ended, Ending, Outcome};
use crate::intake::Intake;
use crate::invariants::fixture;
use crate::lease::{CommitError, Leases, WorkItemLease};
use crate::policy::CrawlPolicy;
use crate::run_state::admission::Enqueue;
use crate::run_state::{
    Asked, Asking, RunState, StateError, Stored, Tally, Unkeyed, Within, mailbox,
};
use crate::staging::Staging;
use crate::tier_choice::TierChoices;
use crate::work_items::{self, Changed, Failure, Leased, Unsuccessful};

/// How many pages a case's crawl may be given: few, so that most are
/// given more than once.
const PAGES: usize = 6;

/// The crawl's policy: a work item is tried again once.
const POLICY: CrawlPolicy = CrawlPolicy {
    handler_timeout: Duration::from_mins(1),
    retry_max: 1,
    retry_handler_failed: false,
    retry_engine_panicked: true,
    handler_concurrency: 16,
};

/// How many unkeyed work items the loop keys in one sealed batch.
const KEYED_AT_ONCE: usize = 2;

/// The tables of a crawl's run file.
const TABLES: [RunStateTables; 12] = [
    work_items::TABLES,
    crate::labels::TABLES,
    crate::tier_choice::TABLES,
    goodfetch_dataset::TABLES,
    goodfetch_session_pool::TABLES,
    goodfetch_frontier::VISITED_TABLES,
    goodfetch_frontier::SCOPE_TABLES,
    goodfetch_frontier::BOUNDS_TABLES,
    goodfetch_frontier::RULES_TABLES,
    goodfetch_frontier::payload::TABLES,
    goodfetch_frontier::sitemap::TABLES,
    goodfetch_frontier::trap_links::TABLES,
];

/// The domain key every page waits under.
const KEY: &str = "shop.test";

/// How long an attempt's lease lasts: the model's clock never moves.
const LEASE: Duration = Duration::from_secs(70);

/// The Error codes the model's work items fail with.
const HANDLER_FAILED: &str = "handler_failed";
const ENGINE_PANICKED: &str = "engine_panicked";
const HANDLER_LOST: &str = "handler_lost";
const SITE_REFUSED: &str = "robots_txt_disallowed";

/// How an attempt ends.
#[derive(Clone, Copy, Debug)]
enum How {
    /// It commits, with what it staged.
    Commit,
    /// Its handler fails with a failure of its own, which is not retried.
    Fail,
    /// It panics inside the engine.
    Panic,
}

#[derive(Clone, Debug)]
enum Transition {
    /// These pages are given as seeds to the running crawl.
    Seed(Vec<usize>),
    /// The loop keys the next unkeyed work items, in a sealed batch.
    Seal,
    /// The loop claims this many of the next pending work items.
    Lease(usize),
    /// An attempt enqueues this page.
    Enqueue(Index, usize),
    /// An attempt ends.
    End(Index, How),
    /// The failures are re-admitted.
    Readmit,
    /// The next pending work item's site asked not to send it: it fails
    /// with no attempt.
    Refuse,
    /// The idle loop asks whether the crawl drained.
    Drain,
    /// The crawl stops cleanly, its leases released, and resumes; or,
    /// with a crash, its process crashes or the power goes inside the
    /// stop, once its last write, the release, was handed to the file.
    Stop(Option<CrashKind>),
    /// The process crashes or the power goes, and the crawl resumes. A
    /// power cut ends the case with what the resumed crawl holds.
    Crash(CrashKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Pending,
    Leased,
    Succeeded,
    Failed(&'static str),
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Succeeded => "succeeded",
            Self::Failed(_) => "failed",
        }
    }
}

/// One work item: its page, and its place among the work items, which is
/// the order the places were given in.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Item {
    page: usize,
    place: u64,
    status: Status,
    keyed: bool,
    attempts: u32,
    /// Its attempts as its latest admission found them.
    retried_from: u32,
    epoch: u64,
}

impl Item {
    fn added(page: usize, place: u64, keyed: bool) -> Self {
        Self {
            page,
            place,
            status: Status::Pending,
            keyed,
            attempts: 0,
            retried_from: 0,
            epoch: 0,
        }
    }

    /// Whether it has a retry left since its latest admission.
    fn may_retry(&self) -> bool {
        self.attempts - self.retried_from <= POLICY.retry_max
    }
}

/// One attempt: its work item's page, the pages it staged, each with the
/// place its enqueue reserved, and whether it ended.
#[derive(Clone, Debug)]
struct Attempt {
    page: usize,
    staged: Vec<(usize, u64)>,
    over: bool,
}

/// What a step answered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Answer {
    /// It asked nothing.
    #[default]
    Nothing,
    /// Whether each page given was added; `None` once the crawl's intake
    /// closed, or the attempt that gave it ended.
    Admitted(Option<Vec<bool>>),
    /// The pages keyed, in order.
    Keyed(Vec<usize>),
    /// The pages leased, in order.
    Leased(Vec<usize>),
    /// Whether the attempt's end ended its work item's lease.
    Ended(bool),
    /// How many failures were re-admitted; `None` once the intake closed.
    Readmitted(Option<u64>),
    /// Whether a work item was refused.
    Refused(bool),
    /// Whether the crawl's intake is closed; `None` while an attempt is in
    /// flight, when the loop does not ask.
    Drained(Option<bool>),
}

/// What the Run state must give: the work items, in the order of their
/// places, the attempts, in claim order, whether the intake is closed, and
/// what the last step answered.
#[derive(Clone, Debug)]
struct Model {
    items: Vec<Item>,
    next_place: u64,
    attempts: Vec<Attempt>,
    closed: bool,
    answered: Answer,
    /// The work items after each write since the last one that was synced,
    /// that one's first: what a power cut may leave.
    unsynced: Vec<Vec<Item>>,
    crashed: bool,
    /// Whether the Run state batches attempts (`batch`), or writes each
    /// write as it is answered (`full`).
    batching: bool,
    /// The work items as the last commit left them: what a reader sees.
    written: Vec<Item>,
    /// The pages claimed since the last commit, which the loop's reads
    /// leave out.
    claimed: Vec<usize>,
}

/// The work items as a crawl resumed after a crash finds them: each one
/// still leased, whose engine stopped without settling it, is tried again
/// while it has a retry left and fails with `handler_lost` otherwise.
fn recovered(mut items: Vec<Item>) -> Vec<Item> {
    for item in &mut items {
        if item.status == Status::Leased {
            item.status = if item.may_retry() {
                Status::Pending
            } else {
                Status::Failed(HANDLER_LOST)
            };
        }
    }
    items
}

/// The work items once the opening crawl keyed each of them.
fn keyed(mut items: Vec<Item>) -> Vec<Item> {
    for item in &mut items {
        item.keyed = true;
    }
    items
}

/// What the run file shows of each work item, in the order of their
/// places: its page, its status, whether it is keyed, its attempts and its
/// lease epoch.
type View = Vec<(usize, String, bool, u32, u64)>;

fn view_of(items: &[Item]) -> View {
    items
        .iter()
        .map(|item| {
            (
                item.page,
                item.status.as_str().to_owned(),
                item.keyed,
                item.attempts,
                item.epoch,
            )
        })
        .collect()
}

/// The requests by state: how many work items are left to deliver, and how
/// many requests ended `succeeded`, and `failed` under each Error code.
type Counts = (u64, u64, Vec<(String, u64)>);

fn counts_of(items: &[Item]) -> Counts {
    let count = |counted: &dyn Fn(&Item) -> bool| {
        u64::try_from(items.iter().filter(|item| counted(item)).count()).unwrap()
    };
    let mut failed = BTreeMap::<&str, u64>::new();
    for item in items {
        if let Status::Failed(code) = item.status {
            *failed.entry(code).or_default() += 1;
        }
    }
    (
        count(&|item| matches!(item.status, Status::Pending | Status::Leased)),
        count(&|item| item.status == Status::Succeeded),
        failed
            .into_iter()
            .map(|(code, count)| (code.to_owned(), count))
            .collect(),
    )
}

impl Model {
    fn holds(&self, page: usize) -> bool {
        self.items.iter().any(|item| item.page == page)
    }

    fn item(&mut self, page: usize) -> &mut Item {
        self.items
            .iter_mut()
            .find(|item| item.page == page)
            .expect("invariant: an attempt's work item is held")
    }

    /// Adds `page` at the next place, keyed as every admission of a
    /// running crawl keys what it adds; answers whether it was added.
    fn add(&mut self, page: usize) -> bool {
        if self.holds(page) {
            return false;
        }
        self.items.push(Item::added(page, self.next_place, true));
        self.next_place += 1;
        true
    }

    /// A write left the work items as they are now: `synced`, it is the
    /// oldest a power cut may leave. At `full` it commits as it is
    /// answered; at `batch` it rides the open batch, which a barrier
    /// ([`committed`](Self::committed)) commits.
    fn wrote(&mut self, synced: bool) {
        if self.batching {
            return;
        }
        if synced {
            self.unsynced.clear();
        }
        self.unsynced.push(self.items.clone());
        self.written.clone_from(&self.items);
        self.claimed.clear();
    }

    /// At `batch`, a barrier commits the open batch with its own write,
    /// synced: the file holds the work items as they are now.
    fn committed(&mut self) {
        if self.batching {
            self.unsynced = vec![self.items.clone()];
            self.written.clone_from(&self.items);
            self.claimed.clear();
        }
    }

    /// The loop's read of the next `count` work items that are pending and
    /// keyed, by page: the run file's, less each claimed since the last
    /// commit.
    fn waiting(&self, count: usize) -> Vec<usize> {
        self.written
            .iter()
            .filter(|item| item.status == Status::Pending && item.keyed)
            .take(count)
            .map(|item| item.page)
            .filter(|page| !self.claimed.contains(page))
            .collect()
    }

    /// Leases the work item of `page`.
    fn lease(&mut self, page: usize) {
        let item = self.item(page);
        item.status = Status::Leased;
        item.attempts += 1;
        item.epoch += 1;
    }

    fn seed(&mut self, pages: &[usize]) -> Answer {
        if self.closed {
            self.committed();
            return Answer::Admitted(None);
        }
        let added: Vec<bool> = pages.iter().map(|page| self.add(*page)).collect();
        self.wrote(added.contains(&true));
        self.committed();
        Answer::Admitted(Some(added))
    }

    /// Keys the next unkeyed work items, a sealed batch of them, in a
    /// write that needs no sync of its own, which the loop seals when it
    /// keyed any; answers their pages.
    fn seal(&mut self) -> Vec<usize> {
        let pages = self
            .items
            .iter_mut()
            .filter(|item| !item.keyed)
            .take(KEYED_AT_ONCE)
            .map(|item| {
                item.keyed = true;
                item.page
            })
            .collect::<Vec<usize>>();
        self.wrote(false);
        if !pages.is_empty() {
            self.committed();
        }
        pages
    }

    fn claim(&mut self, count: usize) -> Answer {
        let pages = self.waiting(count);
        for page in &pages {
            self.lease(*page);
            self.attempts.push(Attempt {
                page: *page,
                staged: Vec::new(),
                over: false,
            });
        }
        self.wrote(false);
        self.claimed.extend(&pages);
        Answer::Leased(pages)
    }

    /// An attempt's enqueue, a barrier however it is answered.
    fn enqueue(&mut self, index: usize, page: usize) -> Answer {
        let answer = self.staged(index, page);
        self.committed();
        answer
    }

    fn staged(&mut self, index: usize, page: usize) -> Answer {
        let attempt = &self.attempts[index];
        if attempt.over {
            return Answer::Admitted(None);
        }
        if self.holds(page) || attempt.staged.iter().any(|(staged, _)| *staged == page) {
            return Answer::Admitted(Some(vec![false]));
        }
        // Its place is reserved at once, in a synced write.
        self.attempts[index].staged.push((page, self.next_place));
        self.next_place += 1;
        self.wrote(true);
        Answer::Admitted(Some(vec![true]))
    }

    fn end(&mut self, index: usize, how: How) -> Answer {
        let attempt = self.attempts[index].clone();
        if attempt.over {
            return Answer::Ended(false);
        }
        self.attempts[index].over = true;
        match how {
            How::Commit => {
                self.item(attempt.page).status = Status::Succeeded;
                // A page another admission added meanwhile is not added
                // again.
                for (page, place) in attempt.staged {
                    if !self.holds(page) {
                        self.items.push(Item::added(page, place, true));
                    }
                }
                self.items.sort_by_key(|item| item.place);
            }
            How::Fail => self.item(attempt.page).status = Status::Failed(HANDLER_FAILED),
            How::Panic => {
                let item = self.item(attempt.page);
                item.status = if item.may_retry() {
                    Status::Pending
                } else {
                    Status::Failed(ENGINE_PANICKED)
                };
            }
        }
        self.wrote(true);
        Answer::Ended(true)
    }

    fn readmit(&mut self) -> Answer {
        if self.closed {
            self.committed();
            return Answer::Readmitted(None);
        }
        let mut readmitted = 0;
        for item in &mut self.items {
            if matches!(item.status, Status::Failed(_)) {
                item.status = Status::Pending;
                item.retried_from = item.attempts;
                readmitted += 1;
            }
        }
        self.wrote(readmitted > 0);
        self.committed();
        Answer::Readmitted(Some(readmitted))
    }

    fn refuse(&mut self) -> Answer {
        let refused = self.waiting(1);
        for page in &refused {
            self.lease(*page);
            self.item(*page).status = Status::Failed(SITE_REFUSED);
            self.wrote(true);
        }
        self.claimed.extend(&refused);
        Answer::Refused(!refused.is_empty())
    }

    fn drain(&mut self) -> Answer {
        let any = |items: &[Item], status| items.iter().any(|item| item.status == status);
        if any(&self.items, Status::Leased) {
            return Answer::Drained(None);
        }
        self.closed = !any(&self.items, Status::Pending);
        self.wrote(false);
        Answer::Drained(Some(self.closed))
    }

    /// The crawl stops cleanly: its last write releases its leases, and
    /// the program's own stop counts no attempt.
    fn release(&mut self) {
        for item in &mut self.items {
            if item.status == Status::Leased {
                item.status = Status::Pending;
                item.attempts -= 1;
            }
        }
        self.wrote(false);
        self.committed();
    }

    /// The crawl's process crashes: the OS still writes what it was given,
    /// every commit, and at `batch` the open batch is lost. The resumed
    /// crawl recovers its leases in a write of its own.
    fn crash(&mut self) {
        if self.batching {
            self.items.clone_from(&self.written);
        } else {
            self.wrote(true);
        }
        self.items = recovered(std::mem::take(&mut self.items));
        self.wrote(false);
        self.committed();
        self.resume();
    }

    /// The crawl crashes as `kind` says: a process crash resumes it, and a
    /// power cut ends the case.
    fn crashes(&mut self, kind: CrashKind) {
        match kind {
            CrashKind::Process => self.crash(),
            CrashKind::PowerCut => self.crashed = true,
        }
    }

    /// The crawl resumes: nothing of its process is left but the work
    /// items, which it keys, a sealed batch at a time.
    fn resume(&mut self) {
        self.attempts.clear();
        self.claimed.clear();
        self.closed = false;
        while !self.seal().is_empty() {}
    }
}

impl ReferenceStateMachine for Model {
    type State = Self;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self> {
        (1_usize..4, any::<bool>())
            .prop_map(|(count, batching)| {
                // The seeds the start wrote, which no gate had keyed.
                let items: Vec<Item> = (0..count)
                    .map(|page| Item::added(page, u64::try_from(page).unwrap(), false))
                    .collect();
                Self {
                    unsynced: vec![items.clone()],
                    written: items.clone(),
                    items,
                    next_place: u64::try_from(count).unwrap(),
                    attempts: Vec::new(),
                    closed: false,
                    answered: Answer::Nothing,
                    crashed: false,
                    batching,
                    claimed: Vec::new(),
                }
            })
            .boxed()
    }

    fn transitions(_: &Self) -> BoxedStrategy<Transition> {
        let crash = prop_oneof![Just(CrashKind::Process), Just(CrashKind::PowerCut)];
        let how = prop_oneof![
            3 => Just(How::Commit),
            1 => Just(How::Fail),
            2 => Just(How::Panic),
        ];
        prop_oneof![
            3 => proptest::collection::vec(0..PAGES, 1..4).prop_map(Transition::Seed),
            2 => Just(Transition::Seal),
            4 => (1_usize..3).prop_map(Transition::Lease),
            3 => (any::<Index>(), 0..PAGES).prop_map(|(attempt, page)| Transition::Enqueue(attempt, page)),
            5 => (any::<Index>(), how).prop_map(|(attempt, how)| Transition::End(attempt, how)),
            1 => Just(Transition::Readmit),
            1 => Just(Transition::Refuse),
            1 => Just(Transition::Drain),
            1 => proptest::option::weighted(0.5, crash.clone()).prop_map(Transition::Stop),
            2 => crash.prop_map(Transition::Crash),
        ]
        .boxed()
    }

    fn apply(mut model: Self, transition: &Transition) -> Self {
        model.answered = Answer::Nothing;
        if model.crashed {
            return model;
        }
        let attempts = model.attempts.len();
        let chosen = |index: &Index| (attempts > 0).then(|| index.index(attempts));
        model.answered = match transition {
            Transition::Seed(pages) => model.seed(pages),
            Transition::Seal => Answer::Keyed(model.seal()),
            Transition::Lease(count) => model.claim(*count),
            Transition::Enqueue(index, page) => {
                chosen(index).map_or(Answer::Nothing, |index| model.enqueue(index, *page))
            }
            Transition::End(index, how) => {
                chosen(index).map_or(Answer::Nothing, |index| model.end(index, *how))
            }
            Transition::Readmit => model.readmit(),
            Transition::Refuse => model.refuse(),
            Transition::Drain => model.drain(),
            Transition::Stop(crash) => {
                model.release();
                match crash {
                    None => model.resume(),
                    Some(kind) => model.crashes(*kind),
                }
                Answer::Nothing
            }
            Transition::Crash(kind) => {
                model.crashes(*kind);
                Answer::Nothing
            }
        };
        model
    }
}

/// A crawl as one process holds it: its run file's writer, its Run state,
/// the mailbox other tasks ask it over, and its attempts, in claim order.
struct Crawl {
    owner: TaskOwner,
    writer: goodfetch_engine::run_state::RunState,
    state: RunState,
    asking: Asking,
    asked: Asked,
    leases: Leases,
    attempts: Vec<(Leased, WorkItemLease)>,
    /// How many attempts hold their lease.
    in_flight: usize,
}

impl Crawl {
    fn stored(&self) -> Stored {
        self.state.stored().clone()
    }

    /// What `call`, another task's, is answered once the loop, which this
    /// plays, took its ask from the mailbox and answered it.
    async fn asks<T>(&mut self, call: impl Future<Output = Result<T, StateError>>) -> T {
        let answer = async {
            let ask = self
                .asked
                .next()
                .await
                .expect("invariant: the mailbox is open");
            self.state.answer(ask).await;
        };
        let (answered, ()) = tokio::join!(call, answer);
        answered.expect("the Run state answers")
    }
}

/// The Run state under test: a run file on the crash VFS, and the crawl
/// that runs on it until it stops or crashes.
struct Subject {
    runtime: Runtime,
    env: TestEnv,
    vfs: CrashVfs,
    path: PathBuf,
    /// The Run state's level: `full`, or `batch` with no threshold, so
    /// only a barrier, a seal or the writer's end writes the open batch.
    durability: Durability,
    crawl: Option<Crawl>,
    finished: bool,
}

/// The level of a model that batches attempts or not.
fn durability(batching: bool) -> Durability {
    if batching {
        Durability {
            pages: BatchThreshold::Never,
            interval: BatchThreshold::Never,
            bytes: BatchThreshold::Never,
            ..Durability::BATCH
        }
    } else {
        Durability::FULL
    }
}

/// The seed of the page `page`.
fn seed(page: usize) -> Seed {
    Seed::Request(RequestSeed {
        url: url(page),
        label: "page".to_owned(),
        payload: None,
        priority: 0,
        forefront: false,
        method: HttpMethod::get(),
        body: None,
        key: None,
    })
}

fn url(page: usize) -> String {
    format!("https://shop.test/p/{page}")
}

/// The page at `url`.
fn page(url: &str) -> usize {
    url.rsplit('/')
        .next()
        .and_then(|page| page.parse().ok())
        .unwrap_or_else(|| panic!("{url} is no page of the model"))
}

/// When the model's attempts end: their outcomes' times are not its
/// subject.
fn ended_at() -> PointInTime {
    PointInTime::from_rfc3339("2026-01-05T09:00:00Z".to_owned())
}

fn failure(code: &str) -> Failure {
    Failure::new(code, "the model's failure")
}

/// Whether each of `answers` says its request was added; one not added is
/// a duplicate, the only refusal the model's pages meet.
fn added(answers: &[Enqueued]) -> Vec<bool> {
    answers
        .iter()
        .map(|answer| match answer {
            Enqueued::Added { .. } => true,
            Enqueued::NotAdded {
                reason: NotAddedReason::Duplicate,
                ..
            } => false,
            Enqueued::NotAdded { .. } => panic!("a page was refused: {answer:?}"),
        })
        .collect()
}

/// What the run file `connection` reads shows of each work item.
fn view(connection: &Connection) -> View {
    connection
        .prepare(
            "SELECT url, status, domain_key IS NOT NULL, attempts, lease_epoch FROM work_items
             ORDER BY position",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                page(&row.get::<_, String>(0)?),
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                u64::try_from(row.get::<_, i64>(4)?).unwrap(),
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The requests by state `tally` counts.
fn counts(tally: &Tally) -> Counts {
    let ended = |kind| -> Vec<(String, u64)> {
        tally
            .ended(kind)
            .map(|(name, count)| (name.to_owned(), count))
            .collect()
    };
    assert_eq!(
        (ended(OutcomeKind::Skipped), ended(OutcomeKind::Blocked)),
        (Vec::new(), Vec::new()),
        "the model skips and blocks no request"
    );
    let succeeded = ended(OutcomeKind::Succeeded)
        .into_iter()
        .map(|(_, count)| count)
        .sum();
    (tally.left(), succeeded, ended(OutcomeKind::Failed))
}

impl Subject {
    fn crawl(&mut self) -> &mut Crawl {
        self.crawl
            .as_mut()
            .expect("invariant: the crawl runs until the case ends")
    }

    /// A connection of its own on the run file, which reads what the
    /// writer committed.
    fn reader(&self) -> Connection {
        Connection::open_with_flags_and_vfs(
            &self.path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            self.vfs.name(),
        )
        .unwrap()
    }

    /// The crawl's process ends: its writer stops and its tasks end.
    fn close(&mut self) {
        if let Some(crawl) = self.crawl.take() {
            let Crawl {
                owner,
                writer,
                state,
                attempts,
                ..
            } = crawl;
            // Every handle on the writer goes before its owner waits for it.
            drop((writer, state, attempts));
            self.runtime.block_on(owner.wait());
        }
    }

    /// Opens the crawl on the run file: started with the seeds of `start`,
    /// which no gate keys, or resumed, as its driver resumes it, each
    /// attempt in flight counted as `handler_lost` and every work item
    /// keyed before its Run state opens.
    fn open(&mut self, start: Option<Vec<Seed>>) {
        let (path, seed) = (self.path.clone(), self.env.seed());
        let options = RunStateOptions {
            vfs: Some(self.vfs.name().to_owned()),
            durability: self.durability,
            ..RunStateOptions::default()
        };
        let start_time = self.env.start_time();
        let crawl = self.runtime.block_on(async {
            let owner = TaskOwner::root(BlockingPolicy::Inline);
            let writer = goodfetch_engine::run_state::RunState::open(
                path,
                &options,
                TABLES.to_vec(),
                1,
                &owner,
            )
            .await
            .unwrap();
            let egress = Arc::new(Policy::new(&["*"], &[] as &[&str]).unwrap());
            let admitting = Arc::clone(&egress);
            let resumed = start.is_none();
            let invariants = writer
                .write(move |connection| {
                    let invariants =
                        fixture::within(connection, goodfetch_frontier::Scope::default())?;
                    match start {
                        Some(seeds) => {
                            work_items::admit_seeds(
                                connection,
                                &mut work_items::Positions::of(connection)?,
                                &invariants,
                                &admitting,
                                &fixture::checked(&invariants, seeds),
                            )?;
                        }
                        None => work_items::recover_leases(
                            connection,
                            &failure(HANDLER_LOST),
                            POLICY.retry_max,
                            &ended_at(),
                            &mut Changed::default(),
                        )?,
                    }
                    Ok(invariants)
                })
                .await
                .unwrap();
            let staging = Staging::default();
            let keys = Keys::new(|_| KEY.to_owned());
            let stored = Stored::new(
                writer.clone(),
                Within {
                    egress,
                    invariants,
                    staging: staging.clone(),
                    intake: Intake::default(),
                },
                keys.clone(),
                CountedWork::default(),
            );
            if resumed {
                loop {
                    let unkeyed = stored.unkeyed(KEYED_AT_ONCE).await.unwrap();
                    if unkeyed.is_empty() {
                        break;
                    }
                    let keys = unkeyed
                        .into_iter()
                        .map(|(_, waiting)| (waiting.id, KEY.to_owned()))
                        .collect();
                    stored.key(keys, Unkeyed::Opening).await.unwrap();
                    stored.seal().await.unwrap();
                }
            }
            let (pool, tally) = stored.open(Entropy::seeded(seed)).await.unwrap();
            let (asking, asked) = mailbox(CountedWork::default());
            Crawl {
                owner,
                writer,
                state: RunState::new(pool, TierChoices::default(), tally, stored),
                asking,
                asked,
                leases: Leases::new(Clock::paused(start_time), "model", 1)
                    .staging_in(staging)
                    .keyed_by(keys),
                attempts: Vec::new(),
                in_flight: 0,
            }
        });
        self.crawl = Some(crawl);
    }

    /// Gives `pages` as seeds to the running crawl, as another task does;
    /// answers whether each was added, `None` once the intake closed.
    fn seeds(&mut self, pages: &[usize]) -> Option<Vec<bool>> {
        let seeds = pages.iter().copied().map(seed).collect();
        let crawl = self
            .crawl
            .as_mut()
            .expect("invariant: the crawl runs until the case ends");
        let asking = crawl.asking.clone();
        self.runtime
            .block_on(crawl.asks(asking.seeds(seeds)))
            .map(|answers| added(&answers.expect("the model's seeds are valid")))
    }

    /// The attempt at `index` enqueues `page`.
    fn enqueue(&mut self, index: usize, page: usize) -> Option<Vec<bool>> {
        let crawl = self
            .crawl
            .as_mut()
            .expect("invariant: the crawl runs until the case ends");
        let leased = &crawl.attempts[index].0;
        let enqueue = Enqueue {
            request: CrawlRequestEnqueueParams {
                run_id: RunId::parse("model").unwrap(),
                crawl_request_id: leased.crawl_request.id.clone(),
                lease_epoch: leased.lease_epoch,
                url: url(page),
                label: "page".to_owned(),
                payload: None,
                priority: 0,
                forefront: false,
                method: HttpMethod::get(),
                body: None,
                key: None,
                document: None,
            },
            payload: None,
            document: None,
        };
        let asking = crawl.asking.clone();
        self.runtime
            .block_on(crawl.asks(asking.enqueue(enqueue)))
            .map(|answer| added(&[answer.expect("the model's pages are valid")]))
    }

    /// The loop claims the next `count` pending work items, in a write
    /// that needs no sync of its own, and grants each one's attempt its
    /// lease; answers their pages.
    fn lease(&mut self, count: usize) -> Vec<usize> {
        let crawl = self
            .crawl
            .as_mut()
            .expect("invariant: the crawl runs until the case ends");
        let asked: Vec<(String, String)> = self
            .runtime
            .block_on(crawl.state.waiting(None, count))
            .unwrap()
            .into_iter()
            .map(|waiting| (waiting.id, waiting.key))
            .collect();
        if asked.is_empty() {
            return Vec::new();
        }
        let leased = self.runtime.block_on(crawl.state.leasable(asked)).unwrap();
        let claim = crawl
            .writer
            .write_with(Syncing::Deferred { charged: 0 }, crawl.state.claim(&leased));
        // The loop does not wait for its claim; the model does, so that
        // every step's file is the one it checks, and the Run state hears
        // the answer at its next operation as the loop's does.
        let unclaimed = self.runtime.block_on(claim).unwrap();
        assert!(
            unclaimed.is_empty(),
            "a claim found its rows changed: {unclaimed:?}"
        );
        crawl
            .state
            .claiming(Box::pin(std::future::ready(Ok(unclaimed))), &leased);
        let _clock = self.runtime.enter();
        leased
            .into_iter()
            .map(|leased| {
                let lease = crawl
                    .leases
                    .grant(&leased.crawl_request.id, leased.lease_epoch, LEASE);
                let page = page(&leased.crawl_request.url);
                crawl.attempts.push((leased, lease));
                crawl.in_flight += 1;
                page
            })
            .collect()
    }

    /// The attempt at `index` ends as `how` says, in its outcome's one
    /// job; answers whether that ended its work item's lease.
    fn end(&mut self, index: usize, how: How) -> bool {
        let crawl = self
            .crawl
            .as_mut()
            .expect("invariant: the crawl runs until the case ends");
        let (leased, lease) = crawl.attempts[index].clone();
        let attempt = (leased.crawl_request.id.clone(), leased.lease_epoch);
        let unsuccessful = |unsuccessful| Ending::End {
            work_item: attempt.0.clone(),
            lease_epoch: attempt.1,
            unsuccessful,
            ended_at: ended_at(),
        };
        let ending = {
            let _clock = self.runtime.enter();
            match how {
                How::Commit => {
                    match lease.committing(&crawl.writer, &Masks::new(), Vec::new(), ended_at()) {
                        Ok(commit) => Ending::Commit {
                            commit,
                            refused: Box::new(|refused| {
                                panic!("the model pushes no Record: {refused:?}")
                            }),
                        },
                        Err(CommitError::LeaseLost) => {
                            crawl.state.settled(leased, false);
                            return false;
                        }
                        Err(error) => panic!("the commit failed: {error:?}"),
                    }
                }
                How::Fail => {
                    lease.release();
                    unsuccessful(Unsuccessful::Failed(failure(HANDLER_FAILED)))
                }
                How::Panic => {
                    // The engine's panic ended the attempt, while it still
                    // ran: what it staged goes with its lease, and it ends
                    // as a failed attempt does (#2285).
                    lease.release();
                    let panicked = failure(ENGINE_PANICKED);
                    unsuccessful(
                        if POLICY.retry_engine_panicked && POLICY.may_retry(leased.tries) {
                            Unsuccessful::Retried(panicked)
                        } else {
                            Unsuccessful::Failed(panicked)
                        },
                    )
                }
            }
        };
        let outcome = Outcome {
            scored: goodfetch_session_pool::Changes::default(),
            chosen: None,
            learning: None,
            ending,
        };
        let stored = crawl.stored();
        let written = self
            .runtime
            .block_on(async { stored.end(outcome).await.await })
            .unwrap();
        crawl.state.changed(written.changed);
        let (ended, retried) = match written.ended {
            Ended::Committed(committed) => {
                let _clock = self.runtime.enter();
                let committed = lease.committed(Ok(Some(Ok(committed)))).unwrap();
                // The model reads every page it leases back from the file.
                crawl.state.added(committed.keyed);
                (true, false)
            }
            Ended::LeaseLost => (false, false),
            Ended::Unsuccessful { ended, retried, .. } => (ended, ended && retried),
            Ended::SitemapRead(_) => panic!("the model reads no sitemap"),
        };
        crawl.state.settled(leased, retried);
        if ended {
            crawl.in_flight -= 1;
        }
        ended
    }

    /// The loop keys the next unkeyed work items, in a sealed batch that
    /// needs no sync of its own, and seals it when it keyed any, as
    /// `key_new` does; answers their pages.
    fn seal(&mut self) -> Vec<usize> {
        let stored = self.crawl().stored();
        let unkeyed = self
            .runtime
            .block_on(stored.unkeyed(KEYED_AT_ONCE))
            .unwrap();
        let pages = unkeyed.iter().map(|(url, _)| page(url)).collect();
        if !unkeyed.is_empty() {
            let keys = unkeyed
                .into_iter()
                .map(|(_, waiting)| (waiting.id, KEY.to_owned()))
                .collect();
            self.runtime
                .block_on(stored.key(keys, Unkeyed::Opening))
                .unwrap();
            let crawl = self
                .crawl
                .as_mut()
                .expect("invariant: the crawl runs until the case ends");
            self.runtime.block_on(crawl.state.seal()).unwrap();
        }
        pages
    }

    /// Re-admits the failures, as another task asks it; answers how many,
    /// `None` once the intake closed.
    fn readmit(&mut self) -> Option<u64> {
        let crawl = self
            .crawl
            .as_mut()
            .expect("invariant: the crawl runs until the case ends");
        let asking = crawl.asking.clone();
        self.runtime.block_on(crawl.asks(asking.readmit(false)))
    }

    /// The next pending work item fails with no attempt: its site asked
    /// not to send it. Answers whether there was one.
    fn refuse(&mut self) -> bool {
        let crawl = self
            .crawl
            .as_mut()
            .expect("invariant: the crawl runs until the case ends");
        let Some(waiting) = self
            .runtime
            .block_on(crawl.state.waiting(None, 1))
            .unwrap()
            .pop()
        else {
            return false;
        };
        let stored = crawl.stored();
        let unsent = vec![(waiting.id.clone(), failure(SITE_REFUSED))];
        let changed = self
            .runtime
            .block_on(stored.end_unsent(unsent, ended_at()))
            .unwrap();
        crawl.state.changed(changed);
        crawl.state.gone([waiting.id]);
        true
    }

    /// The loop, once no attempt is in flight, asks whether the crawl
    /// drained; answers whether its intake closed, `None` when it did not
    /// ask.
    fn drain(&mut self) -> Option<bool> {
        if self.crawl().in_flight > 0 {
            return None;
        }
        let stored = self.crawl().stored();
        let counted = self.crawl().state.counted();
        let unwritten = (failure("outcome_unwritten"), POLICY.retry_max);
        let (closed, changed) = self
            .runtime
            .block_on(stored.close_if_drained(counted, unwritten, ended_at()))
            .unwrap();
        self.crawl().state.changed(changed);
        Some(closed)
    }

    /// The crawl stops cleanly: its last write releases its leases.
    fn release(&mut self) {
        let released = self.crawl().stored();
        self.runtime.block_on(released.release()).unwrap();
    }

    /// The crawl's process ends as `crash` says, cleanly with none, and
    /// the next process resumes it; a power cut ends the case with what
    /// the resumed crawl holds.
    fn ends(&mut self, crash: Option<CrashKind>, unsynced: &[Vec<Item>]) {
        match crash {
            None => {
                self.close();
                self.open(None);
            }
            Some(CrashKind::Process) => {
                self.crash(CrashKind::Process);
                self.open(None);
            }
            Some(CrashKind::PowerCut) => {
                self.power_cut(unsynced);
                self.close();
                self.finished = true;
            }
        }
    }

    /// The crawl's process crashes as `kind` says; answers what the file
    /// the next process opens shows of each work item, once it passed
    /// `PRAGMA integrity_check`.
    fn crash(&mut self, kind: CrashKind) -> View {
        self.vfs.crash(kind);
        self.close();
        let connection = after_crash(&self.vfs, &self.path.to_string_lossy()).unwrap();
        view(&connection)
    }

    /// The power goes, and the crawl resumes: the file holds the work
    /// items as one of `unsynced` has them, the model's after each write
    /// from the last synced one on, and the resumed crawl, given every
    /// page again, adds each one it does not hold, once.
    fn power_cut(&mut self, unsynced: &[Vec<Item>]) {
        let landed = self.crash(CrashKind::PowerCut);
        let survived = unsynced
            .iter()
            .find(|items| view_of(items) == landed)
            .unwrap_or_else(|| {
                panic!(
                    "the power cut left {landed:?}, which none of these writes left: {unsynced:?}"
                )
            });
        self.open(None);
        let resumed = keyed(recovered(survived.clone()));
        self.holds(&resumed, &resumed);
        let pages: Vec<usize> = (0..PAGES).collect();
        let new = pages
            .iter()
            .map(|page| !resumed.iter().any(|item| item.page == *page))
            .collect();
        let again = self.seeds(&pages);
        let mut all: Vec<usize> = view(&self.reader())
            .into_iter()
            .map(|(page, ..)| page)
            .collect();
        all.sort_unstable();
        assert_eq!((again, all), (Some(new), pages));
    }

    /// Checks that the run file holds the work items `written`, the last
    /// commit's, that the requests by state the Run state counts in memory
    /// are those of `items`, every write's, and the run file's those of
    /// `written`; with nothing left to commit, the two counts are one.
    fn holds(&self, items: &[Item], written: &[Item]) {
        let connection = self.reader();
        let crawl = self
            .crawl
            .as_ref()
            .expect("invariant: the crawl runs until the case ends");
        let tally = crawl.state.tally();
        let read = Tally::read(&connection).unwrap();
        assert_eq!(
            (view(&connection), counts(&tally), counts(&read)),
            (view_of(written), counts_of(items), counts_of(written))
        );
        if view_of(items) == view_of(written) {
            assert_eq!(*tally, read);
        }
    }
}

impl StateMachineTest for Subject {
    type SystemUnderTest = Self;
    type Reference = Model;

    fn init_test(model: &Model) -> Self {
        let env = TestEnv::new();
        let folder = env
            .folder()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace(' ', "-");
        let vfs = CrashVfs::register(&format!("run-state-model-{folder}"), env.seed()).unwrap();
        let path = env.folder().join("run.sqlite");
        let mut subject = Self {
            runtime: goodfetch_test_support::runtime(),
            env,
            vfs,
            path,
            durability: durability(model.batching),
            crawl: None,
            finished: false,
        };
        subject.open(Some(
            model.items.iter().map(|item| seed(item.page)).collect(),
        ));
        subject
    }

    fn apply(mut subject: Self, model: &Model, transition: Transition) -> Self {
        if subject.finished {
            return subject;
        }
        let attempts = subject.crawl().attempts.len();
        let chosen = |index: Index| (attempts > 0).then(|| index.index(attempts));
        let answered = match transition {
            Transition::Seed(pages) => Answer::Admitted(subject.seeds(&pages)),
            Transition::Seal => Answer::Keyed(subject.seal()),
            Transition::Lease(count) => Answer::Leased(subject.lease(count)),
            Transition::Enqueue(index, page) => chosen(index).map_or(Answer::Nothing, |index| {
                Answer::Admitted(subject.enqueue(index, page))
            }),
            Transition::End(index, how) => chosen(index).map_or(Answer::Nothing, |index| {
                Answer::Ended(subject.end(index, how))
            }),
            Transition::Readmit => Answer::Readmitted(subject.readmit()),
            Transition::Refuse => Answer::Refused(subject.refuse()),
            Transition::Drain => Answer::Drained(subject.drain()),
            Transition::Stop(crash) => {
                subject.release();
                subject.ends(crash, &model.unsynced);
                Answer::Nothing
            }
            Transition::Crash(kind) => {
                subject.ends(Some(kind), &model.unsynced);
                Answer::Nothing
            }
        };
        assert_eq!(
            answered, model.answered,
            "the step answered otherwise than the model says: {model:?}"
        );
        subject
    }

    fn check_invariants(subject: &Self, model: &Model) {
        if !subject.finished {
            subject.holds(&model.items, &model.written);
        }
    }

    fn teardown(mut subject: Self, _: Model) {
        subject.close();
    }
}

prop_state_machine! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    #[test]
    fn the_run_state_holds_its_models_work_items_and_counts_through_admissions_endings_panics_and_crashes(
        sequential 1..40 => Subject
    );
}
