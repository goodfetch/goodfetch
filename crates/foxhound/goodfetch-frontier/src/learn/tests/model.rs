//! The model-based test of the learned rules that merge URLs (ADR 0060;
//! TESTING.md, Model-based tests and crashes in one process): seeds of one
//! host's `/item[/]?n=<n>[&ref=<r>]` admitted into a run file on real
//! SQLite through the crash VFS, pages of admitted requests fetched with a
//! page signature that `ref` or the trailing slash changes only now and
//! then, and kills, between steps and in the middle of an admission or a
//! fetch's learning, before its commit. A reference model, which knows
//! only the spec's thresholds (3 pairs of pages whose URLs differ only by
//! a parameter, matching, and none differing; 2 such pairs for the
//! trailing slash's form) and that a differing pair stops a learned rule,
//! says which rules hold and which stopped, which candidate is added and
//! which is a `duplicate`, which pending request a rule learned later ends
//! `skipped` with `duplicate`, and which URLs a rule merged. After every
//! step and every crash, the run file holds what the model says, and
//! nothing of a killed step. 16 cases in the Quick suite and 256 in the
//! Pre-merge suite (`proptest_config`).

use std::collections::{BTreeMap, BTreeSet};

use goodfetch_engine::egress::Policy;
use goodfetch_engine::run_state::{RunState, RunStateOptions, RunStateTables};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod};
use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv, after_crash};
use proptest::prelude::*;
use proptest::sample::Index;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use rusqlite::{Connection, OptionalExtension as _};
use sha2::{Digest as _, Sha256};
use tokio::runtime::Runtime;
use url::Url;

use crate::identity::{self, Declared};
use crate::rules::{self, learned_rules};
use crate::scope::{self, Scope};
use crate::{Candidate, EndMerged, PageSeen, admit, bounds, learn};

/// The crawl's own table of the test: each request added, by its ID, with
/// its status: `pending`, `fetched`, or `skipped` once a rule merged it.
const REQUESTS: RunStateTables = RunStateTables {
    name: "model-requests",
    statements: &["CREATE TABLE IF NOT EXISTS requests (
            id INTEGER NOT NULL PRIMARY KEY,
            url TEXT NOT NULL,
            status TEXT NOT NULL
        ) STRICT"],
};

/// The rule of the trailing slash's form, as the model names it.
const SLASH: &str = "slash";

/// A page the case asks for: item `n`, with `ref` or without, at its path
/// with a trailing slash or without.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Asked {
    n: u8,
    reference: Option<u8>,
    slash: bool,
}

impl Asked {
    fn url(self) -> String {
        let path = if self.slash { "/item/" } else { "/item" };
        match self.reference {
            Some(reference) => format!("http://shop.test{path}?n={}&ref={reference}", self.n),
            None => format!("http://shop.test{path}?n={}", self.n),
        }
    }

    /// The page under the rules `learned` holds.
    fn under(self, learned: &BTreeSet<&'static str>) -> (Option<u8>, Option<u8>, bool) {
        (
            (!learned.contains("n")).then_some(self.n),
            self.reference.filter(|_| !learned.contains("ref")),
            self.slash && !learned.contains(SLASH),
        )
    }

    /// Whether the rule `rule` changes it.
    fn changed_by(self, rule: &str) -> bool {
        match rule {
            "n" => true,
            "ref" => self.reference.is_some(),
            _ => self.slash,
        }
    }

    /// The rule a pair of it and `other` is evidence for, if any: they
    /// differ by it alone.
    fn paired(self, other: Self) -> Option<&'static str> {
        match (
            self.n == other.n,
            self.reference == other.reference,
            self.slash == other.slash,
        ) {
            (true, false, true) => Some("ref"),
            (false, true, true) => Some("n"),
            (true, true, false) => Some(SLASH),
            _ => None,
        }
    }
}

fn asked() -> impl Strategy<Value = Asked> {
    (
        0_u8..3,
        proptest::option::weighted(0.6, 0_u8..3),
        proptest::bool::weighted(0.3),
    )
        .prop_map(|(n, reference, slash)| Asked {
            n,
            reference,
            slash,
        })
}

fn kind() -> impl Strategy<Value = CrashKind> {
    prop_oneof![Just(CrashKind::Process), Just(CrashKind::PowerCut)]
}

#[derive(Clone, Debug)]
enum Transition {
    /// A seed is admitted and committed.
    Admit(Asked),
    /// A seed is admitted, and the process crashes or the power goes
    /// before its commit.
    KilledAdmit(Asked, CrashKind),
    /// The page of an added request is fetched and learned from: its text
    /// is its item's, or, `distinct`, one of its own.
    Fetch(Index, bool),
    /// The same, and the process crashes or the power goes before the
    /// learning's commit.
    KilledFetch(Index, bool, CrashKind),
    /// The process crashes or the power goes between steps.
    Crash(CrashKind),
}

/// What the run file must hold.
#[derive(Clone, Debug, Default)]
struct Model {
    /// The pages added, in order: request `i + 1` is `added[i]`.
    added: Vec<Asked>,
    /// The pages fetched, by request, with their page's text.
    fetched: BTreeMap<usize, String>,
    /// The requests a rule learned later ended `skipped`.
    skipped: BTreeSet<usize>,
    /// Each rule's pairs: how many matched and how many differed.
    pairs: BTreeMap<&'static str, (u32, u32)>,
    /// The rules that hold.
    learned: BTreeSet<&'static str>,
    /// The rules a contradicting pair stopped.
    stopped: BTreeSet<&'static str>,
    /// The URLs a rule merged.
    merged: BTreeSet<String>,
    /// The answer the last admission must get: its ID, or `None`.
    last_answer: Option<usize>,
}

impl Model {
    /// The answer to `asked`: the ID it is added with, or `None` for a
    /// duplicate, which a rule merged when the page itself was never
    /// added.
    fn admitted(&self, asked: Asked) -> (Option<usize>, bool) {
        let key = asked.under(&self.learned);
        let duplicate = self
            .added
            .iter()
            .any(|added| added.under(&self.learned) == key);
        let merged = duplicate && !self.added.contains(&asked);
        ((!duplicate).then_some(self.added.len() + 1), merged)
    }

    /// Learns from request `id`'s page, read with `text`, once, unless a
    /// rule ended it.
    fn fetch(&mut self, id: usize, text: String) {
        if self.fetched.contains_key(&id) || self.skipped.contains(&id) {
            return;
        }
        let page = self.added[id - 1];
        for (other_id, other_text) in &self.fetched {
            let Some(rule) = page.paired(self.added[other_id - 1]) else {
                continue;
            };
            let counts = self.pairs.entry(rule).or_default();
            if *other_text == text {
                counts.0 += 1;
            } else {
                counts.1 += 1;
            }
        }
        self.fetched.insert(id, text);
        let mut learned = Vec::new();
        for (rule, (matching, differing)) in &self.pairs {
            let threshold = if *rule == SLASH { 2 } else { 3 };
            if self.learned.contains(rule) && *differing > 0 {
                self.learned.remove(rule);
                self.stopped.insert(rule);
            } else if !self.learned.contains(rule)
                && !self.stopped.contains(rule)
                && *matching >= threshold
                && *differing == 0
            {
                self.learned.insert(rule);
                learned.push(*rule);
            }
        }
        for rule in learned {
            self.end_merged(rule);
        }
    }

    /// Ends each pending request the rule `rule`, just learned, shows to
    /// be one page with another: all of them when one was fetched, else
    /// all but the first.
    fn end_merged(&mut self, rule: &str) {
        let mut pages: BTreeMap<_, Vec<usize>> = BTreeMap::new();
        for (index, added) in self.added.iter().enumerate() {
            pages
                .entry(added.under(&self.learned))
                .or_default()
                .push(index + 1);
        }
        for ids in pages.values() {
            let changed = ids.iter().any(|id| self.added[id - 1].changed_by(rule));
            if ids.len() < 2 || !changed {
                continue;
            }
            let mut pending: Vec<usize> = ids
                .iter()
                .copied()
                .filter(|id| !self.fetched.contains_key(id) && !self.skipped.contains(id))
                .collect();
            if !ids.iter().any(|id| self.fetched.contains_key(id)) && !pending.is_empty() {
                pending.remove(0);
            }
            for id in pending {
                self.skipped.insert(id);
                self.merged.insert(self.added[id - 1].url());
            }
        }
    }
}

/// The text of request `id`'s page: its item's, unless `distinct`.
fn text(page: Asked, id: usize, distinct: bool) -> String {
    if distinct {
        format!("Item {}, request {id}", page.n)
    } else {
        format!("Item {}", page.n)
    }
}

impl ReferenceStateMachine for Model {
    type State = Self;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self> {
        Just(Self::default()).boxed()
    }

    fn transitions(_: &Self) -> BoxedStrategy<Transition> {
        prop_oneof![
            5 => asked().prop_map(Transition::Admit),
            1 => (asked(), kind()).prop_map(|(asked, kind)| Transition::KilledAdmit(asked, kind)),
            5 => (any::<Index>(), proptest::bool::weighted(0.1))
                .prop_map(|(index, distinct)| Transition::Fetch(index, distinct)),
            1 => (any::<Index>(), any::<bool>(), kind())
                .prop_map(|(index, distinct, kind)| Transition::KilledFetch(index, distinct, kind)),
            1 => kind().prop_map(Transition::Crash),
        ]
        .boxed()
    }

    fn apply(mut model: Self, transition: &Transition) -> Self {
        match *transition {
            Transition::Admit(asked) => {
                let (answer, merged) = model.admitted(asked);
                model.last_answer = answer;
                if answer.is_some() {
                    model.added.push(asked);
                } else if merged {
                    model.merged.insert(asked.url());
                }
            }
            Transition::KilledAdmit(asked, _) => model.last_answer = model.admitted(asked).0,
            Transition::Fetch(index, distinct) if !model.added.is_empty() => {
                let id = index.index(model.added.len()) + 1;
                let page = model.added[id - 1];
                model.fetch(id, text(page, id, distinct));
            }
            Transition::Fetch(..) | Transition::KilledFetch(..) | Transition::Crash(_) => {}
        }
        model
    }
}

/// Admission and learning under test: a run file on the crash VFS and its
/// writer.
struct Subject {
    runtime: Runtime,
    _env: TestEnv,
    vfs: CrashVfs,
    path: String,
    owner: TaskOwner,
    state: Option<RunState>,
}

impl Subject {
    fn state(&self) -> RunState {
        self.state
            .clone()
            .expect("invariant: the writer is open between steps")
    }

    /// The next process opens the run file.
    fn open(&mut self) {
        let options = RunStateOptions {
            vfs: Some(self.vfs.name().to_owned()),
            ..RunStateOptions::default()
        };
        let state = self.runtime.block_on(RunState::open(
            self.path.clone().into(),
            &options,
            vec![
                identity::TABLES,
                scope::TABLES,
                bounds::TABLES,
                rules::TABLES,
                REQUESTS,
            ],
            1,
            &self.owner,
        ));
        self.state = Some(state.unwrap());
    }

    fn close(&mut self) {
        self.state = None;
        self.runtime.block_on(self.owner.wait());
    }

    /// The crashed writer stops, and the next process opens the file,
    /// which passes `PRAGMA integrity_check`.
    fn reopen(&mut self) {
        self.close();
        self.owner = TaskOwner::root(BlockingPolicy::Inline);
        drop(after_crash(&self.vfs, &self.path).unwrap());
        self.open();
    }
}

/// Admits `asked` as a seed, adding a pending request to the test's table.
fn admit_seed(connection: &Connection, asked: Asked) -> rusqlite::Result<Enqueued> {
    let method = HttpMethod::get();
    let url = asked.url();
    let candidate = Candidate::Seed {
        url: &url,
        declared: Declared {
            method: &method,
            body: None,
            key: None,
            keeps_fragment: false,
        },
    };
    let every_host = Policy::new(&["*"], &[] as &[&str]).unwrap();
    admit(
        connection,
        &Scope::read(connection)?,
        &every_host,
        candidate,
        |url| {
            connection.query_row(
                "INSERT INTO requests (id, url, status)
                 SELECT coalesce(max(id), 0) + 1, ?1, 'pending' FROM requests RETURNING id",
                [url.as_str()],
                |row| Ok(row.get::<_, i64>(0)?.to_string()),
            )
        },
    )
}

/// The test's crawl ending the requests a rule merged, as a crawl does:
/// each pending one, all but the first when none was fetched.
struct Requests;

impl EndMerged for Requests {
    fn end_merged(
        &mut self,
        connection: &Connection,
        crawl_requests: &BTreeSet<String>,
    ) -> rusqlite::Result<Vec<String>> {
        let mut rows = Vec::new();
        for id in crawl_requests {
            let id: i64 = id.parse().unwrap();
            rows.push(connection.query_row(
                "SELECT id, url, status FROM requests WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )?);
        }
        rows.sort();
        let fetched = rows.iter().any(|(_, _, status)| status == "fetched");
        let mut pending: Vec<_> = rows
            .into_iter()
            .filter(|(_, _, status)| status == "pending")
            .collect();
        if !fetched && !pending.is_empty() {
            pending.remove(0);
        }
        let mut ended = Vec::new();
        for (id, url, _) in pending {
            connection.execute("UPDATE requests SET status = 'skipped' WHERE id = ?1", [id])?;
            ended.push(url);
        }
        Ok(ended)
    }
}

/// Fetches request `id`'s page at `asked`, read with `text`, and learns
/// from it, unless a rule ended the request.
fn fetch(connection: &Connection, id: usize, asked: Asked, text: &str) -> rusqlite::Result<()> {
    let status: Option<String> = connection
        .query_row(
            "SELECT status FROM requests WHERE id = ?1",
            [i64::try_from(id).unwrap()],
            |row| row.get(0),
        )
        .optional()?;
    if status.as_deref() == Some("skipped") {
        return Ok(());
    }
    connection.execute(
        "UPDATE requests SET status = 'fetched' WHERE id = ?1",
        [i64::try_from(id).unwrap()],
    )?;
    let method = HttpMethod::get();
    learn(
        connection,
        PageSeen {
            crawl_request: &id.to_string(),
            url: &Url::parse(&asked.url()).unwrap(),
            method: &method,
            body: None,
            signature: Sha256::digest(text).into(),
            canonical: None,
        },
        &mut Requests,
    )
}

/// What the file holds: its requests' count, the rules that hold and
/// those stopped, every URL a rule merged and the requests skipped.
type Stored = (
    usize,
    BTreeSet<String>,
    BTreeSet<String>,
    BTreeSet<String>,
    BTreeSet<usize>,
);

fn stored(connection: &Connection) -> Stored {
    let added: i64 = connection
        .query_row("SELECT count(*) FROM requests", [], |row| row.get(0))
        .unwrap();
    let skipped = connection
        .prepare("SELECT id FROM requests WHERE status = 'skipped'")
        .unwrap()
        .query_map([], |row| row.get::<_, i64>(0))
        .unwrap()
        .map(|id| usize::try_from(id.unwrap()).unwrap())
        .collect();
    let rules = learned_rules(connection).unwrap();
    let named = |stopped: bool| {
        rules
            .iter()
            .filter(|rule| rule.contradiction.is_some() == stopped)
            .map(|rule| rule.parameter.clone().unwrap_or_else(|| SLASH.to_owned()))
            .collect()
    };
    (
        usize::try_from(added).unwrap(),
        named(false),
        named(true),
        rules.iter().flat_map(|rule| rule.merged.clone()).collect(),
        skipped,
    )
}

impl StateMachineTest for Subject {
    type SystemUnderTest = Self;
    type Reference = Model;

    fn init_test(_: &Model) -> Self {
        let env = TestEnv::new();
        let folder = env
            .folder()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace(' ', "-");
        let vfs = CrashVfs::register(&format!("learning-model-{folder}"), env.seed()).unwrap();
        let path = env
            .folder()
            .join("run.sqlite")
            .to_string_lossy()
            .into_owned();
        let mut subject = Self {
            runtime: goodfetch_test_support::runtime(),
            _env: env,
            vfs,
            path,
            owner: TaskOwner::root(BlockingPolicy::Inline),
            state: None,
        };
        subject.open();
        subject
    }

    fn apply(mut subject: Self, model: &Model, transition: Transition) -> Self {
        match transition {
            Transition::Admit(asked) => {
                let state = subject.state();
                let answer = subject
                    .runtime
                    .block_on(state.write(move |connection| admit_seed(connection, asked)))
                    .unwrap();
                assert_eq!(
                    match answer {
                        Enqueued::Added { id, .. } => Some(id.parse().unwrap()),
                        Enqueued::NotAdded { .. } => None,
                    },
                    model.last_answer,
                    "{asked:?} answered otherwise than the model says"
                );
            }
            Transition::KilledAdmit(asked, kind) => {
                let state = subject.state();
                let vfs = subject.vfs.clone();
                let killed = subject.runtime.block_on(state.write(move |connection| {
                    let answer = admit_seed(connection, asked);
                    vfs.crash(kind);
                    answer
                }));
                assert!(
                    killed.is_err() || model.last_answer.is_none(),
                    "an admission killed before its commit was acknowledged"
                );
                drop(state);
                subject.reopen();
            }
            Transition::Fetch(index, distinct) if !model.added.is_empty() => {
                let id = index.index(model.added.len()) + 1;
                let page = model.added[id - 1];
                let state = subject.state();
                let text = text(page, id, distinct);
                subject
                    .runtime
                    .block_on(state.write(move |connection| fetch(connection, id, page, &text)))
                    .unwrap();
            }
            Transition::KilledFetch(index, distinct, kind) if !model.added.is_empty() => {
                let id = index.index(model.added.len()) + 1;
                let page = model.added[id - 1];
                let state = subject.state();
                let vfs = subject.vfs.clone();
                let text = text(page, id, distinct);
                let killed = subject.runtime.block_on(state.write(move |connection| {
                    let learned = fetch(connection, id, page, &text);
                    vfs.crash(kind);
                    learned
                }));
                drop(killed);
                drop(state);
                subject.reopen();
            }
            Transition::Fetch(..) | Transition::KilledFetch(..) => {}
            Transition::Crash(kind) => {
                subject.vfs.crash(kind);
                subject.reopen();
            }
        }
        subject
    }

    fn check_invariants(subject: &Self, model: &Model) {
        let connection = Connection::open_with_flags_and_vfs(
            &subject.path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            subject.vfs.name(),
        )
        .unwrap();
        let names =
            |rules: &BTreeSet<&'static str>| rules.iter().map(|name| (*name).to_owned()).collect();
        assert_eq!(
            stored(&connection),
            (
                model.added.len(),
                names(&model.learned),
                names(&model.stopped),
                model.merged.clone(),
                model.skipped.clone(),
            )
        );
    }

    fn teardown(mut subject: Self, _: Model) {
        subject.close();
    }
}

prop_state_machine! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:6-216, story:6-217, story:6-218, story:6-222
    #[test]
    fn learned_rules_merge_and_stop_as_the_reference_model_says_through_kills(
        sequential 1..60 => Subject
    );
}
