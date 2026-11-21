//! Two engines that make the default keychain key at once (spec #564,
//! #617): each finds the item missing and would store its own key, so the
//! second store would replace the key the first engine already wraps its
//! Runs' data keys under. keyring-core's mock store stands in for the one
//! keychain both engines share; a reader that finds the item missing waits
//! a moment for the other engine's reader, so the two reads meet where the
//! engines do not order them.
#![expect(
    clippy::disallowed_types,
    reason = "the two engines run on threads of their own, and the stand-in keychain counts their reads under a lock no loom model needs (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "each engine runs on a thread of its own, as two processes would"
)]

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use goodfetch_test_support::TestEnv;
use keyring_core::api::{Credential, CredentialApi, CredentialStoreApi};
use keyring_core::{CredentialStore, Entry, Error};
use secrecy::ExposeSecret as _;

use super::{driver_with, for_the_key, reference};
use crate::mask::Redaction;
use crate::secrets::Keychain;

/// How long a reader that found the item missing waits for the other
/// engine's reader to find it missing too.
const MEETING: Duration = Duration::from_millis(500);

/// What the two engines' keychain calls share: how many readers found the
/// item missing, and how many stores there were.
#[derive(Debug, Default)]
struct Meeting {
    counts: Mutex<(u32, u32)>,
    met: Condvar,
}

impl Meeting {
    /// A reader found the item missing: waits, up to [`MEETING`], until
    /// the other one has too.
    fn missed(&self) {
        let mut counts = self.counts.lock().unwrap();
        counts.0 += 1;
        self.met.notify_all();
        let _ = self
            .met
            .wait_timeout_while(counts, MEETING, |counts| counts.0 < 2)
            .unwrap();
    }

    fn stored(&self) {
        self.counts.lock().unwrap().1 += 1;
    }

    fn stores(&self) -> u32 {
        self.counts.lock().unwrap().1
    }
}

/// The mock store, its reads and stores met in [`Meeting`].
#[derive(Debug)]
struct Shared {
    inner: Arc<CredentialStore>,
    meeting: Arc<Meeting>,
}

impl CredentialStoreApi for Shared {
    fn vendor(&self) -> String {
        "goodfetch test meeting store".to_owned()
    }

    fn id(&self) -> String {
        "meeting".to_owned()
    }

    fn build(
        &self,
        service: &str,
        user: &str,
        modifiers: Option<&HashMap<&str, &str>>,
    ) -> keyring_core::Result<Entry> {
        let inner = self.inner.build(service, user, modifiers)?;
        Ok(Entry::new_with_credential(Arc::new(Met {
            inner,
            meeting: Arc::clone(&self.meeting),
        })))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// One item of [`Shared`].
#[derive(Debug)]
struct Met {
    inner: Entry,
    meeting: Arc<Meeting>,
}

impl CredentialApi for Met {
    fn set_secret(&self, secret: &[u8]) -> keyring_core::Result<()> {
        self.inner.set_secret(secret)?;
        self.meeting.stored();
        Ok(())
    }

    fn get_secret(&self) -> keyring_core::Result<Vec<u8>> {
        let read = self.inner.get_secret();
        if matches!(read, Err(Error::NoEntry)) {
            self.meeting.missed();
        }
        read
    }

    fn delete_credential(&self) -> keyring_core::Result<()> {
        self.inner.delete_credential()
    }

    fn get_credential(&self) -> keyring_core::Result<Option<Arc<Credential>>> {
        Ok(None)
    }

    fn get_specifiers(&self) -> Option<(String, String)> {
        self.inner.get_specifiers()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Two engines on one keychain and one Goodfetch home that make the
/// default key at once store it once, and each reads back the one key the
/// keychain holds, which it then wraps its Runs' data keys under.
/// Covers: story:4-64, setting:run_state.key_reference
#[test]
fn two_engines_that_make_the_default_key_at_once_both_use_the_one_key_stored() {
    let env = TestEnv::new();
    let meeting = Arc::new(Meeting::default());
    let store: Arc<CredentialStore> = Arc::new(Shared {
        inner: keyring_core::mock::Store::new().unwrap(),
        meeting: Arc::clone(&meeting),
    });
    let key = reference("keychain:goodfetch/run-state-key");
    let engine = || {
        let keychain = Keychain::of(Arc::clone(&store));
        let secrets =
            goodfetch_test_support::runtime().block_on(async { driver_with(&env, &[], keychain) });
        let key = &key;
        move || {
            let runtime = goodfetch_test_support::runtime();
            runtime.block_on(secrets.make_key(key)).unwrap();
            let resolving = secrets.for_run_with(Duration::from_secs(30), Redaction::On);
            runtime
                .block_on(resolving.resolve(key, &for_the_key(None, &[])))
                .unwrap()
                .expose_secret()
                .to_owned()
        }
    };

    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(engine());
        let second = scope.spawn(engine());
        (first.join().unwrap(), second.join().unwrap())
    });

    let stored = store
        .build("goodfetch", "run-state-key", None)
        .unwrap()
        .get_password()
        .unwrap();
    assert_eq!(
        (meeting.stores(), first == stored, second == stored),
        (1, true, true)
    );
}
