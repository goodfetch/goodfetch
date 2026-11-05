//! The order a crawl's pending work items wait in, in the run file, on
//! real SQLite through the crash VFS (ADR 0060; TESTING.md, Model-based
//! tests and crashes in one process): requests of three domains come with
//! any priority and `forefront`, are leased each domain's next first, and
//! end done or back to pending, until the process crashes or the power
//! goes. Each step reads as the delivery loop does, through the run file's
//! readers beside its writer (ADR 0138): the requests not keyed yet
//! (`unkeyed`) after the write that added them, a domain's next
//! (`waiting`) before its lease, and whether any is pending
//! (`any_pending`) after every step, each seeing the writes acknowledged
//! before it. A reference lines each domain's requests up by hand; every
//! lease takes the one it puts next, and after the crash the file the next
//! process opens, its leases recovered and its per-domain queues filled
//! again as a resume fills them, admits each domain's requests in the
//! reference's order. 16 cases in the Quick suite and 256 in the Pre-merge
//! suite (`proptest_config`).
#![expect(
    clippy::disallowed_methods,
    reason = "the queues' readings may start from any moment (ADR 0059)"
)]

use std::collections::BTreeMap;

use goodfetch_engine::admission::{Ended, Gate as _};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::run_state::{RunState, RunStateOptions};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_frontier::{
    BOUNDS_TABLES, Candidate, Declared, DomainKeys, DomainQueues, SCOPE_TABLES, Scope,
    VISITED_TABLES, admit,
};
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use goodfetch_protocol::run::PointInTime;
use goodfetch_test_support::{CrashKind, CrashVfs, TestEnv, after_crash};
use proptest::prelude::*;
use proptest::sample::Index;
use rusqlite::Connection;

use super::{Adding, Failure, TABLES, Unsuccessful};

/// The bound of each domain's queue after the crash, small so that a
/// queue drains and fills again from the run file.
const BOUND: usize = 2;

#[derive(Clone, Debug)]
enum Step {
    /// A request of one of three domains comes.
    Comes {
        domain: usize,
        priority: i64,
        forefront: bool,
    },
    /// The next pending request of a domain is leased.
    Lease(usize),
    /// A leased request ends, done or back to pending for a retry.
    Ends { attempt: Index, retried: bool },
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (0_usize..3, -2_i64..=2, any::<bool>()).prop_map(|(domain, priority, forefront)| Step::Comes { domain, priority, forefront }),
        2 => (0_usize..3).prop_map(Step::Lease),
        2 => (any::<Index>(), any::<bool>()).prop_map(|(attempt, retried)| Step::Ends { attempt, retried }),
    ]
}

/// A request in the reference's line of its domain.
#[derive(Clone, Debug)]
struct Lined {
    url: String,
    priority: i64,
    forefront: bool,
    leased: bool,
}

/// Lines `request` up by hand: a `forefront` one in front of the line, any
/// other one behind the `forefront` ones and every one of its priority or
/// higher.
fn line_up(line: &mut Vec<Lined>, request: Lined) {
    let at = if request.forefront {
        0
    } else {
        line.iter()
            .position(|lined| !lined.forefront && lined.priority < request.priority)
            .unwrap_or(line.len())
    };
    line.insert(at, request);
}

fn domain_url(domain: usize, request: usize) -> String {
    format!("https://d{domain}.test/{request}")
}

/// The run file under test, on the crash VFS, and its writer.
struct Subject {
    runtime: tokio::runtime::Runtime,
    _env: TestEnv,
    vfs: CrashVfs,
    path: String,
    owner: TaskOwner,
    state: Option<RunState>,
    keys: DomainKeys,
}

impl Subject {
    fn open() -> Self {
        let env = TestEnv::new();
        let folder = env
            .folder()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace(' ', "-");
        let vfs = CrashVfs::register(&format!("order-{folder}"), env.seed()).unwrap();
        let runtime = goodfetch_test_support::runtime();
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let path = env.folder().join("run.sqlite");
        let state = runtime
            .block_on(RunState::open(
                path.clone(),
                &RunStateOptions {
                    vfs: Some(vfs.name().to_owned()),
                    ..RunStateOptions::default()
                },
                vec![
                    TABLES,
                    goodfetch_dataset::TABLES,
                    VISITED_TABLES,
                    SCOPE_TABLES,
                    BOUNDS_TABLES,
                    goodfetch_frontier::RULES_TABLES,
                    goodfetch_frontier::payload::TABLES,
                    goodfetch_frontier::sitemap::TABLES,
                    goodfetch_frontier::trap_links::TABLES,
                ],
                1,
                &owner,
            ))
            .unwrap();
        Self {
            runtime,
            _env: env,
            vfs,
            path: path.to_string_lossy().into_owned(),
            owner,
            state: Some(state),
            keys: DomainKeys::default(),
        }
    }

    fn write<T: Send + 'static>(
        &self,
        write: impl FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> T {
        let state = self
            .state
            .clone()
            .expect("invariant: the writer is open until the crash");
        self.runtime.block_on(state.write(write)).unwrap()
    }

    /// Reads the run file's committed state as the delivery loop does,
    /// through its readers, beside the writer.
    fn read<T: Send + 'static>(
        &self,
        read: impl FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> T {
        let state = self
            .state
            .clone()
            .expect("invariant: the writer is open until the crash");
        self.runtime.block_on(state.read(read)).unwrap()
    }

    /// Whether any request is pending, as the delivery loop asks when idle.
    fn any_pending(&self) -> bool {
        self.read(super::any_pending)
    }

    /// Adds a request of `url`, then reads the requests not keyed yet and
    /// keys them, as the delivery does.
    fn comes(&self, url: String, priority: i64, forefront: bool) {
        let key = self.keys.key(&url);
        self.write(move |connection| {
            let get = HttpMethod::get();
            let declared = Declared {
                method: &get,
                body: None,
                key: None,
                keeps_fragment: false,
            };
            admit(
                connection,
                &Scope::default(),
                &Policy::new(&["*"], &[] as &[&str]).unwrap(),
                Candidate::Seed {
                    url: &url,
                    declared,
                },
                |request| {
                    super::add(
                        connection,
                        &mut super::Positions::of(connection)?,
                        request,
                        Adding {
                            method: &get,
                            body: None,
                            label: "page",
                            payload: None,
                            priority,
                            forefront,
                            depth: 0,
                        },
                    )
                },
            )
            .map(drop)
        });
        let keys: Vec<(String, String)> = self
            .read(|connection| super::unkeyed(connection, usize::MAX))
            .into_iter()
            .map(|(_, waiting)| (waiting.id, key.clone()))
            .collect();
        assert_eq!(
            keys.len(),
            1,
            "the request just added is read not keyed yet"
        );
        self.write(move |connection| super::set_keys(connection, &keys));
    }

    /// Leases the next pending request of `url`'s domain, as its queue fills
    /// from the run file: its URL, ID and lease epoch.
    fn lease(&self, url: &str) -> Option<(String, String, u64)> {
        let key = self.keys.key(url);
        let next: Vec<String> = self
            .read(move |connection| super::waiting(connection, Some(key.as_str()), 1))
            .into_iter()
            .map(|waiting| waiting.id)
            .collect();
        self.write(move |connection| {
            Ok(super::lease(connection, &next)?
                .into_iter()
                .next()
                .map(|leased| {
                    (
                        leased.crawl_request.url,
                        leased.crawl_request.id,
                        leased.lease_epoch,
                    )
                }))
        })
    }

    /// Ends the attempt of `id` under `lease_epoch`, done or retried.
    fn ends(&self, id: String, lease_epoch: u64, retried: bool) {
        self.write(move |connection| {
            let ended_at = PointInTime::from_rfc3339("2026-01-05T09:00:00Z".to_owned());
            if retried {
                super::end(
                    connection,
                    &id,
                    lease_epoch,
                    Unsuccessful::Retried(Failure {
                        code: "response_unavailable".to_owned(),
                        message: "a 503".to_owned(),
                        details: serde_json::Map::new(),
                    }),
                    &ended_at,
                )
            } else {
                super::commit(
                    connection,
                    &id,
                    lease_epoch,
                    super::Committed {
                        records: &goodfetch_dataset::Checked::default(),
                        fetched: &super::Fetched::default(),
                        fields: &super::FieldsRead::default(),
                        masks: &goodfetch_engine::mask::Masks::new(),
                        ended_at: &ended_at,
                        not_added: &[],
                        work: &goodfetch_counted_work::CountedWork::default(),
                        keeps_records: true,
                    },
                )
            }
        });
    }

    /// Crashes, then opens the file the next process opens, recovers its
    /// leases and admits every pending request through per-domain queues
    /// filled as a resume fills them: each domain's URLs in admission order.
    fn crash_and_resume(mut self, kind: CrashKind) -> BTreeMap<String, Vec<String>> {
        self.vfs.crash(kind);
        self.state = None;
        self.runtime.block_on(self.owner.wait());
        let connection = after_crash(&self.vfs, &self.path).unwrap();
        let lost = Failure::new("handler_lost", "the handler was lost");
        let at = PointInTime::from_rfc3339("2026-01-05T09:00:00Z".to_owned());
        super::recover_leases(&connection, &lost, u32::MAX, &at).unwrap();
        let now = tokio::time::Instant::now();
        let mut queues = DomainQueues::new(DomainKeys::default(), BOUND, Entropy::seeded(1));
        for waiting in super::waiting(&connection, None, BOUND).unwrap() {
            queues.enqueue(waiting);
        }
        let mut admitted: BTreeMap<String, Vec<String>> = BTreeMap::new();
        while let Some(next) = queues.admit(now) {
            let leased = super::lease(&connection, &[next.id]).unwrap();
            queues.ended(&next.key, now, Ended::Done);
            for leased in leased {
                admitted
                    .entry(next.key.clone())
                    .or_default()
                    .push(leased.crawl_request.url);
            }
            if next.drained {
                for waiting in super::waiting(&connection, Some(next.key.as_str()), BOUND).unwrap()
                {
                    queues.enqueue(waiting);
                }
            }
        }
        admitted
    }
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:6-215
    #[test]
    fn pending_requests_keep_their_order_in_each_domain_through_a_crash_and_a_resume(
        steps in proptest::collection::vec(step(), 1..40),
        crash in prop_oneof![Just(CrashKind::Process), Just(CrashKind::PowerCut)],
    ) {
        let subject = Subject::open();
        let mut lines: [Vec<Lined>; 3] = Default::default();
        let mut attempts: Vec<(usize, String, String, u64)> = Vec::new();
        for (request, step) in steps.into_iter().enumerate() {
            match step {
                Step::Comes { domain, priority, forefront } => {
                    let url = domain_url(domain, request);
                    subject.comes(url.clone(), priority, forefront);
                    line_up(&mut lines[domain], Lined { url, priority, forefront, leased: false });
                }
                Step::Lease(domain) => {
                    let leased = subject.lease(&domain_url(domain, 0));
                    let next = lines[domain].iter_mut().find(|lined| !lined.leased);
                    prop_assert_eq!(
                        leased.as_ref().map(|(url, _, _)| url),
                        next.as_ref().map(|lined| &lined.url)
                    );
                    if let (Some((url, id, epoch)), Some(next)) = (leased, next) {
                        next.leased = true;
                        attempts.push((domain, url, id, epoch));
                    }
                }
                Step::Ends { attempt, retried } => {
                    if attempts.is_empty() {
                        continue;
                    }
                    let (domain, url, id, epoch) = attempts.remove(attempt.index(attempts.len()));
                    subject.ends(id, epoch, retried);
                    let lined = lines[domain].iter().position(|lined| lined.url == url).unwrap();
                    if retried {
                        lines[domain][lined].leased = false;
                    } else {
                        lines[domain].remove(lined);
                    }
                }
            }
            let pending = lines.iter().flatten().any(|lined| !lined.leased);
            prop_assert_eq!(subject.any_pending(), pending);
        }

        let admitted = subject.crash_and_resume(crash);

        let expected: BTreeMap<String, Vec<String>> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| !line.is_empty())
            .map(|(domain, line)| {
                (format!("d{domain}.test"), line.iter().map(|lined| lined.url.clone()).collect())
            })
            .collect();
        prop_assert_eq!(admitted, expected);
    }
}
