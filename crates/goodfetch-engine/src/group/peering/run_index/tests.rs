//! Group Run index feed and `reindex_to_lead` over loopback on real time
//! (ADR 0055): a Group member's local Run index writes feed the Lead Daemon's
//! group Run index with `owner` naming that member, and `reindex_to_lead`
//! re-reports rebuilt rows to the Lead Daemon.

use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::group::LeadEpoch;
use goodfetch_protocol::run::{Lifecycle, Presence, RunId, RunIndexRow, RunKind, RunOwner, Tags};
use goodfetch_test_support::{TestEnv, real_time_runtime};

use super::super::loopback::{clock_at, identity, name, now};
use super::super::{PeerServices, Peering, serve};
use super::{feed_run_index, reindex_to_lead};
use crate::Identity;
use crate::group::identity::KeptIdentity;
use crate::group::store::{self, Certified, Joining, STORE};
use crate::group::{CERTIFICATE_LIFETIME, GroupAuthority, GroupMemberKey};
use crate::home::Home;
use crate::run_index::RunIndex;
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::transport::listen_peer;
use crate::user_store::UserStore;

fn sample_row(id: &str) -> RunIndexRow {
    RunIndexRow {
        run_id: RunId::parse(id).unwrap(),
        location: format!("runs/{id}"),
        kind: RunKind::Collie,
        needs: Product::Collie,
        tags: Tags::default(),
        lifecycle: Lifecycle::Running,
        verdict: None,
        completeness: None,
        started_at: now(),
        ended_at: None,
        owner: None,
        presence: Presence::Present,
        progress_message: None,
        stop_reason: None,
        replayed: false,
    }
}

struct LeadFixture {
    _env: TestEnv,
    owner: Arc<TaskOwner>,
    store: UserStore,
    index: RunIndex,
    authority: GroupAuthority,
    kept: KeptIdentity,
    address: String,
}

async fn start_lead(id: &Identity, clock: crate::clock::Clock) -> LeadFixture {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let store = UserStore::open(
        home.per_major(&id.product_version).join(STORE.file()),
        &STORE,
        None,
        Arc::clone(&owner),
    );
    let index = RunIndex::of(
        home.run_index(&id.product_version),
        &id.product_version,
        None,
        Arc::clone(&owner),
    );
    let listener = listen_peer("127.0.0.1:0").await.unwrap();
    let address = listener.local_address().unwrap().to_string();
    let mut authority = GroupAuthority::create(now()).unwrap();
    let kept = KeptIdentity::lead(&mut authority, &name("lead"), address.clone(), now()).unwrap();
    let epoch = store::form(&store, authority.kept(), kept.clone(), now())
        .await
        .unwrap()
        .unwrap();
    let peering = Arc::new(
        Peering::new(
            id.clone(),
            store.clone(),
            &kept,
            epoch,
            vec![kept.name.clone()],
            clock,
            PeerServices {
                run_index: Some(index.clone()),
                ..PeerServices::default()
            },
        )
        .unwrap(),
    );
    owner.spawn(serve(listener, peering, owner.part("peer link")));
    LeadFixture {
        _env: env,
        owner,
        store,
        index,
        authority,
        kept,
        address,
    }
}

struct WorkerFixture {
    _env: TestEnv,
    home: Home,
    owner: Arc<TaskOwner>,
    index: RunIndex,
}

async fn admit_worker(
    lead: &LeadFixture,
    id: &Identity,
    clock: crate::clock::Clock,
) -> WorkerFixture {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let store = UserStore::open(
        home.per_major(&id.product_version).join(STORE.file()),
        &STORE,
        None,
        Arc::clone(&owner),
    );
    let index = RunIndex::of(
        home.run_index(&id.product_version),
        &id.product_version,
        None,
        Arc::clone(&owner),
    );
    let key = GroupMemberKey::generate().unwrap();
    let signed = lead
        .authority
        .sign(&name("worker"), &key.public_key(), now())
        .unwrap();
    let token = crate::group::JoinToken::mint(now()).unwrap();
    assert!(
        store::record_token(&lead.store, &token, now())
            .await
            .unwrap()
    );
    let joining = Joining {
        name: "worker".to_owned(),
        peer_address: "127.0.0.1:0".to_owned(),
        certificate: Certified::from(&signed),
    };
    let _ = store::admit(
        &lead.store,
        secrecy::ExposeSecret::expose_secret(&token.token),
        joining,
        now(),
    )
    .await
    .unwrap();
    let kept = KeptIdentity {
        name: "worker".to_owned(),
        key: key.kept(),
        certificate: signed.certificate,
        authority: lead.authority.certificate().clone(),
        revocations: lead.kept.revocations.clone(),
        peer_address: "127.0.0.1:0".to_owned(),
        lead_address: lead.address.clone(),
        lead_name: "lead".to_owned(),
        certificate_expires_at: now() + CERTIFICATE_LIFETIME,
        renew_at: now() + CERTIFICATE_LIFETIME,
    };
    assert!(
        store::joined(&store, kept.clone(), LeadEpoch::new(1))
            .await
            .unwrap()
    );
    let peering = Arc::new(
        Peering::new(
            id.clone(),
            store,
            &kept,
            LeadEpoch::new(1),
            Vec::new(),
            clock,
            PeerServices {
                run_index: Some(index.clone()),
                ..PeerServices::default()
            },
        )
        .unwrap(),
    );
    owner.spawn(feed_run_index(peering, owner.part("run-index-feed")));
    WorkerFixture {
        _env: env,
        home,
        owner,
        index,
    }
}

#[test]
fn a_group_member_feeds_the_lead_daemons_run_index_and_reindex_re_reports() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let id = identity();
        let lead = start_lead(&id, clock).await;
        let worker = admit_worker(&lead, &id, clock).await;

        // Writing a local Run on the member feeds the Lead Daemon's Run index
        // with `owner == Some("worker")`.
        let row = sample_row("run-live-1");
        worker.index.write(row.clone()).await.unwrap();
        let mut seen = None;
        for _ in 0..40 {
            seen = lead.index.read(row.run_id.clone()).await.unwrap();
            if seen.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let seen = seen.expect("Lead Daemon should receive the member's Run index row");
        assert_eq!(seen.owner, Some(RunOwner::GroupMember("worker".to_owned())));
        assert_eq!(seen.run_id, row.run_id);

        // Stop the feeder, write a second row directly on the member, and verify
        // `reindex_to_lead` re-reports both rows to the Lead Daemon.
        worker.owner.stop().await;
        let second = sample_row("run-reindexed-2");
        let index_file = crate::run_index::IndexFile::of(
            worker.home.run_index(&id.product_version),
            &id.product_version,
        );
        let conn = crate::sqlite_store::StoreFile::open(&index_file).unwrap();
        index_file.write(&conn, &second).unwrap();
        drop(conn);

        let reported = reindex_to_lead(&worker.home, &id, clock).await.unwrap();
        assert_eq!(reported, 2);
        let reindexed = lead
            .index
            .read(second.run_id.clone())
            .await
            .unwrap()
            .expect("Lead Daemon should record re-reported row");
        assert_eq!(
            reindexed.owner,
            Some(RunOwner::GroupMember("worker".to_owned()))
        );

        lead.owner.stop().await;
    });
}
