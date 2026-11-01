//! Certificates whose key changed in "Needs you" (ADR 0042; spec #591's
//! story 28), an engine source its contribution passes in: each pinned
//! host that presented another key and was stopped with
//! `certificate_changed`, as the known-certificates store holds the change
//! waiting for its person, answered with `certs.accept` or `certs.forget`
//! by that person alone, since both act in the caller's own partition.
//! Until the Daemon has accounts, the person is the one every call runs
//! as, which the contribution names. Its items are not dismissed: the host stays stopped
//! until its person answers. It reads again at each storage sample, which
//! also catches another engine's stops.

use goodfetch_protocol::needs_you::{CHANGED_CERTIFICATE, NeedsYouItem};
use goodfetch_protocol::tls::{CERTS_ACCEPT, CERTS_FORGET};
use serde_json::{Map, json};

use super::{Answering, Changes, NeedsYouSource, Primitives, SourceContext, SourceError};
use crate::known_certificates::{Found, Waiting};

/// Each pinned host whose changed key waits for `person`, whose partition
/// of the store holds the pin.
pub fn changed(person: &'static str) -> NeedsYouSource {
    NeedsYouSource::new(Changed { person })
}

struct Changed {
    person: &'static str,
}

impl Primitives for Changed {
    fn dismissable(&self) -> bool {
        false
    }

    fn items<'a>(
        &'a self,
        context: &'a SourceContext,
    ) -> Answering<'a, Result<Vec<NeedsYouItem>, SourceError>> {
        let store = context.known_certificates.clone();
        let person = self.person;
        Box::pin(async move {
            let waiting = context
                .blocking(move || store.waiting(person))
                .await
                .map_err(SourceError::new)?;
            Ok(waiting
                .iter()
                .map(|waiting| item(waiting, person))
                .collect())
        })
    }

    fn changes(&self, context: &SourceContext) -> Changes {
        Changes::of(context.storage())
    }
}

/// The item of the change `waiting`, which waits for `person`.
fn item(waiting: &Waiting, person: &str) -> NeedsYouItem {
    let host = &waiting.host;
    let presented = waiting.presented.to_string();
    let mut details = Map::new();
    details.insert("host".to_owned(), json!(host));
    details.insert("person".to_owned(), json!(person));
    let pinned = match waiting.pinned {
        Found::Pinned(pinned) => {
            let pinned = pinned.to_string();
            details.insert("pinned_spki_sha256".to_owned(), json!(pinned));
            format!("its pin is `{pinned}`")
        }
        Found::Unpinned | Found::Unreadable => "its pin cannot be read".to_owned(),
    };
    details.insert("presented_spki_sha256".to_owned(), json!(presented));
    NeedsYouItem {
        id: format!("{CHANGED_CERTIFICATE}:{host}"),
        kind: CHANGED_CERTIFICATE.to_owned(),
        since: waiting.seen_at.clone(),
        run: waiting.run_id.clone(),
        schedule: None,
        summary: format!(
            "{host} presented a certificate whose public key is `{presented}`, but {pinned}, so nothing is sent to it. Accept the new key only if you know why it changed, or forget the pin so its next connection pins again."
        ),
        actions: vec![CERTS_ACCEPT.name.to_owned(), CERTS_FORGET.name.to_owned()],
        details,
    }
}
