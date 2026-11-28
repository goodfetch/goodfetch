//! Source domain bounds and Human pause approval gates (ADRs 0056 and 0092;
//! ticket #745).
//!
//! - [`SourceDomains`] holds the registrable domains parsed once when a
//!   Session opens from a bound source (`running_browser`, `profile_directory`,
//!   or `cdp_url` naming `domains`). Step 3 of the interception pipeline checks
//!   top-level navigations and new tabs against it: for the person's own code
//!   an unlisted domain fails immediately with `domain_not_allowed`, and for a
//!   Client acting for a model or a Recipe run it raises a Human pause asking
//!   a person to approve widening the Session's domains.
//! - [`Approvals`] tracks every pending Human pause waiting for a person's
//!   approval (unlisted domain navigation, or a model's / Recipe's login
//!   import, borrow, save, deletion, or export), publishes each as an
//!   `approval_requested` "Needs you" item (and as a `run.progress` Human
//!   pause whenever inside a Run), and resolves it when a person calls
//!   `needs_you.answer_approval` (or `page.pause_answer` for a page-level
//!   navigation pause). `session.recovery.human_pause: false` never skips
//!   these approvals, which wait under `timeout.human_pause`.
#![expect(
    clippy::disallowed_types,
    reason = "approval table and source-domain set locks are held only for brief map/set operations, never across an `.await` or another lock, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use goodfetch_engine::audit::Origin;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::needs_you::{Answering, Changes, NeedsYouSource, Primitives, SourceContext};
use goodfetch_engine::settings::RunSettings;
use goodfetch_engine::{CallContext, CallError, EngineContext, host_name};
use goodfetch_protocol::audit::{Actor, HUMAN_PAUSE_ENDED};
use goodfetch_protocol::catalog::SharedTypeValue;
use goodfetch_protocol::connection::{CancelledBy, ClosedValue, WaitedFor};
use goodfetch_protocol::needs_you::NeedsYouItem;
use goodfetch_protocol::page::pause::{
    APPROVAL_REQUESTED_KIND, ApprovalAnswered, HUMAN_PAUSE_DECLINED, HumanPauseDeclinedDetails,
    NEEDS_YOU_ANSWER_APPROVAL, PagePauseAnswered, PauseAnswer,
};
use goodfetch_protocol::page::session::{DOMAIN_NOT_ALLOWED, DomainNotAllowedDetails};
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::run::{EngineStep, HumanPause, PauseOrigin, RunId};
use goodfetch_protocol::settings::session_source::SessionSource;
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_HUMAN_PAUSE};
use jiff::Timestamp;
use serde_json::{Map, Value, json};
use tokio::sync::{oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::browser_run::BrowserRun;
use crate::errors::cancelled;

/// The registrable domains a Session source binds top-level navigation to
/// (ADRs 0056 and 0117; ticket #745), parsed once when the Session opens.
pub(crate) struct SourceDomains {
    source_kind: &'static str,
    requires_human_approval: bool,
    allowed: Mutex<BTreeSet<String>>,
}

impl SourceDomains {
    /// Builds navigation domain bounds for `source` when its kind binds
    /// top-level navigations (`running_browser`, `profile_directory`, or
    /// `cdp_url` naming `domains`).
    pub(crate) fn for_navigation(
        source: &SessionSource,
        requires_human_approval: bool,
    ) -> Option<Self> {
        let domains: &[String] = match source {
            SessionSource::RunningBrowser { domains, .. }
            | SessionSource::ProfileDirectory { domains, .. }
            | SessionSource::CdpUrl {
                domains: Some(domains),
                ..
            } => domains,
            _ => return None,
        };
        let mut allowed = BTreeSet::new();
        for domain in domains {
            if let Some(normalized) = normalize_domain(domain) {
                allowed.insert(normalized);
            }
        }
        Some(Self {
            source_kind: source.kind(),
            requires_human_approval,
            allowed: Mutex::new(allowed),
        })
    }

    /// The `session.source` kind that created these bounds.
    pub(crate) const fn source_kind(&self) -> &'static str {
        self.source_kind
    }

    /// Whether an unlisted navigation raises a Human pause (model or Recipe
    /// run) rather than failing immediately with `domain_not_allowed`.
    pub(crate) const fn requires_human_approval(&self) -> bool {
        self.requires_human_approval
    }

    /// Checks whether a top-level navigation to `url` falls within the allowed
    /// registrable domains; non-HTTP(S) URLs (such as `about:blank`) are left
    /// to normal egress rules.
    ///
    /// # Errors
    ///
    /// Returns the unlisted registrable domain when `url` is outside `allowed`.
    pub(crate) fn check_url(&self, url: &str) -> Result<(), String> {
        let Ok(parsed) = url::Url::parse(url) else {
            return Ok(());
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            return Ok(());
        }
        let domain = match parsed.host() {
            Some(url::Host::Domain(host)) => {
                let Some(normalized) = normalize_domain(host) else {
                    return Ok(());
                };
                normalized
            }
            Some(url::Host::Ipv4(address)) => address.to_string(),
            Some(url::Host::Ipv6(address)) => address.to_string(),
            None => return Ok(()),
        };
        let allowed = self.allowed.lock().unwrap_or_else(PoisonError::into_inner);
        if allowed.contains(&domain) {
            Ok(())
        } else {
            Err(domain)
        }
    }

    /// Adds a person-approved registrable domain to this live Session's bounds.
    pub(crate) fn allow_domain(&self, domain: &str) {
        if let Some(normalized) = normalize_domain(domain) {
            self.allowed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(normalized);
        }
    }

    /// Snapshot of currently allowed registrable domains, sorted.
    pub(crate) fn allowed_list(&self) -> Vec<String> {
        self.allowed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    /// Builds the `domain_not_allowed` refusal for the person's own code.
    pub(crate) fn refused_error(&self, domain: &str, url: Option<&str>) -> CallError {
        let allowed = self.allowed_list();
        let allowed_display = if allowed.is_empty() {
            "none".to_owned()
        } else {
            allowed.join(", ")
        };
        CallError::new(
            &DOMAIN_NOT_ALLOWED,
            format!(
                "Navigation to `{domain}` is outside the `{}` source's allowed domains ({allowed_display}).",
                self.source_kind
            ),
            &DomainNotAllowedDetails {
                domain: domain.to_owned(),
                allowed,
                source: self.source_kind.to_owned(),
                url: url.map(str::to_owned),
                fix: format!(
                    "Add `{domain}` to `session.source.domains` if this Session should reach it."
                ),
            },
        )
    }
}

/// Normalizes a domain string to its lowercase registrable domain (ADR 0117).
fn normalize_domain(domain: &str) -> Option<String> {
    let key = host_name::key(domain)?;
    Some(host_name::registrable(&key).unwrap_or(&key).to_owned())
}

/// What a pending Human pause is asking a person to approve (ADR 0056; #745).
#[derive(Clone, Debug)]
pub(crate) enum ApprovalAction {
    /// Navigating a bound Session's page to an unlisted registrable domain.
    NavigateUnlistedDomain {
        session_id: SessionId,
        page_id: PageId,
        source_kind: &'static str,
        domain: String,
        url: String,
        allowed: Vec<String>,
    },
    /// Importing a login from a Session source into a Session or Saved identity.
    ImportLogin {
        source_kind: &'static str,
        domains: Vec<String>,
        target: String,
    },
    /// Borrowing a browser login (`running_browser`, `cdp_url`, or `client_browser`).
    BorrowLogin {
        source_kind: &'static str,
        domains: Vec<String>,
        target: String,
    },
    /// Saving a live Session's cookies and storage as a Saved identity.
    SaveIdentity {
        session_id: SessionId,
        name: String,
        domains: Vec<String>,
    },
    /// Deleting a Saved identity from the per-major store.
    DeleteIdentity { name: String },
    /// Exporting a Saved identity to an encrypted session bundle.
    ExportIdentity { name: String },
}

impl ApprovalAction {
    /// Short machine-readable action tag.
    pub(crate) const fn kind_name(&self) -> &'static str {
        match self {
            Self::NavigateUnlistedDomain { .. } => "navigate_unlisted_domain",
            Self::ImportLogin { .. } => "import_login",
            Self::BorrowLogin { .. } => "borrow_login",
            Self::SaveIdentity { .. } => "save_identity",
            Self::DeleteIdentity { .. } => "delete_identity",
            Self::ExportIdentity { .. } => "export_identity",
        }
    }

    /// Plain-English description of what would happen if approved.
    pub(crate) fn summary(&self) -> String {
        match self {
            Self::NavigateUnlistedDomain {
                session_id,
                source_kind,
                domain,
                allowed,
                ..
            } => format!(
                "Allow navigating `{source_kind}` Session `{}` to unlisted domain `{domain}` (currently bound to: {})",
                session_id.0,
                format_domains(allowed),
            ),
            Self::ImportLogin {
                source_kind,
                domains,
                target,
            } => format!(
                "Allow importing login from `{source_kind}` into {target} (domains: {})",
                format_domains(domains),
            ),
            Self::BorrowLogin {
                source_kind,
                domains,
                target,
            } => format!(
                "Allow borrowing browser login via `{source_kind}` for {target} (domains: {})",
                format_domains(domains),
            ),
            Self::SaveIdentity {
                session_id,
                name,
                domains,
            } => format!(
                "Allow saving Session `{}` as Saved identity `{name}` (domains: {})",
                session_id.0,
                format_domains(domains),
            ),
            Self::DeleteIdentity { name } => {
                format!("Allow deleting Saved identity `{name}` from this Product major's store")
            }
            Self::ExportIdentity { name } => {
                format!("Allow exporting Saved identity `{name}` as an encrypted session bundle")
            }
        }
    }

    /// Structured details included on the `NeedsYouItem`.
    fn details(&self, actor: &str) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("action".to_owned(), json!(self.kind_name()));
        map.insert("what_would_happen".to_owned(), json!(self.summary()));
        map.insert("actor".to_owned(), json!(actor));
        match self {
            Self::NavigateUnlistedDomain {
                session_id,
                page_id,
                source_kind,
                domain,
                url,
                allowed,
            } => {
                map.insert("session_id".to_owned(), json!(session_id.0));
                map.insert("page_id".to_owned(), json!(page_id.0));
                map.insert("source_kind".to_owned(), json!(source_kind));
                map.insert("target_domain".to_owned(), json!(domain));
                map.insert("url".to_owned(), json!(url));
                map.insert("domains".to_owned(), json!(allowed));
            }
            Self::ImportLogin {
                source_kind,
                domains,
                target,
            }
            | Self::BorrowLogin {
                source_kind,
                domains,
                target,
            } => {
                map.insert("source_kind".to_owned(), json!(source_kind));
                map.insert("domains".to_owned(), json!(domains));
                map.insert("target".to_owned(), json!(target));
            }
            Self::SaveIdentity {
                session_id,
                name,
                domains,
            } => {
                map.insert("session_id".to_owned(), json!(session_id.0));
                map.insert("identity_name".to_owned(), json!(name));
                map.insert("domains".to_owned(), json!(domains));
            }
            Self::DeleteIdentity { name } | Self::ExportIdentity { name } => {
                map.insert("identity_name".to_owned(), json!(name));
            }
        }
        map
    }

    fn session_and_page(&self) -> (String, String) {
        match self {
            Self::NavigateUnlistedDomain {
                session_id,
                page_id,
                ..
            } => (session_id.0.clone(), page_id.0.clone()),
            Self::SaveIdentity { session_id, .. } => (session_id.0.clone(), String::new()),
            Self::ImportLogin { .. }
            | Self::BorrowLogin { .. }
            | Self::DeleteIdentity { .. }
            | Self::ExportIdentity { .. } => (String::new(), String::new()),
        }
    }

    fn page_id(&self) -> Option<PageId> {
        match self {
            Self::NavigateUnlistedDomain { page_id, .. } => Some(page_id.clone()),
            _ => None,
        }
    }
}

fn format_domains(domains: &[String]) -> String {
    if domains.is_empty() {
        "none specified".to_owned()
    } else {
        domains.join(", ")
    }
}

/// Parameters for waiting on a person's approval.
pub(crate) struct ApprovalWait<'a> {
    pub(crate) action: ApprovalAction,
    pub(crate) actor: String,
    pub(crate) run: Option<&'a BrowserRun>,
    pub(crate) engine: &'a EngineContext,
    pub(crate) cancellation: CancellationToken,
}

struct ApprovalDecision {
    approved: bool,
    person: String,
    note: Option<String>,
}

struct PendingApproval {
    item: NeedsYouItem,
    page_id: Option<PageId>,
    tx: Option<oneshot::Sender<ApprovalDecision>>,
}

struct ApprovalsShared {
    next_id: AtomicU64,
    pending: Mutex<BTreeMap<String, PendingApproval>>,
    changed: watch::Sender<u64>,
}

impl Default for ApprovalsShared {
    fn default() -> Self {
        let (changed, _) = watch::channel(0u64);
        Self {
            next_id: AtomicU64::new(1),
            pending: Mutex::new(BTreeMap::new()),
            changed,
        }
    }
}

/// Shared registry of pending Human pause approvals across all Sessions,
/// pages, and `identity.*` operations (ADR 0056; ticket #745).
#[derive(Clone, Default)]
pub(crate) struct Approvals(Arc<ApprovalsShared>);

impl Approvals {
    /// Registers this approval table as a "Needs you" source so every pending
    /// approval appears in `needs_you.list` and emits `needs_you.changed`.
    pub(crate) fn needs_you_source(&self) -> NeedsYouSource {
        NeedsYouSource::new(ApprovalNeedsYou(Arc::clone(&self.0)))
    }

    /// Waits for a person to approve or decline `params.action` under
    /// `timeout.human_pause`, never skipped by `session.recovery.human_pause: false`.
    pub(crate) async fn wait_for_approval(
        &self,
        params: ApprovalWait<'_>,
    ) -> Result<String, CallError> {
        let ApprovalWait {
            action,
            actor,
            run,
            engine,
            cancellation,
        } = params;
        let clock = engine.clock();
        let since = clock.point_in_time();
        let started = clock.now();
        let deadline = approval_deadline(run, engine).await;
        let deadline_at = deadline.as_ref().map(|limit| since + limit.duration());

        let seq = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        let id = format!("approval-{seq}");
        let summary = action.summary();
        let page_id = action.page_id();
        let (session_name, page_name) = action.session_and_page();
        let run_id = run.map(|r| r.identity.id().clone());

        let item = NeedsYouItem {
            id: id.clone(),
            kind: APPROVAL_REQUESTED_KIND.to_owned(),
            since,
            run: run_id.clone(),
            schedule: None,
            summary: summary.clone(),
            actions: vec![NEEDS_YOU_ANSWER_APPROVAL.name.to_owned()],
            details: action.details(&actor),
        };

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self
                .0
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            pending.insert(
                id.clone(),
                PendingApproval {
                    item,
                    page_id,
                    tx: Some(tx),
                },
            );
        }
        self.notify_changed();

        let _guard = ApprovalGuard {
            shared: Arc::clone(&self.0),
            id: id.clone(),
        };
        let _progress_held = run.map(|r| {
            r.progress.pause(HumanPause {
                session: session_name,
                page: page_name,
                reason: Some(summary.clone()),
                until: None,
                raised_by: PauseOrigin::Engine {
                    step: EngineStep::Approval,
                },
                since,
                deadline: deadline_at,
            })
        });

        let mut timer = std::pin::pin!(async {
            if let Some(limit) = &deadline {
                clock.sleep(limit.duration()).await;
            } else {
                std::future::pending::<()>().await;
            }
        });

        let decision = tokio::select! {
            biased;
            res = rx => res.map_err(|_| cancelled(CancelledBy::Program, ClosedValue::Session))?,
            () = cancellation.cancelled() => {
                return Err(cancelled(CancelledBy::Program, ClosedValue::Session));
            }
            () = &mut timer => {
                let elapsed = clock.now() - started;
                let limit = deadline
                    .as_ref()
                    .expect("invariant: timer fired only when deadline is Some");
                return Err(limit.exceeded(WaitedFor::ANSWER, elapsed));
            }
        };

        record_approval_audit(
            engine,
            run_id,
            &action,
            &decision.person,
            decision.approved,
            decision.note.as_deref(),
        )
        .await;

        if decision.approved {
            Ok(decision.person)
        } else {
            Err(declined_error(Some(decision.person), decision.note))
        }
    }

    /// Delivers a person's answer to the pending approval `id`.
    pub(crate) fn answer(
        &self,
        id: &str,
        approve: bool,
        person: String,
        note: Option<String>,
    ) -> ApprovalAnswered {
        let entry = {
            let mut pending = self
                .0
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            pending.remove(id)
        };
        let Some(mut entry) = entry else {
            return ApprovalAnswered {
                accepted: false,
                reason: Some(format!("no pending approval `{id}` is waiting")),
            };
        };
        self.notify_changed();
        let Some(tx) = entry.tx.take() else {
            return ApprovalAnswered {
                accepted: false,
                reason: Some(format!("approval `{id}` was already answered")),
            };
        };
        if tx
            .send(ApprovalDecision {
                approved: approve,
                person,
                note,
            })
            .is_err()
        {
            return ApprovalAnswered {
                accepted: false,
                reason: Some("the waiting approval already ended".to_owned()),
            };
        }
        ApprovalAnswered {
            accepted: true,
            reason: None,
        }
    }

    /// Delivers a `page.pause_answer` to an approval waiting on `page_id`, if
    /// any. Returns `None` when no approval is pending on `page_id` so the
    /// caller can fall through to `ActivePauses`.
    pub(crate) fn answer_for_page(
        &self,
        page_id: &PageId,
        answer: PauseAnswer,
        person: String,
        note: Option<String>,
    ) -> Option<PagePauseAnswered> {
        let id = {
            let pending = self
                .0
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            pending
                .iter()
                .find(|(_, entry)| entry.page_id.as_ref() == Some(page_id) && entry.tx.is_some())
                .map(|(id, _)| id.clone())
        }?;
        let approve = !matches!(answer, PauseAnswer::Decline);
        let answered = self.answer(&id, approve, person, note);
        Some(PagePauseAnswered {
            accepted: answered.accepted,
            reason: answered.reason,
        })
    }

    fn notify_changed(&self) {
        self.0.changed.send_modify(|n| *n = n.wrapping_add(1));
    }
}

struct ApprovalGuard {
    shared: Arc<ApprovalsShared>,
    id: String,
}

impl Drop for ApprovalGuard {
    fn drop(&mut self) {
        let removed = {
            let mut pending = self
                .shared
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            pending.remove(&self.id).is_some()
        };
        if removed {
            self.shared.changed.send_modify(|n| *n = n.wrapping_add(1));
        }
    }
}

struct ApprovalNeedsYou(Arc<ApprovalsShared>);

impl Primitives for ApprovalNeedsYou {
    fn dismissable(&self) -> bool {
        false
    }

    fn items<'a>(
        &'a self,
        _context: &'a SourceContext,
    ) -> Answering<'a, Result<Vec<NeedsYouItem>, goodfetch_engine::needs_you::SourceError>> {
        let items = {
            let pending = self
                .0
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            pending
                .values()
                .map(|entry| entry.item.clone())
                .collect::<Vec<_>>()
        };
        Box::pin(async move { Ok(items) })
    }

    fn changes(&self, _context: &SourceContext) -> Changes {
        Changes::of(self.0.changed.subscribe())
    }
}

/// Resolves `timeout.human_pause` (ignoring `session.recovery.human_pause`).
async fn approval_deadline(run: Option<&BrowserRun>, engine: &EngineContext) -> Option<Deadline> {
    if let Some(run) = run {
        let Some(resolved) = run.identity.settings().map(RunSettings::resolved) else {
            return Some(Deadline::declared(&TIMEOUT_HUMAN_PAUSE));
        };
        if let Some(effective) = resolved.effective(&TIMEOUT_HUMAN_PAUSE) {
            if effective.value.is_null() {
                return None;
            }
            return Deadline::of(resolved, &TIMEOUT_HUMAN_PAUSE);
        }
        return Some(Deadline::declared(&TIMEOUT_HUMAN_PAUSE));
    }
    let snapshot = engine.settings().snapshot().await;
    let Ok(resolved) = snapshot.resolved() else {
        return Some(Deadline::declared(&TIMEOUT_HUMAN_PAUSE));
    };
    if let Some(effective) = resolved.effective(&TIMEOUT_HUMAN_PAUSE) {
        if effective.value.is_null() {
            return None;
        }
        if let Some(duration) = resolved.get::<Option<Duration>>(&TIMEOUT_HUMAN_PAUSE) {
            return Deadline::argument(
                &TIMEOUT_HUMAN_PAUSE,
                u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
            )
            .ok();
        }
        return None;
    }
    Some(Deadline::declared(&TIMEOUT_HUMAN_PAUSE))
}

fn declined_error(person: Option<String>, note: Option<String>) -> CallError {
    let message = match (&person, &note) {
        (Some(who), Some(why)) => format!("Human pause approval declined by `{who}`: {why}"),
        (Some(who), None) => format!("Human pause approval declined by `{who}`."),
        (None, Some(why)) => format!("Human pause approval declined: {why}"),
        (None, None) => "Human pause approval declined by a person.".to_owned(),
    };
    CallError::new(
        &HUMAN_PAUSE_DECLINED,
        message,
        &HumanPauseDeclinedDetails { person, note },
    )
}

async fn record_approval_audit(
    engine: &EngineContext,
    run_id: Option<RunId>,
    action: &ApprovalAction,
    person: &str,
    approved: bool,
    note: Option<&str>,
) {
    let origin = Origin {
        actor: Actor::Person {
            name: person.to_owned(),
        },
        surface: None,
        layer: None,
        run_id,
    };
    let now = engine.clock().timestamp();
    let mut details = json!({
        "action": action.kind_name(),
        "ended": if approved { "approved" } else { "declined" },
        "person": person,
        "summary": action.summary(),
    });
    if let Some(note) = note {
        details["note"] = json!(note);
    }
    if let Err(error) = engine
        .audit()
        .record(&origin, &HUMAN_PAUSE_ENDED, details, now)
        .await
    {
        tracing::warn!(%error, "could not record approval Human pause in Audit log");
    }
}

/// Records one Audit log entry for a login import or borrow with the caller's
/// actor (ADRs 0056 and 0140).
pub(crate) async fn record_login_audit(
    context: &CallContext,
    run_id: Option<RunId>,
    layer: Option<SettingsLayer>,
    kind: &'static SharedTypeValue,
    details: Value,
) {
    let surface = context.surface();
    let origin = Origin {
        actor: Actor::ClientSurface { surface },
        surface: Some(surface),
        layer,
        run_id,
    };
    let now = Timestamp::try_from(context.engine().clock().point_in_time())
        .unwrap_or(Timestamp::UNIX_EPOCH);
    if let Err(error) = context
        .engine()
        .audit()
        .record(&origin, kind, details, now)
        .await
    {
        tracing::warn!(%error, kind = kind.name, "could not record login event in Audit log");
    }
}

/// Human-readable actor label for approval prompts.
pub(crate) fn caller_actor_label(context: &CallContext, is_recipe_run: bool) -> String {
    if is_recipe_run {
        format!("recipe ({})", context.surface().as_str())
    } else {
        context.surface().as_str().to_owned()
    }
}

/// Whether `source` requires a person's approval when opened by a Client
/// acting for a model or a Recipe run (ADR 0056): imports and borrows set by
/// the Run, Recipe, or a model's project configuration wait; sources already
/// pre-configured in Operator policy or user configuration do not.
pub(crate) fn source_requires_approval(
    source: &SessionSource,
    layer: SettingsLayer,
    acting_for_model: bool,
    is_recipe_run: bool,
) -> bool {
    if !(acting_for_model || is_recipe_run) {
        return false;
    }
    if matches!(
        layer,
        SettingsLayer::OperatorPolicy | SettingsLayer::UserConfiguration
    ) {
        return false;
    }
    matches!(
        source,
        SessionSource::StateFile { .. }
            | SessionSource::CookieHeader { .. }
            | SessionSource::InstalledBrowser { .. }
            | SessionSource::ProfileDirectory { .. }
            | SessionSource::RunningBrowser { .. }
            | SessionSource::CdpUrl { .. }
    )
}

/// Builds the [`ApprovalAction`] for opening a Session from `source`.
pub(crate) fn approval_action_for_source(source: &SessionSource, run_id: &RunId) -> ApprovalAction {
    let target = format!("Run `{}`", run_id.as_str());
    let domains: Vec<String> = match source {
        SessionSource::CookieHeader { domain, .. } => vec![domain.clone()],
        other => other.domains().to_vec(),
    };
    match source {
        SessionSource::RunningBrowser { .. } | SessionSource::CdpUrl { .. } => {
            ApprovalAction::BorrowLogin {
                source_kind: source.kind(),
                domains,
                target,
            }
        }
        _ => ApprovalAction::ImportLogin {
            source_kind: source.kind(),
            domains,
            target,
        },
    }
}
