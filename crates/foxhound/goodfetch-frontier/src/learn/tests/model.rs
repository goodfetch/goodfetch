//! The model-based test of the learned parameters (ADR 0060; TESTING.md,
//! Model-based tests and crashes in one process): seeds of one host's
//! `/item?n=<n>[&ref=<r>]` admitted into a run file on real SQLite through
//! the crash VFS, pages of admitted requests fetched with a page signature
//! that `ref` changes only now and then, and kills, between steps and in
//! the middle of an admission or a fetch's learning, before its commit. A
//! reference model, which knows only the spec's thresholds (3 pairs of
//! pages whose URLs differ only by a parameter, matching, and none
//! differing), says which parameters are learned, which candidate is
//! added and which is a `duplicate`, and which URLs a rule merged. After
//! every step and every crash, the run file holds what the model says, and
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
use rusqlite::Connection;
use sha2::{Digest as _, Sha256};
use tokio::runtime::Runtime;
use url::Url;

use crate::identity::{self, Declared};
use crate::rules::{self, learned_rules};
use crate::scope::{self, Scope};
use crate::{Candidate, PageSeen, admit, bounds, learn};

/// The crawl's own table of the test: each request added, by its ID.
const REQUESTS: RunStateTables = RunStateTables {
    name: "model-requests",
    statements: &["CREATE TABLE IF NOT EXISTS requests (
            id INTEGER NOT NULL PRIMARY KEY,
            url TEXT NOT NULL
        ) STRICT"],
};

/// A page the case asks for: item `n`, with `ref` or without.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Asked {
    n: u8,
    reference: Option<u8>,
}

impl Asked {
    fn url(self) -> String {
        match self.reference {
            Some(reference) => format!("http://shop.test/item?n={}&ref={reference}", self.n),
            None => format!("http://shop.test/item?n={}", self.n),
        }
    }

    /// The page without the parameters `learned` holds.
    fn without(self, learned: &BTreeSet<&'static str>) -> (Option<u8>, Option<u8>) {
        (
            (!learned.contains("n")).then_some(self.n),
            self.reference.filter(|_| !learned.contains("ref")),
        )
    }
}

fn asked() -> impl Strategy<Value = Asked> {
    (0_u8..3, proptest::option::weighted(0.6, 0_u8..3))
        .prop_map(|(n, reference)| Asked { n, reference })
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
    /// Each parameter's pairs: how many matched and how many differed.
    pairs: BTreeMap<&'static str, (u32, u32)>,
    /// The parameters learned to have no effect.
    learned: BTreeSet<&'static str>,
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
        let key = asked.without(&self.learned);
        let duplicate = self
            .added
            .iter()
            .any(|added| added.without(&self.learned) == key);
        let merged = duplicate && !self.added.contains(&asked);
        ((!duplicate).then_some(self.added.len() + 1), merged)
    }

    /// Learns from request `id`'s page, read with `text`, once.
    fn fetch(&mut self, id: usize, text: String) {
        if self.fetched.contains_key(&id) {
            return;
        }
        let page = self.added[id - 1];
        for (other_id, other_text) in &self.fetched {
            let other = self.added[other_id - 1];
            let parameter = if other.n == page.n && other.reference != page.reference {
                "ref"
            } else if other.reference == page.reference && other.n != page.n {
                "n"
            } else {
                continue;
            };
            let counts = self.pairs.entry(parameter).or_default();
            if *other_text == text {
                counts.0 += 1;
            } else {
                counts.1 += 1;
            }
        }
        self.fetched.insert(id, text);
        for (parameter, (matching, differing)) in &self.pairs {
            if *matching >= 3 && *differing == 0 {
                self.learned.insert(parameter);
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

/// Admits `asked` as a seed, adding a request to the test's table.
fn admit_seed(connection: &Connection, asked: Asked) -> rusqlite::Result<Enqueued> {
    let method = HttpMethod::get();
    let url = asked.url();
    let candidate = Candidate::Seed {
        url: &url,
        declared: Declared {
            method: &method,
            body: None,
            key: None,
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
                "INSERT INTO requests (id, url)
             SELECT coalesce(max(id), 0) + 1, ?1 FROM requests RETURNING id",
                [url.as_str()],
                |row| Ok(row.get::<_, i64>(0)?.to_string()),
            )
        },
    )
}

/// Learns from request `id`'s page at `asked`, read with `text`.
fn fetch(connection: &Connection, id: usize, asked: Asked, text: &str) -> rusqlite::Result<()> {
    let method = HttpMethod::get();
    learn(
        connection,
        PageSeen {
            crawl_request: &id.to_string(),
            url: &Url::parse(&asked.url()).unwrap(),
            method: &method,
            body: None,
            signature: Sha256::digest(text).into(),
        },
    )
}

/// What the file holds: its requests' count, the parameters learned and
/// every URL a rule merged.
fn stored(connection: &Connection) -> (usize, BTreeSet<String>, BTreeSet<String>) {
    let added: i64 = connection
        .query_row("SELECT count(*) FROM requests", [], |row| row.get(0))
        .unwrap();
    let rules = learned_rules(connection).unwrap();
    (
        usize::try_from(added).unwrap(),
        rules
            .iter()
            .filter_map(|rule| rule.parameter.clone())
            .collect(),
        rules.into_iter().flat_map(|rule| rule.merged).collect(),
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
        assert_eq!(
            stored(&connection),
            (
                model.added.len(),
                model
                    .learned
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect(),
                model.merged.clone(),
            )
        );
    }

    fn teardown(mut subject: Self, _: Model) {
        subject.close();
    }
}

prop_state_machine! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:6-216
    #[test]
    fn learned_parameters_merge_as_the_reference_model_says_through_kills(
        sequential 1..60 => Subject
    );
}
