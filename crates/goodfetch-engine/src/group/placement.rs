//! Group run placement and governor headroom change detection (ADRs 0040,
//! 0055 and 0056, issue 244).
//!
//! Group members push their Resource governor headroom and capabilities to the
//! Lead Daemon whenever headroom changes noticeably. The Lead Daemon filters
//! eligible Group members by readiness, Product minor version (at least the
//! run state's minor for resumed or handed-off runs), capabilities (browsers,
//! Plugins, local addresses, local model servers), explicit `group_member` pin,
//! and ADR 0056 machine-bound / `client_browser` constraints, then picks the
//! eligible Group member with the most headroom.

use std::net::IpAddr;

use goodfetch_protocol::checkpoint::{SESSION_BOUND_TO_MACHINE, SessionBoundToMachineDetails};
use goodfetch_protocol::formats::{Compatibility, RUN_STATE};
use goodfetch_protocol::governor::{GovernorLevel, SystemInfo};
use goodfetch_protocol::group::{
    GROUP_MEMBER_UNKNOWN, MemberCapabilities, MemberHeadroom, MemberRoute, PlacementRequest,
};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::session_source::{BrowserName, SessionSource};
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SettingsLayer,
};

use crate::call_error::CallError;
use crate::run::format_refusal;

/// Snapshot of one Group member's placement metadata held by the Lead Daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRecord {
    /// Its Group member name.
    pub name: String,
    /// Its peer listener `host:port`.
    pub peer_address: String,
    /// Its latest reported Resource governor headroom.
    pub headroom: MemberHeadroom,
    /// Its latest reported capabilities, version and `daemon.advertise_url`.
    pub capabilities: MemberCapabilities,
}

impl MemberRecord {
    /// Its route for forwarding or redirecting Client traffic.
    #[must_use]
    pub fn route(&self) -> MemberRoute {
        MemberRoute {
            member: self.name.clone(),
            peer_address: self.peer_address.clone(),
            advertise_url: self.capabilities.advertise_url.clone(),
        }
    }
}

/// Computes [`MemberHeadroom`] from a Resource governor [`SystemInfo`] sample.
#[must_use]
pub fn headroom_from_system_info(info: Option<&SystemInfo>, ready: bool) -> MemberHeadroom {
    let Some(info) = info else {
        return MemberHeadroom {
            http_available: 0,
            browser_available: 0,
            model_available: 0,
            level: GovernorLevel::Normal,
            ready,
        };
    };
    MemberHeadroom {
        http_available: info
            .concurrency
            .http
            .target
            .saturating_sub(info.concurrency.http.in_use),
        browser_available: info
            .concurrency
            .browser_page
            .target
            .saturating_sub(info.concurrency.browser_page.in_use),
        model_available: info
            .concurrency
            .model_call
            .target
            .saturating_sub(info.concurrency.model_call.in_use),
        level: info.level,
        ready,
    }
}

/// Whether a Group member's headroom or capabilities changed noticeably enough
/// to push an update to the Lead Daemon (ADR 0055).
///
/// Fires on the initial report, whenever readiness, pressure level, or any
/// capability (`browsers`, `plugins`, `local_addresses`,
/// `local_model_servers`, `session_sources`, `advertise_url`, `product_version`)
/// changes, or whenever any concurrency headroom count crosses zero or changes
/// by at least 2 slots and at least 10% of the previous value.
#[must_use]
pub fn headroom_changed_noticeably(
    previous: Option<&(MemberHeadroom, MemberCapabilities)>,
    current: &(MemberHeadroom, MemberCapabilities),
) -> bool {
    let Some((prev_headroom, prev_caps)) = previous else {
        return true;
    };
    let (cur_headroom, cur_caps) = current;
    if prev_caps != cur_caps
        || prev_headroom.ready != cur_headroom.ready
        || prev_headroom.level != cur_headroom.level
    {
        return true;
    }
    slot_changed_noticeably(prev_headroom.http_available, cur_headroom.http_available)
        || slot_changed_noticeably(
            prev_headroom.browser_available,
            cur_headroom.browser_available,
        )
        || slot_changed_noticeably(prev_headroom.model_available, cur_headroom.model_available)
}

fn slot_changed_noticeably(prev: u32, cur: u32) -> bool {
    if (prev == 0) != (cur == 0) {
        return true;
    }
    let delta = prev.abs_diff(cur);
    delta >= 2 && u64::from(delta) * 10 >= u64::from(prev)
}

/// Canonical capability identifier for a machine-bound [`SessionSource`]
/// (ADR 0056). Returns `None` for sources that are portable across Group
/// members (`saved_identity`, `state_file`, `cookie_header`, or a non-loopback
/// `cdp_url`).
#[must_use]
pub fn machine_bound_source_key(source: &SessionSource) -> Option<String> {
    match source {
        SessionSource::ProfileDirectory { path, browser, .. } => Some(format!(
            "profile_directory:{}:{path}",
            browser_name(*browser)
        )),
        SessionSource::InstalledBrowser { browser, .. } => {
            Some(format!("installed_browser:{}", browser_name(*browser)))
        }
        SessionSource::RunningBrowser { browser, .. } => {
            Some(format!("running_browser:{}", browser_name(*browser)))
        }
        SessionSource::CdpUrl { url, .. } if is_loopback_cdp_url(url) => {
            Some(format!("cdp_url:{url}"))
        }
        SessionSource::SavedIdentity { .. }
        | SessionSource::StateFile { .. }
        | SessionSource::CookieHeader { .. }
        | SessionSource::CdpUrl { .. } => None,
    }
}

/// Whether `url` targets a loopback host (`localhost`, `127.0.0.0/8`, `::1`,
/// or IPv4-mapped loopback), making a `cdp_url` machine-bound under ADR 0056.
#[must_use]
pub fn is_loopback_cdp_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let trimmed = host.trim_start_matches('[').trim_end_matches(']');
    if trimmed.eq_ignore_ascii_case("localhost") || trimmed.ends_with(".localhost") {
        return true;
    }
    trimmed
        .parse::<IpAddr>()
        .is_ok_and(|ip| ip.to_canonical().is_loopback())
}

fn browser_name(browser: BrowserName) -> &'static str {
    match browser {
        BrowserName::Chrome => "chrome",
        BrowserName::Edge => "edge",
        BrowserName::Brave => "brave",
        BrowserName::Chromium => "chromium",
        BrowserName::Firefox => "firefox",
        BrowserName::Safari => "safari",
    }
}

/// Selects the best eligible Group member from `members` to run `request`
/// (ADRs 0055 and 0056).
///
/// # Errors
///
/// - `group_member_unknown` when `request.group_member` names no Group member;
/// - `session_bound_to_machine` when a `client_browser`, machine-bound import,
///   or machine-bound `session.source` cannot be placed on its required Group
///   member (or conflicts with a `group_member` pin);
/// - `format_version_unsupported` when no candidate's Product minor is at least
///   `request.run_state_version`;
/// - `capability_unavailable` when no eligible ready Group member satisfies the
///   run's capabilities.
pub fn select_member(
    members: &[MemberRecord],
    request: &PlacementRequest,
) -> Result<MemberRoute, CallError> {
    let candidates = filter_binding_and_pin(members, request)?;

    // 5. Readiness check.
    let ready_candidates: Vec<&MemberRecord> = candidates
        .iter()
        .copied()
        .filter(|m| m.headroom.ready)
        .collect();
    if ready_candidates.is_empty() {
        return Err(CallError::new(
            &CAPABILITY_UNAVAILABLE,
            "No eligible Group member is ready to accept this Run right now.",
            &CapabilityUnavailableDetails {
                key: "group_member".to_owned(),
                layer: SettingsLayer::Run,
                kind: "group_member_ready".to_owned(),
                fix: "Wait for an eligible Group member to become ready or remove the pin."
                    .to_owned(),
            },
        ));
    }

    // 6. Product minor version check for resumed/handed-off runs (ADR 0030, 0055).
    let version_candidates: Vec<&MemberRecord> = if let Some(required) = &request.run_state_version
    {
        let required_mm = required.major_minor();
        ready_candidates
            .iter()
            .copied()
            .filter(|m| {
                let mm = m.capabilities.product_version.major_minor();
                mm.major == required_mm.major && mm.minor >= required_mm.minor
            })
            .collect()
    } else {
        ready_candidates
    };
    if version_candidates.is_empty()
        && let Some(required) = &request.run_state_version
    {
        return Err(format_refusal(
            &RUN_STATE,
            &required.to_string(),
            Compatibility::NewerMinor,
        ));
    }

    // 7. Required capabilities check (browsers, Plugins, local addresses,
    //    local model servers).
    let capable: Vec<&MemberRecord> = version_candidates
        .into_iter()
        .filter(|m| satisfies_capabilities(&m.capabilities, request))
        .collect();
    if capable.is_empty() {
        return Err(CallError::new(
            &CAPABILITY_UNAVAILABLE,
            "No eligible Group member provides all capabilities required by this Run (browsers, Plugins, local addresses, or local model servers).",
            &CapabilityUnavailableDetails {
                key: "group_member".to_owned(),
                layer: SettingsLayer::Run,
                kind: "group_member_capabilities".to_owned(),
                fix: "Check the capabilities configured on each Group member or adjust the Run's settings."
                    .to_owned(),
            },
        ));
    }

    // 8. Pick the eligible Group member with the most headroom (deterministic
    //    tie-breaker on member name).
    let chosen = capable
        .into_iter()
        .max_by(|left, right| {
            headroom_rank(&left.headroom)
                .cmp(&headroom_rank(&right.headroom))
                .then_with(|| right.name.cmp(&left.name))
        })
        .expect("invariant: capable is non-empty");

    Ok(chosen.route())
}

/// Applies steps 1..=4 of placement filtering: membership exclusion,
/// `client_browser`, machine-bound imports, machine-bound `session.source`,
/// and explicit `group_member` pin (ADRs 0055 and 0056).
fn filter_binding_and_pin<'a>(
    members: &'a [MemberRecord],
    request: &PlacementRequest,
) -> Result<Vec<&'a MemberRecord>, CallError> {
    if let Some(pin) = &request.group_member
        && !members.iter().any(|m| m.name == *pin)
    {
        return Err(CallError::new(
            &GROUP_MEMBER_UNKNOWN,
            format!("The Daemon group has no Group member named `{pin}`."),
            &(),
        ));
    }

    let mut candidates: Vec<&MemberRecord> = members
        .iter()
        .filter(|m| !request.exclude_members.iter().any(|ex| ex == &m.name))
        .collect();

    if request.client_browser {
        if let Some(pin) = &request.group_member
            && pin != &request.receiving_member
        {
            return Err(session_bound_error(
                &request.receiving_member,
                format!(
                    "`client_browser` runs only on the Group member holding the SDK's connection (`{}`), not `{pin}`.",
                    request.receiving_member
                ),
            ));
        }
        candidates.retain(|m| m.name == request.receiving_member);
        if candidates.is_empty() {
            return Err(session_bound_error(
                &request.receiving_member,
                format!(
                    "`client_browser` runs only on the Group member holding the SDK's connection (`{}`), which is unavailable.",
                    request.receiving_member
                ),
            ));
        }
    }

    if let Some(bound) = &request.machine_bound_member {
        if let Some(pin) = &request.group_member
            && pin != bound
        {
            return Err(session_bound_error(
                bound,
                format!(
                    "This Run's Session import is bound to Group member `{bound}` and cannot be pinned to `{pin}`."
                ),
            ));
        }
        candidates.retain(|m| m.name == *bound);
        if candidates.is_empty() {
            return Err(session_bound_error(
                bound,
                format!(
                    "This Run's Session import is bound to Group member `{bound}`, which is not available."
                ),
            ));
        }
    }

    if !request.browser_host_served
        && let Some(source) = &request.session_source
        && let Some(source_key) = machine_bound_source_key(source)
    {
        candidates.retain(|m| member_has_source(m, source, &source_key));
        if candidates.is_empty() {
            return Err(session_bound_error(
                &source_key,
                format!("No Group member hosts the machine-bound Session source `{source_key}`."),
            ));
        }
    }

    if let Some(pin) = &request.group_member {
        candidates.retain(|m| m.name == *pin);
        if candidates.is_empty() {
            return Err(session_bound_error(
                pin,
                format!(
                    "Pinned Group member `{pin}` cannot host this Run's machine-bound Session source."
                ),
            ));
        }
    }

    Ok(candidates)
}

fn member_has_source(member: &MemberRecord, source: &SessionSource, source_key: &str) -> bool {
    if member
        .capabilities
        .session_sources
        .iter()
        .any(|s| s == source_key)
    {
        return true;
    }
    match source {
        SessionSource::InstalledBrowser { browser, .. }
        | SessionSource::RunningBrowser { browser, .. } => {
            let name = browser_name(*browser);
            member
                .capabilities
                .browsers
                .iter()
                .any(|b| b.eq_ignore_ascii_case(name))
        }
        _ => false,
    }
}

fn satisfies_capabilities(caps: &MemberCapabilities, request: &PlacementRequest) -> bool {
    let has_all = |have: &[String], need: &[String]| {
        need.iter()
            .all(|req| have.iter().any(|item| item.eq_ignore_ascii_case(req)))
    };
    has_all(&caps.browsers, &request.required_browsers)
        && has_all(&caps.plugins, &request.required_plugins)
        && has_all(&caps.local_addresses, &request.required_local_addresses)
        && has_all(
            &caps.local_model_servers,
            &request.required_local_model_servers,
        )
}

fn headroom_rank(headroom: &MemberHeadroom) -> (u8, u64, u32) {
    let level_score = match headroom.level {
        GovernorLevel::Normal => 2,
        GovernorLevel::Pressure => 1,
        GovernorLevel::Critical => 0,
    };
    let total = u64::from(headroom.browser_available)
        + u64::from(headroom.http_available)
        + u64::from(headroom.model_available);
    (level_score, total, headroom.browser_available)
}

fn session_bound_error(_machine: &str, message: String) -> CallError {
    CallError::new(
        &SESSION_BOUND_TO_MACHINE,
        message,
        &SessionBoundToMachineDetails {
            run_id: RunId::parse("unassigned").expect("invariant: valid fallback RunId"),
        },
    )
}

#[cfg(test)]
mod tests;
