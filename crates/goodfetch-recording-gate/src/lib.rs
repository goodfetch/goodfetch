//! The Recording gate (ADR 0125): a test adapter of the interfaces a page
//! carries for Foxhound (spec #577's `page_value`): its navigation gate,
//! its lease check and its response observer. It records each call a page
//! makes to them and answers as its test sets: a gate that refuses the URLs
//! a test names, holds others for a while, or refuses their documents once
//! they arrive; a lease check that a test ends; an observer that keeps the
//! URLs a test names. Foxhound fills these interfaces and no Collie page
//! does, so only this adapter reaches their call sites in the page crate's
//! integration tests. Only ever a dev-dependency, never shipped.
#![expect(
    clippy::disallowed_types,
    reason = "a test adapter's records: each lock guards one list for one push or read, never across another lock or an `.await` (ADR 0060)"
)]

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use goodfetch_engine::CallError;
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded, PermitWait};
use goodfetch_page::page_value::{
    Admission, DocumentJudgment, DocumentResponse, HandlerPages, Kept, Navigation, NavigationGate,
    ObservedResponse, Observer, PageValue, Permit, Refusal, ResponseObserver, Sending,
};
use goodfetch_protocol::connection::{CALL_CANCELLED, CallCancelledDetails, StoppedBy};
use goodfetch_protocol::page::{PageId, SessionId, SitePreference};
use goodfetch_protocol::run::RunId;

/// The recording gate, lease check and observer every page of an engine
/// takes. Clones share their records.
#[derive(Clone, Debug, Default)]
pub struct RecordingGate {
    shared: Arc<Shared>,
}

#[derive(Debug, Default)]
struct Shared {
    rules: Mutex<Rules>,
    records: Mutex<Records>,
}

#[derive(Debug, Default)]
struct Rules {
    /// Refused at `admit`, by a part of their URL.
    refused: Vec<(String, SitePreference)>,
    /// Refused once their document arrives, by a part of their URL.
    refused_documents: Vec<(String, SitePreference)>,
    /// Held this long in `admit`, by a part of their URL.
    held: Vec<(String, Duration)>,
    /// How the lease ended, once a test ended it.
    lease_ended: Option<LeaseEnded>,
    /// What the observer keeps, once a test registered it.
    kept: Option<Kept>,
}

#[derive(Debug, Default)]
struct Records {
    admitted: Vec<Navigation>,
    sent: Vec<String>,
    released: Vec<String>,
    judged: Vec<String>,
    lease_checks: u64,
    waits: Vec<Wait>,
    observed: Vec<Observed>,
}

/// A permit wait the lease check held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wait {
    /// The URL the permit was for.
    pub url: String,
    /// Whether the wait ended.
    pub ended: bool,
}

/// A response the observer received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observed {
    /// The page whose request got it.
    pub page_id: PageId,
    /// Its URL.
    pub url: String,
    /// Its status.
    pub status: u16,
    /// Whether a service worker's fetch handler answered it.
    pub from_service_worker: bool,
    /// The length of its whole body.
    pub body_length: usize,
    /// Its body as received, cut at the byte cap.
    pub body: Vec<u8>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn first_match<T: Clone>(rules: &[(String, T)], url: &str) -> Option<T> {
    rules
        .iter()
        .find(|(part, _)| url.contains(part.as_str()))
        .map(|(_, value)| value.clone())
}

impl RecordingGate {
    /// A gate that admits everything at once, a lease that holds, and no
    /// observer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Refuses at `admit` every request whose URL holds `part`, for
    /// `reason`.
    pub fn refuse(&self, part: &str, reason: SitePreference) {
        lock(&self.shared.rules)
            .refused
            .push((part.to_owned(), reason));
    }

    /// Refuses the document of every navigation whose response URL holds
    /// `part`, for `reason`, once it arrives.
    pub fn refuse_documents(&self, part: &str, reason: SitePreference) {
        lock(&self.shared.rules)
            .refused_documents
            .push((part.to_owned(), reason));
    }

    /// Holds every request whose URL holds `part` for `duration` in
    /// `admit` before admitting it.
    pub fn hold(&self, part: &str, duration: Duration) {
        lock(&self.shared.rules)
            .held
            .push((part.to_owned(), duration));
    }

    /// Stops holding the requests `hold` held for `part`: those asked from
    /// now on are admitted at once.
    pub fn stop_holding(&self, part: &str) {
        lock(&self.shared.rules)
            .held
            .retain(|(held, _)| held != part);
    }

    /// Ends the lease, as `ended` says.
    pub fn end_lease(&self, ended: LeaseEnded) {
        lock(&self.shared.rules).lease_ended = Some(ended);
    }

    /// Registers the observer on the pages that open from now on, keeping
    /// what `kept` says.
    pub fn observe(&self, kept: Kept) {
        lock(&self.shared.rules).kept = Some(kept);
    }

    /// Each request the gate was asked to admit, in order.
    pub fn admitted(&self) -> Vec<Navigation> {
        lock(&self.shared.records).admitted.clone()
    }

    /// The URL of each send the page told the gate of, in order: a request
    /// sent on, and again one sent on with the Session's proxy credentials.
    pub fn sent(&self) -> Vec<String> {
        lock(&self.shared.records).sent.clone()
    }

    /// The URL of each permit released, in order.
    pub fn released(&self) -> Vec<String> {
        lock(&self.shared.records).released.clone()
    }

    /// The URL of each document the gate judged, in order.
    pub fn judged(&self) -> Vec<String> {
        lock(&self.shared.records).judged.clone()
    }

    /// How many times the lease was checked.
    pub fn lease_checks(&self) -> u64 {
        lock(&self.shared.records).lease_checks
    }

    /// Each permit wait the lease check held, in order.
    pub fn waits(&self) -> Vec<Wait> {
        lock(&self.shared.records).waits.clone()
    }

    /// Each response the observer received, in order.
    pub fn observed(&self) -> Vec<Observed> {
        lock(&self.shared.records).observed.clone()
    }
}

impl NavigationGate for RecordingGate {
    fn admit<'a>(&'a self, navigation: &'a Navigation) -> Admission<'a> {
        lock(&self.shared.records).admitted.push(navigation.clone());
        let (refused, held) = {
            let rules = lock(&self.shared.rules);
            (
                first_match(&rules.refused, &navigation.url),
                first_match(&rules.held, &navigation.url),
            )
        };
        Box::pin(async move {
            if let Some(held) = held {
                tokio::time::sleep(held).await;
            }
            if let Some(reason) = refused {
                return Err(Refusal {
                    reason,
                    message: format!("the Recording gate refuses {}", navigation.url),
                });
            }
            let (shared, url) = (Arc::clone(&self.shared), navigation.url.clone());
            Ok(Permit::new(move || {
                lock(&shared.records).released.push(url);
            }))
        })
    }

    fn sending<'a>(&'a self, url: &'a str) -> Sending<'a> {
        lock(&self.shared.records).sent.push(url.to_owned());
        Box::pin(async {})
    }

    fn judge_document<'a>(&'a self, response: &'a DocumentResponse<'a>) -> DocumentJudgment<'a> {
        lock(&self.shared.records)
            .judged
            .push(response.url.to_owned());
        let judged = match first_match(&lock(&self.shared.rules).refused_documents, response.url) {
            Some(reason) => Err(Refusal {
                reason,
                message: format!(
                    "the Recording gate refuses the document of {}",
                    response.url
                ),
            }),
            None => Ok(()),
        };
        Box::pin(std::future::ready(judged))
    }
}

/// What a held permit wait records when it ends.
struct WaitEnd {
    shared: Arc<Shared>,
    index: usize,
}

impl Drop for WaitEnd {
    fn drop(&mut self) {
        if let Some(wait) = lock(&self.shared.records).waits.get_mut(self.index) {
            wait.ended = true;
        }
    }
}

impl LeaseCheck for RecordingGate {
    fn check(&self) -> Result<(), LeaseEnded> {
        lock(&self.shared.records).lease_checks += 1;
        match lock(&self.shared.rules).lease_ended {
            Some(ended) => Err(ended),
            None => Ok(()),
        }
    }

    /// `call_cancelled`, stopped by the program: the Recording gate's stand-in
    /// for the Product's own code (Foxhound's is `attempt_ended`), which a
    /// test outside Foxhound's crates does not name.
    fn withheld(&self, ended: LeaseEnded, method: &str) -> CallError {
        CallError::new(
            &CALL_CANCELLED,
            format!("the Recording gate's lease ended ({ended}): `{method}` withheld"),
            &CallCancelledDetails {
                by: StoppedBy::Program,
                closed: None,
            },
        )
    }

    fn permit_wait(&self, url: &str) -> PermitWait {
        let mut records = lock(&self.shared.records);
        records.waits.push(Wait {
            url: url.to_owned(),
            ended: false,
        });
        let index = records.waits.len() - 1;
        drop(records);
        PermitWait::holding(WaitEnd {
            shared: Arc::clone(&self.shared),
            index,
        })
    }
}

impl ResponseObserver for RecordingGate {
    fn kept(&self) -> Kept {
        lock(&self.shared.rules).kept.clone().unwrap_or(Kept {
            urls: Vec::new(),
            byte_cap: 0,
        })
    }

    fn on_response(&self, response: &ObservedResponse<'_>, body: &[u8]) {
        lock(&self.shared.records).observed.push(Observed {
            page_id: response.page_id.clone(),
            url: response.url.to_owned(),
            body_length: response.body_length,
            status: response.status,
            from_service_worker: response.from_service_worker,
            body: body.to_vec(),
        });
    }
}

impl HandlerPages for RecordingGate {
    fn value(&self, _run: &RunId, _session: &SessionId) -> PageValue {
        PageValue::new(self.clone(), self.clone())
    }

    fn observer(&self, _run: &RunId, _session: &SessionId) -> Option<Observer> {
        lock(&self.shared.rules)
            .kept
            .is_some()
            .then(|| Observer::new(self.clone()))
    }
}
