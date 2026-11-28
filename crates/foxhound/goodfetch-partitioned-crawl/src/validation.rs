//! Start-time validation and refusals for Partitioned crawls (ADRs 0054 and
//! 0056, story:3-120).
//!
//! Refuses:
//! - `partitions_need_group` when `partitions > 1` or `"auto"` outside a Daemon
//!   group (standalone Daemon or Attached engine);
//! - `session_bound_to_machine` for `profile_directory`, `installed_browser`,
//!   `running_browser`, loopback `cdp_url`, `client_browser`, a machine-bound
//!   import, or a run-only import (`state_file` / `cookie_header`) on direct
//!   egress (no proxy configured);
//! - `browser_borrowed` for any other Borrowed browser (non-loopback `cdp_url`);
//! - `partitions_exceed_group_members` when the requested count exceeds the
//!   eligible Group members that can run the crawl.

use goodfetch_engine::CallError;
use goodfetch_engine::group::placement::is_loopback_cdp_url;
use goodfetch_protocol::checkpoint::{SESSION_BOUND_TO_MACHINE, SessionBoundToMachineDetails};
use goodfetch_protocol::foxhound::crawl::{
    PARTITIONS_EXCEED_GROUP_MEMBERS, PARTITIONS_NEED_GROUP, PartitionsExceedGroupMembersDetails,
};
use goodfetch_protocol::group::MemberRoute;
use goodfetch_protocol::page::session::BROWSER_BORROWED;
use goodfetch_protocol::run::{Partitions, RunId};
use goodfetch_protocol::settings::session_source::SessionSource;

/// Inputs checked before placing a multi-Partition crawl (ADRs 0054 and 0056).
#[derive(Clone, Debug, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent ADR 0056 start refusal conditions evaluated together before placement"
)]
pub struct PartitionedStartCheck {
    /// Requested `partitions` parameter, if any.
    pub partitions: Option<Partitions>,
    /// Effective `session.source` setting (run, connection, or label layer), if any.
    pub session_source: Option<SessionSource>,
    /// Whether the crawl uses `client_browser` (`run.borrow()`).
    pub client_browser: bool,
    /// Whether the crawl's Session source is a machine-bound import.
    pub machine_bound_import: bool,
    /// Whether a proxy (`proxy.server` or `proxy.provider`) is configured
    /// (making `state_file` upload / `cookie_header` non-direct egress).
    pub has_proxy: bool,
    /// Whether the Session source is served via a person's Browser host.
    pub browser_host_served: bool,
}

/// Result of checking `partitions` and selecting distinct Group members.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidatedPartitions {
    /// Single-run crawl (`partitions` absent or `1`).
    Single,
    /// Multi-Partition crawl placed on these distinct Group members (length >= 1).
    Partitioned(Vec<MemberRoute>),
}

/// Validates a crawl's `partitions` request against group membership, ADR 0056
/// Session-source rules, and the eligible Group members list.
///
/// # Errors
///
/// Returns `partitions_need_group`, `session_bound_to_machine`,
/// `browser_borrowed`, or `partitions_exceed_group_members`.
pub fn validate_and_select(
    check: &PartitionedStartCheck,
    in_group: bool,
    eligible_members: Vec<MemberRoute>,
    run_id_hint: Option<&RunId>,
) -> Result<ValidatedPartitions, CallError> {
    let Some(partitions) = check.partitions else {
        return Ok(ValidatedPartitions::Single);
    };
    if !partitions.is_multi() {
        return Ok(ValidatedPartitions::Single);
    }
    if !in_group {
        return Err(CallError::new(
            &PARTITIONS_NEED_GROUP,
            "A Partitioned crawl with more than 1 Partition requires a Daemon group; use `partitions = 1` outside a group or join a Daemon group.",
            &(),
        ));
    }

    // ADR 0056: check Session source compatibility before placing Partitions.
    check_session_source_for_partitions(check, run_id_hint)?;

    let desired = match partitions {
        Partitions::Count(n) => n.get(),
        Partitions::Auto(_) => u32::try_from(eligible_members.len()).unwrap_or(u32::MAX),
    };

    if desired == 0 || (desired as usize) > eligible_members.len() {
        let requested = match partitions {
            Partitions::Count(n) => n.get(),
            Partitions::Auto(_) => 1,
        };
        let eligible = u32::try_from(eligible_members.len()).unwrap_or(u32::MAX);
        return Err(CallError::new(
            &PARTITIONS_EXCEED_GROUP_MEMBERS,
            format!(
                "Requested {requested} Partitions, but only {eligible} eligible Group member(s) can run this crawl."
            ),
            &PartitionsExceedGroupMembersDetails {
                requested,
                eligible,
            },
        ));
    }

    Ok(ValidatedPartitions::Partitioned(
        eligible_members
            .into_iter()
            .take(desired as usize)
            .collect(),
    ))
}

/// Enforces ADR 0054 / ADR 0056 Session-source refusals for crawls with more
/// than one Partition.
///
/// # Errors
///
/// Returns `session_bound_to_machine` or `browser_borrowed`.
pub fn check_session_source_for_partitions(
    check: &PartitionedStartCheck,
    run_id_hint: Option<&RunId>,
) -> Result<(), CallError> {
    let fallback_id = || {
        run_id_hint.cloned().unwrap_or_else(|| {
            RunId::parse("partitioned-crawl").expect("invariant: valid fallback RunId")
        })
    };

    if check.client_browser {
        return Err(CallError::new(
            &SESSION_BOUND_TO_MACHINE,
            "`client_browser` is bound to the machine holding the SDK's connection and cannot be used with more than 1 Partition.",
            &SessionBoundToMachineDetails {
                run_id: fallback_id(),
            },
        ));
    }

    if check.machine_bound_import {
        return Err(CallError::new(
            &SESSION_BOUND_TO_MACHINE,
            "A machine-bound Session import cannot be used across multiple Partitions on distinct Group members.",
            &SessionBoundToMachineDetails {
                run_id: fallback_id(),
            },
        ));
    }

    let Some(source) = &check.session_source else {
        return Ok(());
    };

    match source {
        SessionSource::ProfileDirectory { .. }
        | SessionSource::InstalledBrowser { .. }
        | SessionSource::RunningBrowser { .. } => Err(CallError::new(
            &SESSION_BOUND_TO_MACHINE,
            format!(
                "Session source `{}` is bound to one machine and cannot be used with more than 1 Partition.",
                source.kind()
            ),
            &SessionBoundToMachineDetails {
                run_id: fallback_id(),
            },
        )),
        SessionSource::CdpUrl { url, .. } if is_loopback_cdp_url(url) => Err(CallError::new(
            &SESSION_BOUND_TO_MACHINE,
            "A loopback `cdp_url` is bound to one machine and cannot be used with more than 1 Partition.",
            &SessionBoundToMachineDetails {
                run_id: fallback_id(),
            },
        )),
        SessionSource::CdpUrl { .. } => Err(CallError::new(
            &BROWSER_BORROWED,
            "A Borrowed browser (`cdp_url`) serves one Run at a time and cannot be shared across multiple Partitions.",
            &(),
        )),
        SessionSource::StateFile { .. } | SessionSource::CookieHeader { .. }
            if !check.has_proxy =>
        {
            Err(CallError::new(
                &SESSION_BOUND_TO_MACHINE,
                format!(
                    "A run-only `{}` import on direct egress is bound to one machine and cannot be used across multiple Partitions; configure a proxy or use `partitions = 1`.",
                    source.kind()
                ),
                &SessionBoundToMachineDetails {
                    run_id: fallback_id(),
                },
            ))
        }
        SessionSource::StateFile { .. }
        | SessionSource::CookieHeader { .. }
        | SessionSource::SavedIdentity { .. } => Ok(()),
    }
}

#[cfg(test)]
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
mod tests {
    use std::num::NonZeroU32;

    use goodfetch_protocol::group::MemberRoute;
    use goodfetch_protocol::run::{AutoPartitions, Partitions};
    use goodfetch_protocol::settings::session_source::{BrowserName, SessionSource};

    use super::{PartitionedStartCheck, ValidatedPartitions, validate_and_select};

    fn members(count: usize) -> Vec<MemberRoute> {
        (0..count)
            .map(|i| MemberRoute {
                member: format!("member-{i}"),
                peer_address: format!("127.0.0.1:{}", 7000 + i),
                advertise_url: None,
            })
            .collect()
    }

    /// Covers: story:3-120, error:partitions_need_group
    #[test]
    fn partitions_above_one_outside_group_refused_with_partitions_need_group() {
        let check = PartitionedStartCheck {
            partitions: Some(Partitions::Count(NonZeroU32::new(2).unwrap())),
            ..Default::default()
        };
        let err = validate_and_select(&check, false, vec![], None).unwrap_err();
        assert_eq!(err.code(), "partitions_need_group");

        let auto_check = PartitionedStartCheck {
            partitions: Some(Partitions::Auto(AutoPartitions::Auto)),
            ..Default::default()
        };
        let err = validate_and_select(&auto_check, false, vec![], None).unwrap_err();
        assert_eq!(err.code(), "partitions_need_group");

        // partitions = 1 is allowed outside a group.
        let single_check = PartitionedStartCheck {
            partitions: Some(Partitions::Count(NonZeroU32::new(1).unwrap())),
            ..Default::default()
        };
        assert_eq!(
            validate_and_select(&single_check, false, vec![], None).unwrap(),
            ValidatedPartitions::Single
        );
    }

    /// Covers: story:3-120, error:partitions_exceed_group_members
    #[test]
    fn partitions_exceeding_eligible_members_refused() {
        let check = PartitionedStartCheck {
            partitions: Some(Partitions::Count(NonZeroU32::new(4).unwrap())),
            ..Default::default()
        };
        let err = validate_and_select(&check, true, members(3), None).unwrap_err();
        assert_eq!(err.code(), "partitions_exceed_group_members");
    }

    /// Covers: story:3-116
    #[test]
    fn auto_and_count_select_distinct_group_members() {
        let auto_check = PartitionedStartCheck {
            partitions: Some(Partitions::Auto(AutoPartitions::Auto)),
            ..Default::default()
        };
        let ValidatedPartitions::Partitioned(chosen) =
            validate_and_select(&auto_check, true, members(3), None).unwrap()
        else {
            panic!("expected Partitioned");
        };
        assert_eq!(chosen.len(), 3);

        let count_check = PartitionedStartCheck {
            partitions: Some(Partitions::Count(NonZeroU32::new(2).unwrap())),
            ..Default::default()
        };
        let ValidatedPartitions::Partitioned(chosen) =
            validate_and_select(&count_check, true, members(3), None).unwrap()
        else {
            panic!("expected Partitioned");
        };
        assert_eq!(chosen.len(), 2);
        assert_ne!(chosen[0].member, chosen[1].member);
    }

    /// Covers: story:3-120, error:session_bound_to_machine, error:browser_borrowed
    #[test]
    fn adr_0056_session_source_refusals() {
        let multi = Some(Partitions::Count(NonZeroU32::new(2).unwrap()));

        // Profile directory, installed browser, running browser, loopback cdp_url -> session_bound_to_machine
        for source in [
            SessionSource::ProfileDirectory {
                path: "/tmp/p".to_owned(),
                browser: BrowserName::Chrome,
                chromium_profile: None,
                domains: vec!["example.com".to_owned()],
            },
            SessionSource::InstalledBrowser {
                browser: BrowserName::Chrome,
                chromium_profile: None,
                domains: vec!["example.com".to_owned()],
            },
            SessionSource::RunningBrowser {
                browser: BrowserName::Chrome,
                domains: vec!["example.com".to_owned()],
            },
            SessionSource::CdpUrl {
                url: "ws://127.0.0.1:9222/devtools/browser/abc".to_owned(),
                headers: None,
                domains: None,
                exclusive: None,
            },
            SessionSource::StateFile {
                path: "state.json".to_owned(),
                format: None,
                key: None,
                domains: None,
            },
            SessionSource::CookieHeader {
                header: "a=b".to_owned(),
                domain: "example.com".to_owned(),
            },
        ] {
            let check = PartitionedStartCheck {
                partitions: multi,
                session_source: Some(source),
                has_proxy: false,
                ..Default::default()
            };
            let err = validate_and_select(&check, true, members(3), None).unwrap_err();
            assert_eq!(err.code(), "session_bound_to_machine");
        }

        // client_browser & machine_bound_import -> session_bound_to_machine
        let err = validate_and_select(
            &PartitionedStartCheck {
                partitions: multi,
                client_browser: true,
                ..Default::default()
            },
            true,
            members(3),
            None,
        )
        .unwrap_err();
        assert_eq!(err.code(), "session_bound_to_machine");

        let err = validate_and_select(
            &PartitionedStartCheck {
                partitions: multi,
                machine_bound_import: true,
                ..Default::default()
            },
            true,
            members(3),
            None,
        )
        .unwrap_err();
        assert_eq!(err.code(), "session_bound_to_machine");

        // Non-loopback cdp_url -> browser_borrowed
        let err = validate_and_select(
            &PartitionedStartCheck {
                partitions: multi,
                session_source: Some(SessionSource::CdpUrl {
                    url: "wss://remote-browser.example.com/devtools/browser/abc".to_owned(),
                    headers: None,
                    domains: None,
                    exclusive: Some(true),
                }),
                ..Default::default()
            },
            true,
            members(3),
            None,
        )
        .unwrap_err();
        assert_eq!(err.code(), "browser_borrowed");

        // Run-only import WITH proxy -> allowed
        let ok = validate_and_select(
            &PartitionedStartCheck {
                partitions: multi,
                session_source: Some(SessionSource::CookieHeader {
                    header: "a=b".to_owned(),
                    domain: "example.com".to_owned(),
                }),
                has_proxy: true,
                ..Default::default()
            },
            true,
            members(3),
            None,
        )
        .unwrap();
        assert!(matches!(ok, ValidatedPartitions::Partitioned(m) if m.len() == 2));
    }
}
