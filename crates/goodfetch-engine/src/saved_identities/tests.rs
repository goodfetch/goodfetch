//! Unit tests for the encrypted per-major Saved identity store (ADRs 0015,
//! 0035, 0056, 0089 and 0098; ticket #736).

#![expect(
    clippy::disallowed_methods,
    reason = "unit tests inspect on-disk files and permissions directly"
)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::Compatibility;
use goodfetch_protocol::page::proxy::ProxyKind;
use goodfetch_protocol::session_export::SavedProxyBinding;
use secrecy::SecretString;
use secrecy::zeroize::Zeroizing;

use super::{NewSavedIdentity, SAVED_IDENTITY_STORE, SavedIdentityStore, StoreError};
use crate::home::Home;
use crate::lowest_minor::{self, Marker};
use crate::run_key::KeyError;
use crate::run_state::RunStateError;
use crate::tasks::{BlockingPolicy, TaskOwner};

fn product(major: u16, minor: u16) -> ProductVersion {
    ProductVersion::parse(&format!("{major}.{minor}.0")).unwrap()
}

fn kek(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn sample_identity(name: &str, cookie: &str) -> NewSavedIdentity {
    NewSavedIdentity {
        name: name.to_owned(),
        access_mode: "standard".to_owned(),
        identity_seed: 42,
        browser_family: "chromium".to_owned(),
        user_agent: Some("Mozilla/5.0 Test".to_owned()),
        locale: Some("fr-FR".to_owned()),
        timezone_id: Some("Europe/Paris".to_owned()),
        proxy_binding: None,
        machine_binding: None,
        preserve_existing_machine_binding: true,
        was_lent: false,
        source_kind: None,
        domains: BTreeSet::from(["portal.test".to_owned()]),
        cookie_count: 1,
        origin_count: 1,
        payload_plaintext: Zeroizing::new(
            format!(r#"{{"cookies":[{{"name":"sid","value":"{cookie}"}}]}}"#).into_bytes(),
        ),
    }
}

#[tokio::test]
async fn saved_identity_is_encrypted_on_disk_and_mode_0600() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(dir.path().to_path_buf());
    let version = product(1, 0);
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let store = SavedIdentityStore::of(
        home.saved_identities(&version),
        home.saved_identities_lock(&version),
        &version,
        None,
        owner,
    );
    let canary = "c4n4ry-s3ss10n-t0k3n-9911";
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let age = Some(Duration::from_hours(720));

    let info = store
        .save(
            sample_identity("portal", canary),
            "secretfile:run-state-key".to_owned(),
            kek("test-kek-material"),
            now,
            age,
        )
        .await
        .unwrap();
    assert_eq!(info.name, "portal");
    assert_eq!(info.domains, vec!["portal.test"]);

    // No file anywhere under Goodfetch home holds the canary in clear.
    for entry in walk(dir.path()) {
        let bytes = std::fs::read(&entry).unwrap();
        assert!(
            !bytes
                .windows(canary.len())
                .any(|window| window == canary.as_bytes()),
            "canary leaked in clear into {}",
            entry.display()
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(home.saved_identities(&version))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o077,
            0,
            "store file must not be readable by group/other (mode {mode:o})"
        );
    }

    // Wrong KEK cannot open it; right KEK decrypts exact payload.
    let (sealed, expired) = store
        .load_sealed("portal".to_owned(), now, age, false)
        .await
        .unwrap();
    assert_eq!(expired, Vec::<String>::new());
    let sealed = sealed.unwrap();
    assert!(matches!(
        sealed.clone().open(&kek("wrong-kek")),
        Err(KeyError::DoesNotOpen)
    ));
    let loaded = sealed.open(&kek("test-kek-material")).unwrap();
    assert!(
        std::str::from_utf8(&loaded.payload_plaintext)
            .unwrap()
            .contains(canary)
    );
}

#[tokio::test]
async fn replace_keeps_machine_binding_and_concurrent_saves_serialize() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(dir.path().to_path_buf());
    let version = product(1, 0);
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let store = SavedIdentityStore::of(
        home.saved_identities(&version),
        home.saved_identities_lock(&version),
        &version,
        None,
        owner,
    );
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let mut initial = sample_identity("acct", "token-v1");
    initial.machine_binding = Some("machine-abc".to_owned());
    let first = store
        .save(initial, "env:KEK".to_owned(), kek("same-kek"), now, None)
        .await
        .unwrap();
    assert!(first.machine_bound);

    // Concurrent saves replacing the same name serialize under the lock and
    // preserve the existing machine binding.
    let mut handles = Vec::new();
    for i in 0_u64..8 {
        let store = store.clone();
        handles.push(tokio::spawn(async move {
            let mut next = sample_identity("acct", &format!("token-v{i}"));
            next.was_lent = i % 2 == 1;
            store
                .save(
                    next,
                    "env:KEK".to_owned(),
                    kek("same-kek"),
                    now + Duration::from_secs(i + 1),
                    None,
                )
                .await
        }));
    }
    for handle in handles {
        let info = handle.await.unwrap().unwrap();
        assert!(info.machine_bound, "machine binding must be preserved");
    }

    let (list, _) = store
        .list(now + Duration::from_secs(10), None)
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].machine_bound);
    let (sealed, _) = store
        .load_sealed(
            "acct".to_owned(),
            now + Duration::from_secs(10),
            None,
            false,
        )
        .await
        .unwrap();
    let loaded = sealed.unwrap().open(&kek("same-kek")).unwrap();
    assert_eq!(loaded.machine_binding.as_deref(), Some("machine-abc"));
}

#[tokio::test]
async fn expiry_after_30_days_unused_and_major_isolation_and_lowest_minor() {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(dir.path().to_path_buf());
    let v1 = product(1, 0);
    let v2 = product(2, 0);
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let store_v1 = SavedIdentityStore::of(
        home.saved_identities(&v1),
        home.saved_identities_lock(&v1),
        &v1,
        None,
        Arc::clone(&owner),
    );
    let store_v2 = SavedIdentityStore::of(
        home.saved_identities(&v2),
        home.saved_identities_lock(&v2),
        &v2,
        None,
        Arc::clone(&owner),
    );
    let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let thirty_days = Some(Duration::from_hours(720));

    let mut id = sample_identity("stale", "cookie-1");
    id.proxy_binding = Some(SavedProxyBinding {
        server: Some("http://proxy.test:8080".to_owned()),
        provider: None,
        country: Some("FR".to_owned()),
        kind: Some(ProxyKind::Residential),
        sticky_session: None,
    });
    store_v1
        .save(id, "env:KEK".to_owned(), kek("k"), start, thirty_days)
        .await
        .unwrap();

    // Another Product major never reads v1's store.
    let (v2_list, _) = store_v2.list(start, thirty_days).await.unwrap();
    assert_eq!(v2_list.len(), 0);

    // After 29 days, still present; after 30 days, expires and is deleted.
    let day_29 = start + Duration::from_hours(696);
    let (still_there, expired) = store_v1.list(day_29, thirty_days).await.unwrap();
    assert_eq!(still_there.len(), 1);
    assert_eq!(expired, Vec::<String>::new());

    let day_30 = start + Duration::from_hours(720);
    let (after_expiry, expired) = store_v1.list(day_30, thirty_days).await.unwrap();
    assert_eq!(after_expiry.len(), 0);
    assert_eq!(expired, vec!["stale"]);

    // Raising lowest_minor past engine 1.0 refuses the older engine.
    let conn = rusqlite::Connection::open(home.saved_identities(&v1)).unwrap();
    lowest_minor::write_sqlite(
        &conn,
        SAVED_IDENTITY_STORE.name,
        Marker {
            version: SAVED_IDENTITY_STORE.version,
            lowest_minor: 2,
        },
    )
    .unwrap();
    drop(conn);

    let err = store_v1.list(day_30, thirty_days).await.unwrap_err();
    assert!(matches!(
        err,
        StoreError::State(RunStateError::Format {
            compatibility: Compatibility::NewerMinor,
            ..
        })
    ));
}

fn walk(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}
