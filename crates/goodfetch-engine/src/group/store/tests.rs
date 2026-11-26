//! The group store through its interface, on real SQLite in the test's
//! folder: a group is formed or joined once, a join token admits one
//! Daemon and expires, a Group member name is taken once, and no join token
//! reaches the disk, only its digest.
#![expect(
    clippy::disallowed_methods,
    reason = "a test reads the store's files as bytes; on no Shared path (ADR 0140)"
)]

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::group::LeadEpoch;
use goodfetch_test_support::{TestEnv, runtime};
use rstest::rstest;
use secrecy::ExposeSecret;

use super::{
    Admission, Joining, STORE, admit, authority, form, identity, is_in_a_group, joined,
    record_token,
};
use crate::group::identity::KeptIdentity;
use crate::group::{
    GroupAuthority, GroupMemberName, JOIN_TOKEN_LIFETIME, JoinToken, JoinTokenRefused,
};
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::user_store::UserStore;

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_hours(500_000)
}

fn store(env: &TestEnv) -> UserStore {
    UserStore::open(
        env.home().join("0.1").join(STORE.file()),
        &STORE,
        None,
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
    )
}

/// The identity of the Lead Daemon `lead` of `authority`.
fn lead(authority: &mut GroupAuthority) -> KeptIdentity {
    KeptIdentity::lead(
        authority,
        &GroupMemberName::new("lead").unwrap(),
        "127.0.0.1:7000".to_owned(),
        now(),
    )
    .unwrap()
}

/// A store whose group is formed, and its authority.
async fn formed(store: &UserStore) -> GroupAuthority {
    let mut created = GroupAuthority::create(now()).unwrap();
    let lead = lead(&mut created);
    form(store, created.kept(), lead, now())
        .await
        .unwrap()
        .unwrap();
    created
}

/// A token minted at `now()` and recorded in `store`.
async fn recorded(store: &UserStore) -> JoinToken {
    let minted = JoinToken::mint(now()).unwrap();
    assert!(record_token(store, &minted, now()).await.unwrap());
    minted
}

/// What the store answers a Daemon that joins at `at` with the join token
/// `given`, under a name of the token's, so each token's Daemon has its own.
async fn redeem(store: &UserStore, given: &str, at: SystemTime) -> Admission {
    let name = format!("worker-{}", &given[..given.len().min(8)]);
    joining_as(store, given, &name, at).await
}

/// What the store answers the Daemon `name` that joins at `at` with the
/// join token `given`.
async fn joining_as(store: &UserStore, given: &str, name: &str, at: SystemTime) -> Admission {
    let joining = Joining {
        name: name.to_owned(),
        peer_address: "127.0.0.1:7001".to_owned(),
    };
    admit(store, given, joining, at).await.unwrap()
}

#[test]
fn a_formed_group_keeps_its_authority_at_lead_epoch_one() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        let mut created = GroupAuthority::create(now()).unwrap();
        let lead = lead(&mut created);

        let epoch = form(&store, created.kept(), lead, now()).await.unwrap();
        let kept = authority(&store).await.unwrap().unwrap();

        assert_eq!(epoch, Some(LeadEpoch::new(1)));
        let restored = GroupAuthority::restore(kept).unwrap();
        assert_eq!(restored.certificate(), created.certificate());
        assert_eq!(restored.key_hash(), created.key_hash());
    });
}

#[test]
fn a_group_is_formed_once_and_keeps_its_first_authority() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        let first = formed(&store).await;
        let mut second = GroupAuthority::create(now()).unwrap();
        let lead = lead(&mut second);

        let again = form(&store, second.kept(), lead, now()).await.unwrap();

        assert_eq!(again, None);
        let kept = authority(&store).await.unwrap().unwrap();
        assert_eq!(&kept.certificate, first.certificate());
    });
}

/// The Lead Daemon keeps its own identity, which a restart serves the
/// peer link with, at the lead epoch it formed the group at.
#[test]
fn a_formed_group_keeps_the_lead_daemons_identity() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        assert!(!is_in_a_group(&store).await.unwrap());
        let created = formed(&store).await;

        let (kept, epoch) = identity(&store).await.unwrap().unwrap();

        assert!(is_in_a_group(&store).await.unwrap());
        assert_eq!(
            (kept.name.as_str(), kept.lead_address.as_str(), epoch),
            ("lead", "127.0.0.1:7000", LeadEpoch::new(1))
        );
        assert_eq!(&kept.authority, created.certificate());
    });
}

/// A Daemon that joined keeps its identity at the group's lead epoch, and
/// can neither join again nor form a group.
#[test]
fn a_daemon_joins_one_group_once() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        let mut theirs = GroupAuthority::create(now()).unwrap();
        let member = lead(&mut theirs);
        let again = lead(&mut theirs);

        assert!(joined(&store, member, LeadEpoch::new(3)).await.unwrap());
        assert!(!joined(&store, again, LeadEpoch::new(3)).await.unwrap());
        let mut own = GroupAuthority::create(now()).unwrap();
        let lead = lead(&mut own);
        let formed = form(&store, own.kept(), lead, now()).await.unwrap();

        assert_eq!(formed, None);
        let (_, epoch) = identity(&store).await.unwrap().unwrap();
        assert_eq!(epoch, LeadEpoch::new(3));
        assert!(authority(&store).await.unwrap().is_none());
    });
}

#[test]
fn a_store_with_no_group_keeps_no_authority_and_records_no_token() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        let minted = JoinToken::mint(now()).unwrap();

        assert!(authority(&store).await.unwrap().is_none());
        assert!(identity(&store).await.unwrap().is_none());
        assert!(!record_token(&store, &minted, now()).await.unwrap());
        assert_eq!(
            redeem(&store, minted.token.expose_secret(), now()).await,
            Admission::NotLead
        );
    });
}

#[test]
fn a_join_token_works_once() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        formed(&store).await;
        let minted = recorded(&store).await;
        let token = minted.token.expose_secret();

        assert_eq!(redeem(&store, token, now()).await, Admission::Admitted);
        assert_eq!(
            redeem(&store, token, now()).await,
            Admission::Refused(JoinTokenRefused::Unknown)
        );
    });
}

#[test]
fn a_join_token_past_its_lifetime_is_refused_as_expired() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        formed(&store).await;
        let minted = recorded(&store).await;

        assert_eq!(minted.expires, now() + Duration::from_mins(30));
        assert_eq!(
            redeem(&store, minted.token.expose_secret(), minted.expires).await,
            Admission::Refused(JoinTokenRefused::Expired)
        );
    });
}

#[test]
fn a_token_never_minted_is_refused_as_unknown() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        formed(&store).await;
        let minted = recorded(&store).await;
        let mut other = minted.token.expose_secret().to_owned();
        other.replace_range(..1, if other.starts_with('0') { "1" } else { "0" });

        assert_eq!(
            redeem(&store, &other, now()).await,
            Admission::Refused(JoinTokenRefused::Unknown)
        );
        assert_eq!(
            redeem(&store, minted.token.expose_secret(), now()).await,
            Admission::Admitted
        );
    });
}

/// A Daemon that asks under a Group member's name is refused, and its join
/// token stays valid for another name; an unknown token is refused before
/// the name is looked at, so it learns no name.
#[test]
fn a_taken_name_is_refused_and_keeps_the_token() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        formed(&store).await;
        let minted = recorded(&store).await;
        let token = minted.token.expose_secret();

        assert_eq!(
            joining_as(&store, "unknown", "lead", now()).await,
            Admission::Refused(JoinTokenRefused::Unknown)
        );
        assert_eq!(
            joining_as(&store, token, "lead", now()).await,
            Admission::NameTaken
        );
        assert_eq!(
            joining_as(&store, token, "worker", now()).await,
            Admission::Admitted
        );
    });
}

#[test]
fn each_minted_token_is_its_own() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        formed(&store).await;
        let first = recorded(&store).await;
        let second = recorded(&store).await;

        assert_ne!(first.token.expose_secret(), second.token.expose_secret());
        assert_eq!(
            redeem(&store, second.token.expose_secret(), now()).await,
            Admission::Admitted
        );
        assert_eq!(
            redeem(&store, first.token.expose_secret(), now()).await,
            Admission::Admitted
        );
    });
}

/// A canary: every file of the store, its WAL included, holds a join
/// token's digest and never the token.
#[rstest]
#[case::kept_open(false)]
#[case::closed(true)]
fn no_join_token_reaches_the_disk(#[case] closed: bool) {
    let env = TestEnv::new();
    let store = store(&env);
    let minted = runtime().block_on(async {
        formed(&store).await;
        recorded(&store).await
    });
    if closed {
        drop(store);
    }

    let mut bytes = Vec::new();
    for entry in std::fs::read_dir(env.home().join("0.1")).unwrap() {
        bytes.extend(std::fs::read(entry.unwrap().path()).unwrap());
    }

    let holds = |needle: &str| {
        bytes
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    };
    assert!(!holds(minted.token.expose_secret()), "the token is on disk");
    assert!(holds(&minted.digest()));
}

#[test]
fn a_token_expired_a_lifetime_ago_is_forgotten_at_the_next_mint() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        formed(&store).await;
        let old = recorded(&store).await;
        let later = old.expires + JOIN_TOKEN_LIFETIME;

        let minted = JoinToken::mint(later).unwrap();
        record_token(&store, &minted, later).await.unwrap();

        assert_eq!(
            redeem(&store, old.token.expose_secret(), later).await,
            Admission::Refused(JoinTokenRefused::Unknown)
        );
    });
}
