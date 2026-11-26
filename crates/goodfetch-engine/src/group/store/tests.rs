use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::group::LeadEpoch;
use goodfetch_test_support::{TestEnv, runtime};
use secrecy::ExposeSecret;

use super::{STORE, authority, form, record_token, redeem};
use crate::group::{GroupAuthority, JOIN_TOKEN_LIFETIME, JoinToken, JoinTokenRefused};
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

/// A store whose group is formed, and its authority.
async fn formed(store: &UserStore) -> GroupAuthority {
    let created = GroupAuthority::create(now()).unwrap();
    form(store, created.kept()).await.unwrap().unwrap();
    created
}

/// A token minted at `now()` and recorded in `store`.
async fn recorded(store: &UserStore) -> JoinToken {
    let minted = JoinToken::mint(now()).unwrap();
    assert!(record_token(store, &minted, now()).await.unwrap());
    minted
}

#[test]
fn a_formed_group_keeps_its_authority_at_lead_epoch_one() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        let created = GroupAuthority::create(now()).unwrap();

        let epoch = form(&store, created.kept()).await.unwrap();
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

        let again = form(&store, GroupAuthority::create(now()).unwrap().kept())
            .await
            .unwrap();

        assert_eq!(again, None);
        let kept = authority(&store).await.unwrap().unwrap();
        assert_eq!(&kept.certificate, first.certificate());
    });
}

#[test]
fn a_store_with_no_group_keeps_no_authority_and_records_no_token() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        let minted = JoinToken::mint(now()).unwrap();

        assert!(authority(&store).await.unwrap().is_none());
        assert!(!record_token(&store, &minted, now()).await.unwrap());
        assert_eq!(
            redeem(&store, minted.token.expose_secret(), now())
                .await
                .unwrap(),
            Err(JoinTokenRefused::Unknown)
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

        assert_eq!(redeem(&store, token, now()).await.unwrap(), Ok(()));
        assert_eq!(
            redeem(&store, token, now()).await.unwrap(),
            Err(JoinTokenRefused::Unknown)
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
            redeem(&store, minted.token.expose_secret(), minted.expires)
                .await
                .unwrap(),
            Err(JoinTokenRefused::Expired)
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
            redeem(&store, &other, now()).await.unwrap(),
            Err(JoinTokenRefused::Unknown)
        );
        assert_eq!(
            redeem(&store, minted.token.expose_secret(), now())
                .await
                .unwrap(),
            Ok(())
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
            redeem(&store, second.token.expose_secret(), now())
                .await
                .unwrap(),
            Ok(())
        );
        assert_eq!(
            redeem(&store, first.token.expose_secret(), now())
                .await
                .unwrap(),
            Ok(())
        );
    });
}

#[test]
fn the_store_keeps_a_tokens_digest_never_the_token() {
    let env = TestEnv::new();
    let store = store(&env);
    runtime().block_on(async {
        formed(&store).await;
        let minted = recorded(&store).await;

        let kept: Vec<String> = store
            .read(Vec::new(), |connection| {
                connection
                    .prepare("SELECT digest FROM join_tokens")?
                    .query_map([], |row| row.get(0))?
                    .collect()
            })
            .await
            .unwrap();

        assert_eq!(kept, vec![minted.digest()]);
        assert_ne!(kept[0], minted.token.expose_secret());
    });
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
            redeem(&store, old.token.expose_secret(), later)
                .await
                .unwrap(),
            Err(JoinTokenRefused::Unknown)
        );
    });
}
