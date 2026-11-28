//! The Wall allowance (ADR 0052; spec #577, ticket #757): reading past Walls
//! (`paywall`, `consent_wall`, `ad_blocker_wall`) only on domains a person
//! approved in `wall.domains`, refetching without the site's stored data only
//! on domains also in `wall.refetch_domains`, with `wall.denied_domains`
//! always winning (even over `*`).
//!
//! All detection is generic and rule-data driven (`goodfetch_clean_up_rules::walls`):
//! - compiled once per Run on [`RunWalls`], zero per-page recompilation;
//! - executed inside `recovery`'s single composed detection read (`page_check.js`);
//! - records Provenance actions + `wall` marker and at most one Audit log
//!   entry (`wall_allowance_used`) per `(run, domain)`;
//! - a refetch stopped by 401/403, a Block cause, or a login page ends
//!   refetching for that registrable domain for the rest of the Run and calls
//!   `RunIndex::record_refetch_stop` so `needs-you list` shows
//!   `wall_refetch_stopped`.
use goodfetch_clean_up_rules::walls::withholds_content;
#[cfg(test)]
use goodfetch_engine::CallError;
use goodfetch_engine::audit::Origin;
use goodfetch_engine::needs_you::consent::ConsentDomains;
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::foxhound::provenance::{WallKind, WallMarker};
use goodfetch_protocol::needs_you::WALL_ALLOWANCE_USED;
use goodfetch_protocol::page::{PageId, PageResponse, SessionId};
#[cfg(test)]
use goodfetch_protocol::settings::{SETTING_LOCKED, SettingLimit, SettingLockedDetails};
use goodfetch_protocol::settings::{
    SettingsLayer, WALL_DENIED_DOMAINS, WALL_DOMAINS, WALL_ON_WALL, WALL_REFETCH_DOMAINS,
};
use serde::Deserialize;
#[cfg(test)]
use serde_json::Value;
use serde_json::json;
use url::Url;

use crate::browser_run::BrowserRun;
use crate::cdp::CdpPage;
use crate::cdp::driver::PageDriver;
use crate::interception::domain_key;
use crate::page_reads::Reads;
use crate::trace;
pub(crate) use crate::wall_state::RefetchEligibility;
#[cfg(test)]
pub(crate) use crate::wall_state::{RefetchSkipReason, RunWalls};

/// Isolated-world snippet that hides a matched covering overlay and lifts any
/// `overflow: hidden` scroll lock the Wall left (ADR 0052).
const UNLOCK_WALL: &str = r"(selector) => {
  if (selector) {
    for (const el of document.querySelectorAll(selector)) {
      el.style.setProperty('display', 'none', 'important');
    }
  }
  if (document.documentElement) {
    document.documentElement.style.removeProperty('overflow');
    document.documentElement.style.removeProperty('position');
  }
  if (document.body) {
    document.body.style.removeProperty('overflow');
    document.body.style.removeProperty('position');
  }
  return true;
}";

/// Effective `wall.on_wall` mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OnWall {
    Report,
    Ask,
}

/// Resolved Wall allowance policy for a Run.
#[derive(Clone, Debug)]
pub(crate) struct WallPolicy {
    allowed: ConsentDomains,
    refetch: ConsentDomains,
    denied: ConsentDomains,
    on_wall: OnWall,
}

impl WallPolicy {
    /// Resolves `wall.*` from `settings`.
    pub(crate) fn of(settings: &Resolved) -> Self {
        let allowed = ConsentDomains::of(settings.value(&WALL_DOMAINS));
        let refetch = ConsentDomains::of(settings.value(&WALL_REFETCH_DOMAINS));
        let denied = ConsentDomains::of(settings.value(&WALL_DENIED_DOMAINS));
        let on_wall = match settings.value(&WALL_ON_WALL).as_str() {
            Some("ask") => OnWall::Ask,
            _ => OnWall::Report,
        };
        Self {
            allowed,
            refetch,
            denied,
            on_wall,
        }
    }

    /// Whether `domain` is denied by `wall.denied_domains` (always wins, even
    /// over `*`).
    pub(crate) fn is_denied(&self, domain: &str) -> bool {
        self.denied.covers(domain)
    }

    /// Whether `domain` is allowed to read past Walls (`wall.domains` minus
    /// `wall.denied_domains`).
    pub(crate) fn is_allowed(&self, domain: &str) -> bool {
        !self.is_denied(domain) && self.allowed.covers(domain)
    }

    /// Whether `domain` is allowed to refetch without site data
    /// (`wall.refetch_domains`, only when also in `wall.domains` and not
    /// denied).
    pub(crate) fn is_refetch_allowed(&self, domain: &str) -> bool {
        self.is_allowed(domain) && self.refetch.covers(domain)
    }

    /// Effective `wall.on_wall` for `domain`: a denied domain always acts as
    /// `Report` (ADR 0052).
    pub(crate) fn on_wall_for(&self, domain: &str) -> OnWall {
        if self.is_denied(domain) {
            OnWall::Report
        } else {
            self.on_wall
        }
    }

    /// Whether any domain is allowed at all (fast path for ordinary runs).
    pub(crate) fn has_any_allowance(&self) -> bool {
        !self.allowed.is_empty()
    }
}

/// Refusal when an interactive answer attempts to widen `wall.refetch_domains`,
/// which is only set in Operator policy or user/project configuration, never
/// by an answer (ADR 0052).
#[cfg(test)]
pub(crate) fn refuse_refetch_answer_widening(layer: SettingsLayer) -> CallError {
    CallError::new(
        &SETTING_LOCKED,
        "`wall.refetch_domains` can only be set in Operator policy or user/project configuration, never widened by an answer (ADR 0052).",
        &SettingLockedDetails {
            key: WALL_REFETCH_DOMAINS.name.to_owned(),
            layer,
            limit: SettingLimit {
                key: WALL_REFETCH_DOMAINS.name.to_owned(),
                value: Value::Array(Vec::new()),
                layer: SettingsLayer::OperatorPolicy,
                origin: None,
            },
            fix: "edit `wall.refetch_domains` in configuration instead of answering a pause"
                .to_owned(),
        },
    )
}

/// Wall detection output read inside `recovery`'s composed `page_check.js`.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct WallObservation {
    /// Detected Wall kind: `"paywall"` or `"ad_blocker_wall"`.
    pub(crate) kind: Option<String>,
    /// Matching overlay selector, when one covered the page.
    pub(crate) overlay_selector: Option<String>,
    /// Whether any JSON-LD `isAccessibleForFree: false` section had no text in
    /// the served HTML.
    #[serde(default)]
    pub(crate) paid_section_empty: bool,
    /// Word count of the served page's main content.
    #[serde(default)]
    pub(crate) main_words: usize,
}

impl WallObservation {
    pub(crate) fn wall_kind(&self) -> Option<WallKind> {
        match self.kind.as_deref() {
            Some("paywall") => Some(WallKind::Paywall),
            Some("ad_blocker_wall") => Some(WallKind::AdBlockerWall),
            Some("consent_wall") => Some(WallKind::ConsentWall),
            _ => None,
        }
    }
}

/// Applies the Wall allowance after `recovery`'s composed read detects a Wall.
pub(crate) async fn handle_detected_wall(
    run: &BrowserRun,
    session: &SessionId,
    page_id: &PageId,
    driver: &PageDriver,
    response: &PageResponse,
    observation: &WallObservation,
    eligibility: RefetchEligibility,
) {
    let Some(kind) = observation.wall_kind() else {
        return;
    };
    let Some(domain) = domain_key(&response.url) else {
        return;
    };
    let Some(settings) = run
        .identity
        .settings()
        .map(goodfetch_engine::settings::RunSettings::resolved)
    else {
        return;
    };
    let policy = WallPolicy::of(settings);
    if !policy.is_allowed(&domain) {
        let mode = policy.on_wall_for(&domain);
        trace::provenance(
            &run.trace,
            json!({
                "action": "left",
                "page_id": page_id,
                "session_id": session,
                "url": response.url,
                "domain": domain,
                "wall": WallMarker { kind, vendor: None },
                "on_wall": match mode {
                    OnWall::Report => "report",
                    OnWall::Ask => "ask",
                },
            }),
        )
        .await;
        return;
    }

    let mut actions = vec!["read_served"];
    if let Some(selector) = observation.overlay_selector.as_deref() {
        let _ = Reads::of(CdpPage::of(driver))
            .in_page(UNLOCK_WALL, json!(selector), "wall.hide")
            .await;
        actions.push("hidden");
    }

    let marker = WallMarker { kind, vendor: None };
    trace::provenance(
        &run.trace,
        json!({
            "action": "wall_allowance",
            "page_id": page_id,
            "session_id": session,
            "url": response.url,
            "domain": domain,
            "wall": marker,
            "actions": actions,
        }),
    )
    .await;

    if run.walls.mark_audited(&domain) {
        let origin = Origin {
            actor: Actor::Engine,
            surface: None,
            layer: Some(SettingsLayer::UserConfiguration),
            run_id: Some(run.identity.id().clone()),
        };
        let _ = run
            .engine
            .audit()
            .record(
                &origin,
                &WALL_ALLOWANCE_USED,
                json!({
                    "domain": domain,
                    "kind": kind,
                    "actions": actions,
                }),
                run.identity.clock().timestamp(),
            )
            .await;
    }

    if kind != WallKind::Paywall
        || !withholds_content(true, observation.paid_section_empty, observation.main_words)
        || !policy.is_refetch_allowed(&domain)
    {
        return;
    }
    attempt_refetch(
        run,
        session,
        page_id,
        driver,
        response,
        &domain,
        kind,
        eligibility,
    )
    .await;
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal helper carries the page's navigation context for one uncached refetch"
)]
async fn attempt_refetch(
    run: &BrowserRun,
    session: &SessionId,
    page_id: &PageId,
    driver: &PageDriver,
    response: &PageResponse,
    domain: &str,
    kind: WallKind,
    eligibility: RefetchEligibility,
) {
    if let Err(reason) = run
        .walls
        .try_begin_refetch(&response.url, domain, session, eligibility)
    {
        trace::provenance(
            &run.trace,
            json!({
                "action": "refetch_skipped",
                "page_id": page_id,
                "session_id": session,
                "url": response.url,
                "domain": domain,
                "reason": reason.as_str(),
            }),
        )
        .await;
        return;
    }

    let origin = Url::parse(&response.url)
        .map(|parsed| parsed.origin().ascii_serialization())
        .unwrap_or_default();
    let refetch_status = CdpPage::of(driver)
        .clear_site_data_and_reload_uncached(&origin)
        .await;

    match refetch_status {
        Ok(Some(status)) if status == 401 || status == 403 => {
            let cause = format!("http_{status}");
            record_refetch_stop(run, session, page_id, &response.url, domain, &cause).await;
        }
        Ok(_) => {
            trace::provenance(
                &run.trace,
                json!({
                    "action": "refetched_without_site_data",
                    "page_id": page_id,
                    "session_id": session,
                    "url": response.url,
                    "domain": domain,
                    "wall": WallMarker { kind, vendor: None },
                }),
            )
            .await;
        }
        Err(_) => {
            record_refetch_stop(
                run,
                session,
                page_id,
                &response.url,
                domain,
                "refetch_failed",
            )
            .await;
        }
    }
}

/// Records a stopped refetch in `RunWalls`, Provenance, and `RunIndex` so
/// `needs-you list` shows `wall_refetch_stopped` (ADR 0052; moved from #862).
pub(crate) async fn record_refetch_stop(
    run: &BrowserRun,
    session: &SessionId,
    page_id: &PageId,
    url: &str,
    domain: &str,
    cause: &str,
) {
    run.walls.mark_stopped(domain);
    trace::provenance(
        &run.trace,
        json!({
            "action": "refetch_stopped",
            "page_id": page_id,
            "session_id": session,
            "url": url,
            "domain": domain,
            "cause": cause,
        }),
    )
    .await;
    let _ = run
        .engine
        .run_index()
        .record_refetch_stop(&run.identity, domain.to_owned(), cause.to_owned())
        .await;
}

#[cfg(test)]
mod tests;
