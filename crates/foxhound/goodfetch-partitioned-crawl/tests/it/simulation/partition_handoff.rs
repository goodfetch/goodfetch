//! Partition handoff Simulation under Group membership changes (member added,
//! member removed, `runs adopt` fallback, stale forward re-routing, and
//! network partitions) with TLA+ model trace validation (`goodfetch::partition`,
//! ADRs 0054, 0055, 0060 and 0087, issue 249).
//!
//! Three simulated Group members (`lead`, `worker-a`, `worker-b`) run on
//! `turmoil`:
//! 1. Epoch 1 starts with `lead` and `worker-a` owning 6 domains via real
//!    SHA-256 rendezvous hashing (`PartitionMap`). Each Partition fetches URLs
//!    with Wall allowance refetch-stop per-domain state attached to `walled.test`.
//! 2. Membership addition (`worker-b` joins) publishes Epoch 2: ONLY domains
//!    won by `worker-b` move. Each moving domain is fenced on its old owner,
//!    drains in-flight fetches to 0, transfers pending URLs, seen fingerprints,
//!    and per-domain state over turmoil TCP, waits for ack, and is admitted on
//!    `worker-b` ONLY after ack.
//! 3. A stale forward routed under Epoch 1 arrives at an old owner and is
//!    re-routed under Epoch 2 to the current owner.
//! 4. Membership removal (`worker-a` leaves) publishes Epoch 3: domains
//!    covered by the drain grace hand off live, while any domain not covered
//!    by the grace period is completed by `runs adopt` from `worker-a`'s kept
//!    state, preserving seen fingerprints and per-domain Wall allowance state.
//!
//! Invariants judged on every run:
//! - **No domain is fetched by two Partitions at once** (`concurrent_fetch_violations == 0`).
//! - **No already-seen URL is re-fetched after handoff** (`duplicate_fetches == 0`).
//! - **Per-domain state (Wall allowance refetch stop) moves with the domain key**
//!   on both live handoff and `runs adopt`.
//! - **Every transition emitted under `goodfetch::partition` matches
//!   `PartitionHandoff.tla`** (`TracePartitionHandoff.tla`).
#![expect(
    clippy::disallowed_methods,
    reason = "turmoil host servers run !Send futures on LocalSet inside the Simulation (ADR 0060)"
)]

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::entropy::Entropy;
use goodfetch_partitioned_crawl::forwarding::{
    ForwardLinksRequest, ForwardLinksResponse, ForwardedLink, classify_incoming,
};
use goodfetch_partitioned_crawl::handoff::{
    ActivateDomainRequest, HandoffDomainRequest, HandoffDomainResponse, TARGET,
    TransferredRefetchStop, compute_moves, map_with_added_member, map_without_member,
    trace_ack_handoff, trace_admit_domain, trace_adopt_handoff, trace_assign_initial,
    trace_drain_inflight, trace_fence_domain, trace_finish_fetch, trace_publish_map,
    trace_reroute_forward, trace_send_handoff, trace_start_fetch,
};
use goodfetch_partitioned_crawl::rendezvous::{PartitionMap, PartitionSlot};
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use goodfetch_protocol::run::RunId;
use goodfetch_simulation::turmoil;
use goodfetch_simulation::{Fault, Host, Injection, Simulation};
use goodfetch_test_support::{
    CapturedLog, ObservationLog, Operation, Record, Replay, TestEnv, check_determinism,
    model_trace, model_traces,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tracing_subscriber::layer::SubscriberExt as _;

const SCENARIO: &str = "partition_handoff";
const MODEL: &str = "partition-handoff";
const LEAD: &str = "lead";
const WORKER_A: &str = "worker-a";
const WORKER_B: &str = "worker-b";
const PORT: u16 = 9678;
const RPC_TIMEOUT: Duration = Duration::from_millis(120);

const DOMAINS: [&str; 6] = [
    "alpha.test",
    "beta.test",
    "gamma.test",
    "delta.test",
    "epsilon.test",
    "walled.test",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Seen {
    MapPublished {
        epoch: u64,
    },
    DomainHandedOff {
        domain: String,
        from: String,
        to: String,
        epoch: u64,
        adopted: bool,
    },
    ForwardRerouted {
        domain: String,
        from: String,
        to: String,
        old_epoch: u64,
        new_epoch: u64,
    },
    Fetched {
        member: String,
        domain: String,
        url: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WireFrame {
    Handoff(HandoffDomainRequest),
    Activate(ActivateDomainRequest),
    Forward(ForwardLinksRequest),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WireReply {
    Handoff(HandoffDomainResponse),
    Activated { ok: bool },
    Forward(ForwardLinksResponse),
}

#[derive(Default)]
struct MemberState {
    admitting: BTreeSet<String>,
    pending: BTreeMap<String, Vec<String>>,
    seen: BTreeMap<String, BTreeSet<String>>,
    refetch_stops: BTreeMap<String, Vec<TransferredRefetchStop>>,
    domain_state: BTreeMap<String, BTreeMap<String, Value>>,
    in_flight: BTreeSet<String>,
}

#[derive(Default)]
struct SimState {
    map: Option<PartitionMap>,
    members: BTreeMap<String, MemberState>,
    /// Currently fetching members per domain (invariant: len <= 1 at ALL times).
    active_fetchers: BTreeMap<String, BTreeSet<String>>,
    /// Count of any concurrent-fetch invariant violation (must stay 0).
    concurrent_fetch_violations: u64,
    /// URLs fetched across the entire crawl (invariant: each URL fetched once).
    fetched_urls: BTreeMap<String, u64>,
    /// Number of stale forwards successfully re-routed.
    rerouted_forwards: u64,
    /// Number of live handoffs completed.
    live_handoffs: u64,
    /// Number of `runs adopt` handoffs completed.
    adopted_handoffs: u64,
    /// Set by `coordinator_client` once the scenario has completed so member
    /// hosts stay online for every retry and exit cleanly together.
    done: bool,
}

#[derive(Clone)]
struct World {
    start: SystemTime,
    crawl_id: RunId,
    state: Rc<RefCell<SimState>>,
    log: Rc<RefCell<ObservationLog<Seen>>>,
}

struct HandoffRun {
    log: ObservationLog<Seen>,
    trace: Vec<u8>,
    concurrent_fetch_violations: u64,
    duplicate_fetches: u64,
    rerouted_forwards: u64,
    live_handoffs: u64,
    adopted_handoffs: u64,
    walled_stop_holder: Option<String>,
    walled_final_owner: String,
    walled_state_preserved: bool,
}

impl World {
    fn new(start: SystemTime) -> Self {
        let crawl_id = RunId::parse("sim-ph-1").unwrap();
        let map1 = PartitionMap::new(
            1,
            vec![
                PartitionSlot {
                    partition: 0,
                    member: LEAD.to_owned(),
                    run_id: RunId::parse("sim-ph-1-p0").unwrap(),
                    peer_address: format!("{LEAD}:{PORT}"),
                },
                PartitionSlot {
                    partition: 1,
                    member: WORKER_A.to_owned(),
                    run_id: RunId::parse("sim-ph-1-p1").unwrap(),
                    peer_address: format!("{WORKER_A}:{PORT}"),
                },
            ],
        );

        let mut st = SimState {
            map: Some(map1.clone()),
            ..SimState::default()
        };
        for m in [LEAD, WORKER_A, WORKER_B] {
            st.members.insert(m.to_owned(), MemberState::default());
        }
        for domain in DOMAINS {
            let owner = map1.owner_of(domain);
            let ms = st.members.get_mut(&owner.member).unwrap();
            ms.admitting.insert(domain.to_owned());
            ms.pending.insert(
                domain.to_owned(),
                vec![
                    format!("https://{domain}/page-1"),
                    format!("https://{domain}/page-2"),
                    format!("https://{domain}/page-3"),
                ],
            );
            if domain == "walled.test" {
                ms.refetch_stops.insert(
                    domain.to_owned(),
                    vec![TransferredRefetchStop {
                        domain: domain.to_owned(),
                        first_stopped_ms: 1_700_000_000_000,
                        last_run: owner.run_id.clone(),
                        cause: "challenge".to_owned(),
                        project_file: None,
                    }],
                );
                ms.domain_state.insert(
                    domain.to_owned(),
                    BTreeMap::from([(
                        "wall_allowance".to_owned(),
                        json!({ "refetch_stopped": true }),
                    )]),
                );
            }
        }

        Self {
            start,
            crawl_id,
            state: Rc::new(RefCell::new(st)),
            log: Rc::new(RefCell::new(ObservationLog::new())),
        }
    }

    fn record(&self, clock: &goodfetch_engine::clock::Clock, who: &str, seen: Seen) {
        let at = clock.wall().duration_since(self.start).unwrap_or_default();
        let at = u64::try_from(at.as_millis()).unwrap_or(u64::MAX);
        self.log
            .borrow_mut()
            .push(Record::new(who, Operation::Other(seen)).at(at));
    }

    fn current_map(&self) -> PartitionMap {
        self.state.borrow().map.clone().unwrap()
    }
}

async fn send_frame(target_host: &str, frame: &WireFrame) -> Result<WireReply, String> {
    let fut = async {
        let mut stream = turmoil::net::TcpStream::connect((target_host, PORT))
            .await
            .map_err(|e| e.to_string())?;
        let mut line = serde_json::to_vec(frame).map_err(|e| e.to_string())?;
        line.push(b'\n');
        stream.write_all(&line).await.map_err(|e| e.to_string())?;
        stream.flush().await.map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(stream);
        let mut resp = String::new();
        let n = reader
            .read_line(&mut resp)
            .await
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("closed".to_owned());
        }
        serde_json::from_str::<WireReply>(&resp).map_err(|e| e.to_string())
    };
    tokio::time::timeout(RPC_TIMEOUT, fut)
        .await
        .map_err(|_| "timeout".to_owned())?
}

async fn send_with_retry(target_host: &str, frame: &WireFrame) -> WireReply {
    loop {
        match send_frame(target_host, frame).await {
            Ok(reply) => return reply,
            Err(_) => tokio::time::sleep(Duration::from_millis(25)).await,
        }
    }
}

async fn handle_member_frame(
    member_name: &'static str,
    clock: &goodfetch_engine::clock::Clock,
    w: &World,
    frame: WireFrame,
) -> WireReply {
    match frame {
        WireFrame::Handoff(req) => handle_handoff_frame(member_name, w, &req),
        WireFrame::Activate(req) => {
            w.state
                .borrow_mut()
                .members
                .get_mut(member_name)
                .unwrap()
                .admitting
                .insert(req.domain_key.clone());
            trace_admit_domain(&req.domain_key, &req.member, req.map_epoch);
            WireReply::Activated { ok: true }
        }
        WireFrame::Forward(req) => handle_forward_frame(member_name, clock, w, req).await,
    }
}

fn handle_handoff_frame(
    member_name: &'static str,
    w: &World,
    req: &HandoffDomainRequest,
) -> WireReply {
    let domain = req.snapshot.domain_key.clone();
    let installed = {
        let mut st = w.state.borrow_mut();
        let ms = st.members.get_mut(member_name).unwrap();
        let seen_set = ms.seen.entry(domain.clone()).or_default();
        for fp in &req.snapshot.seen_fingerprints {
            seen_set.insert(fp.clone());
        }
        let pend = ms.pending.entry(domain.clone()).or_default();
        let mut added = 0_usize;
        for item in &req.snapshot.pending {
            if !seen_set.contains(&item.url) && !pend.contains(&item.url) {
                pend.push(item.url.clone());
                added += 1;
            }
        }
        if !req.refetch_stops.is_empty() {
            ms.refetch_stops
                .entry(domain.clone())
                .or_default()
                .extend(req.refetch_stops.clone());
        }
        if !req.snapshot.domain_state.is_empty() {
            ms.domain_state
                .entry(domain.clone())
                .or_default()
                .extend(req.snapshot.domain_state.clone());
        }
        added
    };
    if req.adopted {
        trace_adopt_handoff(&domain, &req.from_member, &req.to_member, req.map_epoch);
    } else {
        trace_ack_handoff(&domain, &req.from_member, &req.to_member, req.map_epoch);
    }
    WireReply::Handoff(HandoffDomainResponse {
        acked: true,
        map_epoch: req.map_epoch,
        installed,
    })
}

async fn handle_forward_frame(
    member_name: &'static str,
    clock: &goodfetch_engine::clock::Clock,
    w: &World,
    req: ForwardLinksRequest,
) -> WireReply {
    let map = w.current_map();
    let local_part = map
        .slots
        .iter()
        .find(|s| s.member == member_name)
        .map_or(req.target_partition, |s| s.partition);
    let (local_links, reroute) = classify_incoming(&map, local_part, req.links);
    let accepted = local_links.len();
    {
        let mut st = w.state.borrow_mut();
        let ms = st.members.get_mut(member_name).unwrap();
        for link in local_links {
            let seen = ms.seen.entry(link.domain_key.clone()).or_default();
            let pend = ms.pending.entry(link.domain_key).or_default();
            if !seen.contains(&link.url) && !pend.contains(&link.url) {
                pend.push(link.url);
            }
        }
    }
    let mut rerouted = 0_usize;
    for (next_part, links) in reroute {
        let Some(slot) = map.slot(next_part).cloned() else {
            continue;
        };
        for link in &links {
            trace_reroute_forward(
                &link.domain_key,
                member_name,
                &slot.member,
                req.partition_map_epoch,
                map.map_epoch,
            );
            w.record(
                clock,
                member_name,
                Seen::ForwardRerouted {
                    domain: link.domain_key.clone(),
                    from: member_name.to_owned(),
                    to: slot.member.clone(),
                    old_epoch: req.partition_map_epoch,
                    new_epoch: map.map_epoch,
                },
            );
        }
        rerouted += links.len();
        w.state.borrow_mut().rerouted_forwards += u64::try_from(links.len()).unwrap_or(0);
        let next_req = ForwardLinksRequest {
            partitioned_crawl_id: req.partitioned_crawl_id.clone(),
            target_partition: slot.partition,
            target_run_id: slot.run_id.clone(),
            sender_partition: local_part,
            sender_run_epoch: req.sender_run_epoch,
            partition_map_epoch: map.map_epoch,
            credits_requested: 16,
            links,
        };
        let _ = send_with_retry(&slot.member, &WireFrame::Forward(next_req)).await;
    }
    WireReply::Forward(ForwardLinksResponse {
        accepted,
        rerouted,
        credits_granted: 16,
        current_map_epoch: map.map_epoch,
    })
}

async fn member_host(member_name: &'static str, host: Host, world: World) -> turmoil::Result {
    let listener =
        turmoil::net::TcpListener::bind((IpAddr::V4(Ipv4Addr::UNSPECIFIED), PORT)).await?;
    let w = world.clone();
    let clock = *host.clock();
    let _server = tokio::task::spawn_local(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let w = w.clone();
            tokio::task::spawn_local(async move {
                let (r, mut wr) = stream.into_split();
                let mut reader = BufReader::new(r);
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    return;
                }
                let Ok(frame) = serde_json::from_str::<WireFrame>(&line) else {
                    return;
                };
                let reply = handle_member_frame(member_name, &clock, &w, frame).await;
                let mut out = serde_json::to_vec(&reply).unwrap();
                out.push(b'\n');
                let _ = wr.write_all(&out).await;
                let _ = wr.flush().await;
            });
        }
    });

    // Fetch loop: admits at most 1 URL at a time per admitting domain until
    // the coordinator signals scenario completion.
    while !world.state.borrow().done {
        let next_work: Option<(String, String)> = {
            let mut st = world.state.borrow_mut();
            let ms = st.members.get_mut(member_name).unwrap();
            let mut chosen = None;
            for domain in ms.admitting.clone() {
                if ms.in_flight.contains(&domain) {
                    continue;
                }
                if let Some(urls) = ms.pending.get_mut(&domain)
                    && !urls.is_empty()
                {
                    let url = urls.remove(0);
                    ms.in_flight.insert(domain.clone());
                    chosen = Some((domain, url));
                    break;
                }
            }
            if let Some((ref domain, _)) = chosen {
                let active = st.active_fetchers.entry(domain.clone()).or_default();
                active.insert(member_name.to_owned());
                if active.len() > 1 {
                    st.concurrent_fetch_violations += 1;
                }
            }
            chosen
        };
        if let Some((domain, url)) = next_work {
            trace_start_fetch(&domain, member_name);
            tokio::time::sleep(Duration::from_millis(20)).await;
            trace_finish_fetch(&domain, member_name);
            {
                let mut st = world.state.borrow_mut();
                if let Some(active) = st.active_fetchers.get_mut(&domain) {
                    active.remove(member_name);
                }
                let ms = st.members.get_mut(member_name).unwrap();
                ms.in_flight.remove(&domain);
                ms.seen
                    .entry(domain.clone())
                    .or_default()
                    .insert(url.clone());
                *st.fetched_urls.entry(url.clone()).or_insert(0) += 1;
            }
            world.record(
                host.clock(),
                member_name,
                Seen::Fetched {
                    member: member_name.to_owned(),
                    domain,
                    url,
                },
            );
        } else {
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }
    Ok(())
}

fn take_handoff_snapshot(
    world: &World,
    mv: &goodfetch_partitioned_crawl::handoff::DomainMove,
) -> (
    goodfetch_crawl::partition::DomainHandoffSnapshot,
    Vec<TransferredRefetchStop>,
) {
    let mut st = world.state.borrow_mut();
    let ms = st.members.get_mut(&mv.from_member).unwrap();
    let pending_urls = ms.pending.remove(&mv.domain_key).unwrap_or_default();
    let seen_fps: Vec<String> = ms
        .seen
        .remove(&mv.domain_key)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let stops = ms.refetch_stops.remove(&mv.domain_key).unwrap_or_default();
    let dstate = ms.domain_state.remove(&mv.domain_key).unwrap_or_default();
    let pending = pending_urls
        .into_iter()
        .map(|url| goodfetch_crawl::partition::DomainHandoffItem {
            url,
            label: "page".to_owned(),
            payload: None,
            method: HttpMethod::get(),
            body: None,
            key: None,
            priority: 0,
            forefront: false,
            depth: 0,
            domain_key: mv.domain_key.clone(),
        })
        .collect();
    (
        goodfetch_crawl::partition::DomainHandoffSnapshot {
            domain_key: mv.domain_key.clone(),
            pending,
            seen_fingerprints: seen_fps,
            domain_state: dstate,
        },
        stops,
    )
}

async fn perform_handoff(
    world: &World,
    clock: &goodfetch_engine::clock::Clock,
    mv: &goodfetch_partitioned_crawl::handoff::DomainMove,
    adopted: bool,
) {
    trace_fence_domain(&mv.domain_key, &mv.from_member, &mv.to_member, mv.map_epoch);
    world
        .state
        .borrow_mut()
        .members
        .get_mut(&mv.from_member)
        .unwrap()
        .admitting
        .remove(&mv.domain_key);

    while world
        .state
        .borrow()
        .members
        .get(&mv.from_member)
        .unwrap()
        .in_flight
        .contains(&mv.domain_key)
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let (snapshot, refetch_stops) = take_handoff_snapshot(world, mv);
    if !adopted {
        trace_drain_inflight(&mv.domain_key, &mv.from_member, mv.map_epoch);
        trace_send_handoff(&mv.domain_key, &mv.from_member, &mv.to_member, mv.map_epoch);
    }

    let req = HandoffDomainRequest {
        crawl_id: world.crawl_id.clone(),
        from_partition: mv.from_partition,
        from_member: mv.from_member.clone(),
        to_partition: mv.to_partition,
        to_run_id: mv.to_run_id.clone(),
        to_member: mv.to_member.clone(),
        map_epoch: mv.map_epoch,
        sender_run_epoch: 1,
        snapshot,
        refetch_stops,
        adopted,
    };
    let reply = send_with_retry(&mv.to_member, &WireFrame::Handoff(req)).await;
    assert!(matches!(
        reply,
        WireReply::Handoff(HandoffDomainResponse { acked: true, .. })
    ));

    let act = ActivateDomainRequest {
        crawl_id: world.crawl_id.clone(),
        target_run_id: mv.to_run_id.clone(),
        member: mv.to_member.clone(),
        domain_key: mv.domain_key.clone(),
        map_epoch: mv.map_epoch,
    };
    let _ = send_with_retry(&mv.to_member, &WireFrame::Activate(act)).await;
    {
        let mut st = world.state.borrow_mut();
        if adopted {
            st.adopted_handoffs += 1;
        } else {
            st.live_handoffs += 1;
        }
    }
    world.record(
        clock,
        LEAD,
        Seen::DomainHandedOff {
            domain: mv.domain_key.clone(),
            from: mv.from_member.clone(),
            to: mv.to_member.clone(),
            epoch: mv.map_epoch,
            adopted,
        },
    );
}

async fn coordinator_client(host: Host, world: World) -> turmoil::Result {
    let clock = *host.clock();
    let domain_list: Vec<String> = DOMAINS.iter().map(|d| (*d).to_owned()).collect();

    // Let epoch-1 fetches start so in-flight draining is exercised.
    tokio::time::sleep(Duration::from_millis(35)).await;

    // 1. Add `worker-b` -> publish Epoch 2.
    let map1 = world.current_map();
    let (map2, _) = map_with_added_member(
        &map1,
        &world.crawl_id,
        WORKER_B.to_owned(),
        format!("{WORKER_B}:{PORT}"),
    )
    .unwrap();
    trace_publish_map(map2.map_epoch);
    world.state.borrow_mut().map = Some(map2.clone());
    world.record(
        &clock,
        LEAD,
        Seen::MapPublished {
            epoch: map2.map_epoch,
        },
    );

    let add_moves = compute_moves(&map1, &map2, &domain_list);
    for mv in &add_moves {
        perform_handoff(&world, &clock, mv, false).await;
    }

    // 2. Send a stale forward routed under Epoch 1 to an old owner so it
    //    re-routes to `worker-b` under Epoch 2!
    if let Some(moved) = add_moves.first() {
        let stale_req = ForwardLinksRequest {
            partitioned_crawl_id: world.crawl_id.clone(),
            target_partition: moved.from_partition,
            target_run_id: moved.from_run_id.clone(),
            sender_partition: 0,
            sender_run_epoch: 1,
            partition_map_epoch: 1,
            credits_requested: 4,
            links: vec![ForwardedLink {
                url: format!("https://{}/rerouted-1", moved.domain_key),
                label: "page".to_owned(),
                payload: None,
                method: HttpMethod::get(),
                body: None,
                key: None,
                priority: 0,
                forefront: false,
                domain_key: moved.domain_key.clone(),
            }],
        };
        let _ = send_with_retry(&moved.from_member, &WireFrame::Forward(stale_req)).await;
    }

    tokio::time::sleep(Duration::from_millis(40)).await;

    // 3. Remove `worker-a` -> publish Epoch 3. Hand off first domain live and
    //    complete remaining domains via `runs adopt` fallback!
    let map3 = map_without_member(&map2, WORKER_A).unwrap();
    trace_publish_map(map3.map_epoch);
    world.state.borrow_mut().map = Some(map3.clone());
    world.record(
        &clock,
        LEAD,
        Seen::MapPublished {
            epoch: map3.map_epoch,
        },
    );

    let remove_moves = compute_moves(&map2, &map3, &domain_list);
    for (idx, mv) in remove_moves.iter().enumerate() {
        let adopted = idx > 0;
        perform_handoff(&world, &clock, mv, adopted).await;
    }

    // Wait for all remaining pending URLs to finish fetching.
    tokio::time::sleep(Duration::from_millis(500)).await;
    world.state.borrow_mut().done = true;
    Ok(())
}

fn run_handoff_simulation(
    seed: u64,
    start: SystemTime,
    injections: Vec<Injection>,
) -> Result<HandoffRun, String> {
    let captured = CapturedLog::default();
    let writer = captured.clone();
    let subscriber =
        tracing_subscriber::registry().with(model_trace(&[TARGET], move || writer.clone()));
    let world = World::new(start);
    let outcome = tracing::subscriber::with_default(subscriber, || {
        // Re-emit initial assignment inside the subscriber so the trace starts
        // with `assign_initial` for every domain.
        let map1 = world.current_map();
        for domain in DOMAINS {
            trace_assign_initial(domain, &map1.owner_of(domain).member);
        }
        let mut sim = Simulation::new(seed, start);
        for name in [LEAD, WORKER_A, WORKER_B] {
            let w = world.clone();
            sim.host(name, move |host| member_host(name, host, w.clone()));
        }
        let coord = world.clone();
        sim.client("coordinator", move |host| {
            coordinator_client(host, coord.clone())
        });
        sim.run_injecting(injections)
    });
    outcome.map_err(|e| format!("{e}"))?;

    let (
        concurrent_fetch_violations,
        duplicate_fetches,
        rerouted_forwards,
        live_handoffs,
        adopted_handoffs,
        walled_stop_holder,
        walled_final_owner,
        walled_state_preserved,
    ) = {
        let st = world.state.borrow();
        let duplicate_fetches = st.fetched_urls.values().filter(|&&count| count > 1).count() as u64;
        let final_map = st.map.clone().unwrap();
        let walled_final_owner = final_map.owner_of("walled.test").member.clone();
        let walled_stop_holder = st
            .members
            .iter()
            .find(|(_, ms)| {
                ms.refetch_stops
                    .get("walled.test")
                    .is_some_and(|v| !v.is_empty())
            })
            .map(|(m, _)| m.clone());
        let walled_state_preserved = st
            .members
            .get(&walled_final_owner)
            .and_then(|ms| ms.domain_state.get("walled.test"))
            .and_then(|m| m.get("wall_allowance"))
            .is_some_and(|v| v["refetch_stopped"] == true);
        (
            st.concurrent_fetch_violations,
            duplicate_fetches,
            st.rerouted_forwards,
            st.live_handoffs,
            st.adopted_handoffs,
            walled_stop_holder,
            walled_final_owner,
            walled_state_preserved,
        )
    };
    let Ok(log) = Rc::try_unwrap(world.log) else {
        panic!("the hosts are done with the log");
    };

    Ok(HandoffRun {
        log: log.into_inner(),
        trace: captured.written(),
        concurrent_fetch_violations,
        duplicate_fetches,
        rerouted_forwards,
        live_handoffs,
        adopted_handoffs,
        walled_stop_holder,
        walled_final_owner,
        walled_state_preserved,
    })
}

fn drawn_faults(seed: u64) -> Vec<Injection> {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test draws its Simulation's faults from its seed (ADR 0060)"
    )]
    let mut entropy = Entropy::seeded(seed);
    let mut injections = Vec::new();
    let cut = Duration::from_millis(60 + entropy.next_u64() % 120);
    injections.push(Injection {
        at: cut,
        host: WORKER_A.to_owned(),
        fault: Fault::Partition {
            peer: WORKER_B.to_owned(),
        },
    });
    injections.push(Injection {
        at: cut + Duration::from_millis(80),
        host: WORKER_A.to_owned(),
        fault: Fault::Heal {
            peer: WORKER_B.to_owned(),
        },
    });
    injections
}

fn keep(folder: &Path, file: &str, contents: &str) {
    std::fs::create_dir_all(folder).expect("model trace folder");
    std::fs::write(folder.join(file), contents).expect("write model trace");
}

/// Partition handoff under member addition, stale forward re-routing, member
/// removal, `runs adopt`, and network partitions: no domain is ever fetched by
/// two Partitions at once, seen fingerprints prevent duplicate fetches, and
/// Wall allowance per-domain state follows the domain key to its final owner.
///
/// Covers: story:3-118
#[test]
fn partition_handoff_preserves_single_owner_and_state_under_membership_changes_and_partitions() {
    let env = TestEnv::new();
    let run = run_handoff_simulation(env.seed(), env.start_time(), drawn_faults(env.seed()))
        .expect("partition handoff simulation succeeds");

    assert_eq!(
        run.concurrent_fetch_violations, 0,
        "no domain may ever be fetched by two Partitions at once"
    );
    assert_eq!(
        run.duplicate_fetches, 0,
        "transferred seen fingerprints must prevent re-fetching already-seen URLs"
    );
    assert!(
        run.live_handoffs > 0,
        "member addition/removal must execute live fenced handoffs"
    );
    assert!(
        run.rerouted_forwards > 0,
        "stale forward under epoch 1 must be re-routed under epoch 2"
    );
    assert_eq!(
        run.walled_stop_holder.as_deref(),
        Some(run.walled_final_owner.as_str()),
        "Wall allowance refetch stop must move with `walled.test` to its final owner"
    );
    assert!(
        run.walled_state_preserved,
        "attached per-domain state must move with `walled.test`"
    );

    let trace = String::from_utf8(run.trace).unwrap();
    let transitions: BTreeSet<String> = trace
        .lines()
        .map(|line| {
            let val: Value = serde_json::from_str(line).unwrap();
            val["transition"].as_str().unwrap().to_owned()
        })
        .collect();
    for expected in [
        "assign_initial",
        "publish_map",
        "fence_domain",
        "drain_inflight",
        "send_handoff",
        "ack_handoff",
        "admit_domain",
        "reroute_forward",
    ] {
        assert!(
            transitions.contains(expected),
            "expected `{expected}` transition in trace; got {transitions:?}"
        );
    }
    if run.adopted_handoffs > 0 {
        assert!(transitions.contains("adopt_handoff"));
    }

    if let Some(folder) = model_traces() {
        keep(
            &folder.join(MODEL),
            &format!("{SCENARIO}-seed-{}.ndjson", env.seed()),
            &trace,
        );
    }
}

/// Same seed reproduces identical Observation log and `goodfetch::partition`
/// model trace deterministically (ADR 0060).
///
/// Covers: story:3-118
#[test]
fn partition_handoff_simulation_is_deterministic() {
    let env = TestEnv::new();
    #[expect(
        clippy::disallowed_methods,
        reason = "a test draws its Simulations' seeds from its own (ADR 0060)"
    )]
    let mut entropy = Entropy::seeded(env.seed());
    let seeds: Vec<u64> = (0..3).map(|_| entropy.next_u64()).collect();
    let checked = check_determinism(seeds, |seed| {
        let run = run_handoff_simulation(seed, env.start_time(), drawn_faults(seed))
            .expect("deterministic handoff simulation");
        Replay::new(&run.log, &run.trace)
    });
    assert_eq!(checked, Ok(()));
}
