//! Unit and loopback peer-link tests for scale-down handoff, restart resume,
//! non-hijack Run index reporting, and `runs adopt` machine-bound refusal
//! (ADRs 0055 and 0056, issue 245).
#![expect(
    clippy::disallowed_methods,
    reason = "unit tests create temporary run folders and SQLite rows"
)]

use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::checkpoint::{RunBinding, SESSION_BOUND_TO_MACHINE};
use goodfetch_protocol::group::{
    GROUP_RUN_INDEX_LIST_ASSIGNED, LeadEpoch, PEER_PROTOCOL_VERSION,
    PeerRunIndexListAssignedParams, PeerRunIndexListAssignedResult, PeerRunIndexReportParams,
    PeerRunIndexReportResult, RUN_INDEX_REPORT,
};
use goodfetch_protocol::run::{Lifecycle, Presence, RunId, RunIndexRow, RunKind, RunOwner, Tags};
use goodfetch_test_support::TestEnv;

use super::super::loopback::{self, clock_at, identity, now};
use super::super::{PeerServices, ask_lead};
use super::{adopt_runs, prepare_checkpoint};
use crate::group::store;
use crate::home::Home;
use crate::run_index::RunIndex;
use crate::run_state::{RunState, RunStateOptions};
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::user_store::UserStore;

fn row_for(run_id: &str, owner: &str, lifecycle: Lifecycle) -> RunIndexRow {
    RunIndexRow {
        run_id: RunId::parse(run_id).unwrap(),
        kind: RunKind::Collie,
        needs: Product::Collie,
        lifecycle,
        verdict: None,
        completeness: None,
        started_at: now(),
        ended_at: None,
        tags: Tags::default(),
        location: format!("runs/{run_id}"),
        presence: Presence::Present,
        owner: Some(RunOwner::GroupMember(owner.to_owned())),
        progress_message: None,
        stop_reason: None,
        replayed: false,
    }
}

async fn create_run_state_folder(
    env: &TestEnv,
    run_id: &str,
    binding: Option<&str>,
) -> std::path::PathBuf {
    let home = Home::new(env.home().to_owned());
    let folder = home.runs(&identity().product_version).join(run_id);
    std::fs::create_dir_all(&folder).unwrap();
    let run_file = folder.join(goodfetch_protocol::home::RUN_FILE);
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let state = RunState::open(run_file, &RunStateOptions::default(), Vec::new(), 1, &owner)
        .await
        .unwrap();
    let id = run_id.to_owned();
    let binding_json = binding.map(str::to_owned);
    state
        .write(move |conn| {
            conn.execute(
                "INSERT INTO run (run_id, kind, needs, caller, epoch, inputs, lifecycle, started_at, binding)
                 VALUES (?1, 'collie', 'collie', 'os_user', 1, '{}', 'stopped', '2026-01-05T09:00:00Z', ?2)",
                (id, binding_json),
            )
            .map(drop)
        })
        .await
        .unwrap();
    drop(state);
    owner.wait().await;
    folder
}

/// Passive Run index reports from a restarted/re-added Group member never
/// overwrite a Run already assigned to another Group member, while `claim: true`
/// transfers ownership, and `group.run_index.list_assigned` returns only
/// non-ended Runs still assigned to the requesting member.
///
/// Covers: story:3-111
#[test]
fn run_index_list_assigned_and_non_hijack_prevent_double_execution() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = clock_at(now());
        let env = TestEnv::new();
        let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
        let run_index = RunIndex::of(
            env.home().join("run-index.sqlite"),
            &identity().product_version,
            None,
            Arc::clone(&owner),
        );
        let services = PeerServices {
            run_index: Some(run_index.clone()),
            owner: Some(Arc::clone(&owner)),
            ..PeerServices::default()
        };
        let lead =
            loopback::lead_with_services(clock, now() + Duration::from_hours(12), services).await;

        let (kept_a, cert_a) = lead.member("worker-a", now() + Duration::from_hours(12));
        lead.admit("worker-a", cert_a).await;
        let (kept_b, cert_b) = lead.member("worker-b", now() + Duration::from_hours(12));
        lead.admit("worker-b", cert_b).await;

        let report_a = PeerRunIndexReportParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: identity().product,
            product_version: identity().product_version,
            lead_epoch: LeadEpoch::new(1),
            member: "worker-a".to_owned(),
            rows: vec![
                row_for("run-still-a", "worker-a", Lifecycle::Stopped),
                row_for("run-adopted-b", "worker-a", Lifecycle::Stopped),
                row_for("run-done-a", "worker-a", Lifecycle::Finished),
            ],
            claim: false,
        };
        let _: PeerRunIndexReportResult = ask_lead(&kept_a, clock, RUN_INDEX_REPORT, &report_a)
            .await
            .unwrap();

        let claim_b = PeerRunIndexReportParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: identity().product,
            product_version: identity().product_version,
            lead_epoch: LeadEpoch::new(1),
            member: "worker-b".to_owned(),
            rows: vec![row_for("run-adopted-b", "worker-b", Lifecycle::Running)],
            claim: true,
        };
        let _: PeerRunIndexReportResult = ask_lead(&kept_b, clock, RUN_INDEX_REPORT, &claim_b)
            .await
            .unwrap();

        // Passive re-report from worker-a must NOT hijack `run-adopted-b` back.
        let _: PeerRunIndexReportResult = ask_lead(&kept_a, clock, RUN_INDEX_REPORT, &report_a)
            .await
            .unwrap();

        let owner_after = run_index
            .read(RunId::parse("run-adopted-b").unwrap())
            .await
            .unwrap()
            .unwrap()
            .owner;
        assert_eq!(
            owner_after,
            Some(RunOwner::GroupMember("worker-b".to_owned()))
        );

        let list_params = PeerRunIndexListAssignedParams {
            protocol_version: PEER_PROTOCOL_VERSION,
            product: identity().product,
            product_version: identity().product_version,
            lead_epoch: LeadEpoch::new(1),
            member: "worker-a".to_owned(),
            for_member: "worker-a".to_owned(),
        };
        let listed: PeerRunIndexListAssignedResult =
            ask_lead(&kept_a, clock, GROUP_RUN_INDEX_LIST_ASSIGNED, &list_params)
                .await
                .unwrap();
        let ids: Vec<String> = listed
            .rows
            .into_iter()
            .map(|r| r.run_id.as_str().to_owned())
            .collect();
        assert_eq!(ids, vec!["run-still-a".to_owned()]);

        owner.stop().await;
    });
}

/// `runs adopt` refuses a machine-bound Run (ADR 0056) with
/// `session_bound_to_machine`, leaving it assigned to the removed Group member.
///
/// Covers: story:3-111
#[test]
fn adopt_runs_refuses_machine_bound_run_with_session_bound_to_machine() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = clock_at(now());
        let lead = loopback::lead(clock, now() + Duration::from_hours(12)).await;

        let adopter_env = TestEnv::new();
        let adopter_home = Home::new(adopter_env.home().to_owned());
        let adopter_store_path = adopter_home
            .per_major(&identity().product_version)
            .join(store::STORE.file());
        std::fs::create_dir_all(adopter_store_path.parent().unwrap()).unwrap();
        let adopter_owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
        let adopter_store = UserStore::open(
            adopter_store_path,
            &store::STORE,
            None,
            Arc::clone(&adopter_owner),
        );
        let (kept_b, cert_b) = lead.member("worker-b", now() + Duration::from_hours(12));
        lead.admit("worker-b", cert_b).await;
        assert!(
            store::joined(&adopter_store, kept_b, LeadEpoch::new(1))
                .await
                .unwrap()
        );
        drop(adopter_store);
        adopter_owner.wait().await;

        let kept_vol = TestEnv::new();
        let binding_json = serde_json::to_string(&RunBinding::Machine {
            machine: "worker-a-host".to_owned(),
        })
        .unwrap();
        let bound_folder =
            create_run_state_folder(&kept_vol, "run-bound-1", Some(&binding_json)).await;
        let prepared = prepare_checkpoint(&bound_folder, 7).unwrap().unwrap();
        assert!(matches!(
            prepared.held.binding,
            Some(RunBinding::Machine { .. })
        ));

        let err = adopt_runs(
            &Home::new(adopter_env.home().to_owned()),
            &identity(),
            clock,
            &[bound_folder],
            Some("worker-a"),
        )
        .await
        .expect_err("machine-bound run must be refused by runs adopt");
        assert_eq!(err.code(), SESSION_BOUND_TO_MACHINE.name);
    });
}
