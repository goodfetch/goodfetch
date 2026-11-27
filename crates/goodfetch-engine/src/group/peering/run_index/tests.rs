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

#[test]
fn a_group_member_feeds_the_lead_daemons_run_index_and_reindex_re_reports() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let id = identity();
        let lead_env = TestEnv::new();
        let lead_home = Home::new(lead_env.home().to_path_buf());
        let lead_owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
        let lead_store = UserStore::open(
            lead_home.per_major(&id.product_version).join(STORE.file()),
            &STORE,
            None,
            Arc::clone(&lead_owner),
        );
        let lead_index = RunIndex::of(
            lead_home.run_index(&id.product_version),
            &id.product_version,
            None,
            Arc::clone(&lead_owner),
        );
        let listener = listen_peer("127.0.0.1:0").await.unwrap();
        let lead_address = listener.local_address().unwrap().to_string();
        let mut authority = GroupAuthority::create(now()).unwrap();
        let lead_kept =
            KeptIdentity::lead(&mut authority, &name("lead"), lead_address.clone(), now()).unwrap();
        let epoch = store::form(&lead_store, authority.kept(), lead_kept.clone(), now())
            .await
            .unwrap()
            .unwrap();
        let lead_peering = Arc::new(
            Peering::new(
                id.clone(),
                lead_store.clone(),
                &lead_kept,
                epoch,
                vec![lead_kept.name.clone()],
                clock,
                PeerServices {
                    run_index: Some(lead_index.clone()),
                    ..PeerServices::default()
                },
            )
            .unwrap(),
        );
        lead_owner.spawn(serve(
            listener,
            Arc::clone(&lead_peering),
            lead_owner.part("peer link"),
        ));

        // Admit a worker member.
        let worker_env = TestEnv::new();
        let worker_home = Home::new(worker_env.home().to_path_buf());
        let worker_owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
        let worker_store = UserStore::open(
            worker_home
                .per_major(&id.product_version)
                .join(STORE.file()),
            &STORE,
            None,
            Arc::clone(&worker_owner),
        );
        let worker_index = RunIndex::of(
            worker_home.run_index(&id.product_version),
            &id.product_version,
            None,
            Arc::clone(&worker_owner),
        );
        let worker_key = GroupMemberKey::generate().unwrap();
        let signed = authority
            .sign(&name("worker"), &worker_key.public_key(), now())
            .unwrap();
        let token = crate::group::JoinToken::mint(now()).unwrap();
        assert!(
            store::record_token(&lead_store, &token, now())
                .await
                .unwrap()
        );
        let joining = Joining {
            name: "worker".to_owned(),
            peer_address: "127.0.0.1:0".to_owned(),
            certificate: Certified::from(&signed),
        };
        let _ = store::admit(
            &lead_store,
            secrecy::ExposeSecret::expose_secret(&token.token),
            joining,
            now(),
        )
        .await
        .unwrap();
        let worker_kept = KeptIdentity {
            name: "worker".to_owned(),
            key: worker_key.kept(),
            certificate: signed.certificate,
            authority: authority.certificate().clone(),
            revocations: lead_kept.revocations.clone(),
            peer_address: "127.0.0.1:0".to_owned(),
            lead_address,
            lead_name: "lead".to_owned(),
            certificate_expires_at: now() + CERTIFICATE_LIFETIME,
            renew_at: now() + CERTIFICATE_LIFETIME,
        };
        assert!(
            store::joined(&worker_store, worker_kept.clone(), LeadEpoch::new(1))
                .await
                .unwrap()
        );
        let worker_peering = Arc::new(
            Peering::new(
                id.clone(),
                worker_store.clone(),
                &worker_kept,
                LeadEpoch::new(1),
                Vec::new(),
                clock,
                PeerServices {
                    run_index: Some(worker_index.clone()),
                    ..PeerServices::default()
                },
            )
            .unwrap(),
        );
        let feeder = worker_owner.part("run-index-feed");
        worker_owner.spawn(feed_run_index(Arc::clone(&worker_peering), feeder));

        // Writing a local Run on the member feeds the Lead Daemon's Run index
        // with `owner == Some("worker")`.
        let row = sample_row("run-live-1");
        worker_index.write(row.clone()).await.unwrap();
        let mut seen = None;
        for _ in 0..40 {
            seen = lead_index.read(row.run_id.clone()).await.unwrap();
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
        worker_owner.stop().await;
        let second = sample_row("run-reindexed-2");
        let index_file = crate::run_index::IndexFile::of(
            worker_home.run_index(&id.product_version),
            &id.product_version,
        );
        let conn = crate::sqlite_store::StoreFile::open(&index_file).unwrap();
        index_file.write(&conn, &second).unwrap();
        drop(conn);

        let reported = reindex_to_lead(&worker_home, &id, clock).await.unwrap();
        assert_eq!(reported, 2);
        let reindexed = lead_index
            .read(second.run_id.clone())
            .await
            .unwrap()
            .expect("Lead Daemon should record re-reported row");
        assert_eq!(
            reindexed.owner,
            Some(RunOwner::GroupMember("worker".to_owned()))
        );

        lead_owner.stop().await;
    });
}
