//! The model-based test of admission and identity (ADR 0060; TESTING.md,
//! Model-based tests and crashes in one process): seeds admitted into a
//! run file on real SQLite through the crash VFS, each spelled one of the
//! ways the URL key merges (tracking parameters, parameter order, the
//! fragment, `;jsessionid=` and empty pairs), sent with `GET` or `POST`
//! and a body or none, or under a declared key; redirect hops checked and
//! their final URLs recorded; and kills, between admissions and in the
//! middle of one, before its commit. A reference model, which knows each
//! candidate's identity from how the case built it, says which candidate
//! is added and which is a `duplicate`, and where each hop leads. After
//! every step and every crash, the run file holds the model's requests and
//! visited set, nothing of a killed admission, and the next process admits
//! on from there; `after_crash` checks `PRAGMA integrity_check`. 16 cases
//! in the Quick suite and 256 in the Pre-merge suite (`proptest_config`).

use std::collections::BTreeMap;

use goodfetch_engine::egress::Policy;
use goodfetch_engine::run_state::{RunState, RunStateOptions, RunStateTables};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod};
use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv, after_crash};
use proptest::prelude::*;
use proptest::sample::Index;
use proptest_state_machine::{ReferenceStateMachine, StateMachineTest, prop_state_machine};
use rusqlite::Connection;
use tokio::runtime::Runtime;
use url::Url;

use crate::bounds::{self, Bounds};
use crate::identity::{self, Declared, Hop};
use crate::invariants::Invariants;
use crate::scope::{self, Scope};
use crate::{Candidate, admit};

use super::every_host;

/// The crawl's own table of the test: each request added, by its ID.
const REQUESTS: RunStateTables = RunStateTables {
    name: "model-requests",
    statements: &["CREATE TABLE IF NOT EXISTS requests (
            id INTEGER NOT NULL PRIMARY KEY,
            url TEXT NOT NULL
        ) STRICT"],
};

/// One of the spellings the key makes one URL of.
#[derive(Clone, Copy, Debug)]
enum Spelling {
    Plain,
    Tracking,
    Reordered,
    Fragment,
    Session,
}

/// A request the case asks for: a page in a spelling, a method, a body
/// and a declared key, each from a small set, so that identities meet.
#[derive(Clone, Copy, Debug)]
struct Asked {
    page: u8,
    spelling: Spelling,
    post: bool,
    body: Option<u8>,
    key: Option<u8>,
}

/// A request's identity as the model knows it, from how the case built it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Identity {
    Declared(u8),
    Made {
        post: bool,
        page: u8,
        body: Option<u8>,
    },
}

impl Asked {
    fn url(self) -> String {
        let page = format!("http://shop.test/page/{}", self.page);
        match self.spelling {
            Spelling::Plain => format!("{page}?a=1&b=2"),
            Spelling::Tracking => format!("{page}?utm_source=mail&a=1&gclid=x&b=2&fbclid=y"),
            Spelling::Reordered => format!("{page}?b=2&a=1"),
            Spelling::Fragment => format!("{page}?a=1&b=2#reviews"),
            Spelling::Session => format!("{page};jsessionid=0A1B?&a=1&&b=2&"),
        }
    }

    fn body(self) -> Option<Vec<u8>> {
        self.body
            .map(|body| format!("{{\"page\":{body}}}").into_bytes())
    }

    fn identity(self) -> Identity {
        match self.key {
            Some(key) => Identity::Declared(key),
            None => Identity::Made {
                post: self.post,
                page: self.page,
                body: self.body,
            },
        }
    }

    /// Its identity as a redirect hop has it: a hop declares no key.
    fn hop_identity(self) -> Identity {
        Self { key: None, ..self }.identity()
    }
}

fn asked() -> impl Strategy<Value = Asked> {
    (
        0_u8..3,
        prop_oneof![
            Just(Spelling::Plain),
            Just(Spelling::Tracking),
            Just(Spelling::Reordered),
            Just(Spelling::Fragment),
            Just(Spelling::Session),
        ],
        any::<bool>(),
        proptest::option::of(0_u8..2),
        proptest::option::weighted(0.2, 0_u8..2),
    )
        .prop_map(|(page, spelling, post, body, key)| Asked {
            page,
            spelling,
            post,
            body,
            key,
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
    /// before its commit; the next process opens the run file.
    KilledAdmit(Asked, CrashKind),
    /// A redirect hop of an added request to a URL: followed, its final
    /// URL recorded, or a duplicate.
    Redirect(Index, Asked),
    /// The process crashes or the power goes between steps; the next
    /// process opens the run file.
    Crash(CrashKind),
}

/// What the run file must hold: the requests added, numbered from 1, and
/// the visited set, each identity with the request it was admitted for or
/// that a redirect led to it.
#[derive(Clone, Debug, Default)]
struct Model {
    added: u32,
    visited: BTreeMap<Identity, u32>,
    /// The answer the last admission must get: its ID, or `None` for a
    /// duplicate.
    last_answer: Option<u32>,
    /// Where the last redirect hop must lead.
    last_hop: Option<Hop>,
}

impl Model {
    /// The answer to `asked`: the ID it is added with, or `None` for a
    /// duplicate.
    fn admitted(&self, asked: Asked) -> Option<u32> {
        (!self.visited.contains_key(&asked.identity())).then_some(self.added + 1)
    }

    /// Where a hop of request `from` to `to` leads.
    fn hop(&self, from: u32, to: Asked) -> Hop {
        match self.visited.get(&to.hop_identity()) {
            Some(visitor) if *visitor != from => Hop::Duplicate,
            _ => Hop::Follow,
        }
    }

    /// The visited set's count of identities per request, as the file must
    /// hold it.
    fn per_request(&self) -> BTreeMap<u32, u32> {
        let mut counted = BTreeMap::new();
        for visitor in self.visited.values() {
            *counted.entry(*visitor).or_default() += 1;
        }
        counted
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
            6 => asked().prop_map(Transition::Admit),
            2 => (asked(), kind()).prop_map(|(asked, kind)| Transition::KilledAdmit(asked, kind)),
            3 => (any::<Index>(), asked()).prop_map(|(from, to)| Transition::Redirect(from, to)),
            1 => kind().prop_map(Transition::Crash),
        ]
        .boxed()
    }

    fn apply(mut model: Self, transition: &Transition) -> Self {
        match *transition {
            Transition::Admit(asked) => {
                model.last_answer = model.admitted(asked);
                if let Some(id) = model.last_answer {
                    model.added = id;
                    model.visited.insert(asked.identity(), id);
                }
            }
            Transition::Redirect(from, to) if model.added > 0 => {
                let from = from_id(from, model.added);
                let hop = model.hop(from, to);
                model.last_hop = Some(hop);
                if hop == Hop::Follow {
                    model.visited.entry(to.hop_identity()).or_insert(from);
                }
            }
            Transition::KilledAdmit(asked, _) => model.last_answer = model.admitted(asked),
            Transition::Redirect(..) | Transition::Crash(_) => {}
        }
        model
    }
}

/// The added request a hop starts from.
fn from_id(index: Index, added: u32) -> u32 {
    u32::try_from(index.index(usize::try_from(added).unwrap())).unwrap() + 1
}

/// Admission under test: a run file on the crash VFS and its writer.
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
        let path = self.path.clone().into();
        let options = RunStateOptions {
            vfs: Some(self.vfs.name().to_owned()),
            ..RunStateOptions::default()
        };
        let state = self.runtime.block_on(RunState::open(
            path,
            &options,
            vec![
                identity::TABLES,
                scope::TABLES,
                bounds::TABLES,
                crate::rules::TABLES,
                REQUESTS,
            ],
            1,
            &self.owner,
        ));
        self.state = Some(state.unwrap());
    }

    /// The writer stops and the test's tasks end.
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
    let method = if asked.post {
        HttpMethod::parse("POST").unwrap()
    } else {
        HttpMethod::get()
    };
    let body = asked.body();
    let key = asked.key.map(|key| format!("item-{key}"));
    let url = asked.url();
    let candidate = Candidate::Seed {
        url: &url,
        declared: Declared {
            method: &method,
            body: body.as_deref(),
            key: key.as_deref(),
            keeps_fragment: false,
        },
    };
    admit(
        connection,
        &Invariants::new(Scope::read(connection)?, Bounds::read(connection)?).unwrap(),
        &every_host(),
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

/// The requests the file holds, and its visited set's count of identities
/// per request.
fn stored(connection: &Connection) -> (u32, BTreeMap<u32, u32>) {
    let added = connection
        .query_row("SELECT count(*) FROM requests", [], |row| row.get(0))
        .unwrap();
    let per_request = connection
        .prepare("SELECT crawl_request, count(*) FROM visited GROUP BY crawl_request")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?.parse().unwrap(),
                row.get::<_, u32>(1)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    (added, per_request)
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
        let vfs = CrashVfs::register(&format!("admission-model-{folder}"), env.seed()).unwrap();
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
                let expected = model.last_answer;
                assert_eq!(
                    match answer {
                        Enqueued::Added { id, .. } => Some(id.parse().unwrap()),
                        Enqueued::NotAdded { .. } => None,
                    },
                    expected,
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
                // A killed duplicate wrote nothing of its own, so whether
                // its empty commit was acknowledged changes nothing.
                assert!(
                    killed.is_err() || model.last_answer.is_none(),
                    "an admission killed before its commit was acknowledged"
                );
                drop(state);
                subject.reopen();
            }
            Transition::Redirect(from, to) if model.added > 0 => {
                let from = from_id(from, model.added);
                let state = subject.state();
                let hop = subject
                    .runtime
                    .block_on(state.write(move |connection| {
                        let method = if to.post {
                            HttpMethod::parse("POST").unwrap()
                        } else {
                            HttpMethod::get()
                        };
                        let url = Url::parse(&to.url()).unwrap();
                        let body = to.body();
                        let crawl_request = from.to_string();
                        let hop = identity::redirect_hop(
                            connection,
                            &Scope::read(connection)?,
                            &Policy::new(&["*"], &[] as &[&str])
                                .expect("invariant: every host is an entry"),
                            &crawl_request,
                            &url,
                            &method,
                            body.as_deref(),
                        )?;
                        if hop == Hop::Follow {
                            return identity::redirect_reached(
                                connection,
                                &crawl_request,
                                &url,
                                &method,
                                body.as_deref(),
                            );
                        }
                        Ok(hop)
                    }))
                    .unwrap();
                assert_eq!(Some(hop), model.last_hop, "a hop of {from} to {to:?}");
            }
            Transition::Redirect(..) => {}
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
        assert_eq!(stored(&connection), (model.added, model.per_request()));
    }

    fn teardown(mut subject: Self, _: Model) {
        subject.close();
    }
}

prop_state_machine! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:6-212, story:6-213, story:6-214, story:6-220
    #[test]
    fn admission_keeps_one_request_per_identity_through_redirects_and_kills_mid_admission(
        sequential 1..40 => Subject
    );
}
