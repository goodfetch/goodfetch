//! Unit tests for Daemon group placement and headroom change detection
//! (ADRs 0040, 0055 and 0056, issue 244).

use goodfetch_protocol::checkpoint::SESSION_BOUND_TO_MACHINE;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::governor::{
    AllowanceSource, Concurrency, CpuReading, GovernorLevel, KindConcurrency, MemoryReading,
    SystemInfo,
};
use goodfetch_protocol::group::{
    GROUP_MEMBER_UNKNOWN, MemberCapabilities, MemberHeadroom, PlacementRequest,
};
use goodfetch_protocol::settings::CAPABILITY_UNAVAILABLE;
use goodfetch_protocol::settings::session_source::{BrowserName, SessionSource};

use super::{
    MemberRecord, headroom_changed_noticeably, headroom_from_system_info, is_loopback_cdp_url,
    machine_bound_source_key, select_member,
};

fn version(minor: u64) -> ProductVersion {
    ProductVersion::parse(&format!("1.{minor}.0")).unwrap()
}

fn member(name: &str, minor: u64, browser_avail: u32, http_avail: u32) -> MemberRecord {
    MemberRecord {
        name: name.to_owned(),
        peer_address: format!("127.0.0.1:700{}", name.len()),
        headroom: MemberHeadroom {
            http_available: http_avail,
            browser_available: browser_avail,
            model_available: 4,
            level: GovernorLevel::Normal,
            ready: true,
        },
        capabilities: MemberCapabilities {
            browsers: vec!["chromium".to_owned()],
            plugins: vec!["solver".to_owned()],
            local_addresses: vec!["203.0.113.10".to_owned()],
            local_model_servers: vec!["ollama".to_owned()],
            session_sources: Vec::new(),
            advertise_url: Some(format!("https://{name}.example.test")),
            product_version: version(minor),
        },
    }
}

/// Group members push headroom only when it changes noticeably (level/ready/
/// capabilities change, zero-crossing, or >=2 slots and >=10%).
///
/// Covers: story:3-109, story:3-110
#[test]
fn headroom_push_fires_on_noticeable_changes_only() {
    let base_caps = MemberCapabilities {
        browsers: vec!["chromium".to_owned()],
        product_version: version(2),
        ..MemberCapabilities::default()
    };
    let base_headroom = MemberHeadroom {
        http_available: 20,
        browser_available: 10,
        model_available: 10,
        level: GovernorLevel::Normal,
        ready: true,
    };
    let base = (base_headroom, base_caps.clone());

    assert!(headroom_changed_noticeably(None, &base));

    // Tiny 1-slot fluctuation is not noticeable.
    let minor_jitter = (
        MemberHeadroom {
            http_available: 19,
            ..base_headroom
        },
        base_caps.clone(),
    );
    assert!(!headroom_changed_noticeably(Some(&base), &minor_jitter));

    // >=2 slots AND >=10% drop is noticeable.
    let noticeable_drop = (
        MemberHeadroom {
            browser_available: 8,
            ..base_headroom
        },
        base_caps.clone(),
    );
    assert!(headroom_changed_noticeably(Some(&base), &noticeable_drop));

    // Dropping to zero is always noticeable.
    let hit_zero = (
        MemberHeadroom {
            model_available: 0,
            ..base_headroom
        },
        base_caps.clone(),
    );
    assert!(headroom_changed_noticeably(Some(&base), &hit_zero));

    // Capability change (e.g. advertise_url or local_model_servers) is noticeable.
    let mut caps_changed = base_caps;
    caps_changed.local_model_servers.push("vllm".to_owned());
    assert!(headroom_changed_noticeably(
        Some(&base),
        &(base_headroom, caps_changed)
    ));
}

/// `headroom_from_system_info` computes available slots (`target - active`).
///
/// Covers: story:3-109
#[test]
fn headroom_from_system_info_subtracts_active_from_target() {
    let info = SystemInfo {
        level: GovernorLevel::Pressure,
        pressure_source: None,
        last_resort: false,
        memory: MemoryReading {
            allowance: 1_000_000,
            allowance_source: AllowanceSource::Physical,
            used: 200_000,
            browsers: 100_000,
            host: GovernorLevel::Normal,
        },
        cpu: CpuReading {
            parallelism: 8,
            busy: 25,
            tokio_busy: 10,
        },
        concurrency: Concurrency {
            http: KindConcurrency {
                floor: 1,
                ceiling: 40,
                target: 20,
                in_use: 6,
            },
            browser_page: KindConcurrency {
                floor: 1,
                ceiling: 12,
                target: 8,
                in_use: 3,
            },
            model_call: KindConcurrency {
                floor: 1,
                ceiling: 10,
                target: 4,
                in_use: 5,
            },
        },
    };
    let h = headroom_from_system_info(Some(&info), true);
    assert_eq!(h.http_available, 14);
    assert_eq!(h.browser_available, 5);
    assert_eq!(h.model_available, 0);
    assert_eq!(h.level, GovernorLevel::Pressure);
    assert!(h.ready);
}

/// Lead Daemon picks the eligible Group member with the most headroom whose
/// minor is at least that of the run state, and honours a `group_member` pin.
///
/// Covers: story:3-109, story:3-110
#[test]
fn placement_prefers_highest_headroom_enforces_minor_and_honours_pin() {
    let m1 = member("member-a", 1, 20, 40); // highest headroom, older minor 1.1
    let m2 = member("member-b", 2, 12, 20); // newer minor 1.2, more headroom than m3
    let m3 = member("member-c", 3, 4, 10); // newest minor 1.3, least headroom
    let members = vec![m1, m2, m3];

    // New run with no minor constraint picks member-a (highest headroom).
    let chosen = select_member(&members, &PlacementRequest::default()).unwrap();
    assert_eq!(chosen.member, "member-a");

    // Resumed run with run state version 1.2 excludes member-a (1.1) and picks member-b (1.2).
    let resumed = PlacementRequest {
        run_state_version: Some(version(2)),
        ..PlacementRequest::default()
    };
    let chosen = select_member(&members, &resumed).unwrap();
    assert_eq!(chosen.member, "member-b");

    // Explicit `group_member` pin picks member-c despite lower headroom.
    let pinned = PlacementRequest {
        group_member: Some("member-c".to_owned()),
        run_state_version: Some(version(2)),
        ..PlacementRequest::default()
    };
    let chosen = select_member(&members, &pinned).unwrap();
    assert_eq!(chosen.member, "member-c");

    // Pinning an unknown member fails with `group_member_unknown`.
    let unknown = PlacementRequest {
        group_member: Some("ghost".to_owned()),
        ..PlacementRequest::default()
    };
    assert_eq!(
        select_member(&members, &unknown).unwrap_err().code(),
        GROUP_MEMBER_UNKNOWN.name
    );

    // Pinning a member whose minor is older than run state fails with `format_version_unsupported`.
    let too_old = PlacementRequest {
        group_member: Some("member-a".to_owned()),
        run_state_version: Some(version(2)),
        ..PlacementRequest::default()
    };
    assert_eq!(
        select_member(&members, &too_old).unwrap_err().code(),
        "format_version_unsupported"
    );
}

/// Capabilities filter (browsers, Plugins, local addresses, local model servers).
///
/// Covers: story:3-109, story:3-110
#[test]
fn placement_filters_by_capabilities() {
    let mut m1 = member("member-a", 2, 30, 50);
    m1.capabilities.plugins = vec![];
    let mut m2 = member("member-b", 2, 5, 10);
    m2.capabilities.browsers = vec!["firefox".to_owned()];
    m2.capabilities.plugins = vec!["captcha".to_owned()];
    m2.capabilities.local_addresses = vec!["198.51.100.7".to_owned()];
    m2.capabilities.local_model_servers = vec!["ollama".to_owned()];

    let req = PlacementRequest {
        required_browsers: vec!["firefox".to_owned()],
        required_plugins: vec!["captcha".to_owned()],
        required_local_addresses: vec!["198.51.100.7".to_owned()],
        required_local_model_servers: vec!["ollama".to_owned()],
        ..PlacementRequest::default()
    };
    let chosen = select_member(&[m1.clone(), m2.clone()], &req).unwrap();
    assert_eq!(chosen.member, "member-b");

    let impossible = PlacementRequest {
        required_local_model_servers: vec!["nonexistent".to_owned()],
        ..PlacementRequest::default()
    };
    assert_eq!(
        select_member(&[m1, m2], &impossible).unwrap_err().code(),
        CAPABILITY_UNAVAILABLE.name
    );
}

/// ADR 0056 placement: machine-bound Session sources (`profile_directory`,
/// `installed_browser`, `running_browser`, loopback `cdp_url`), machine-bound
/// imports, and `client_browser` go only to their owning Group member; a
/// source served by a Browser host can be placed on any Group member.
///
/// Covers: story:3-109, story:3-110
#[test]
fn placement_enforces_adr_0056_machine_bound_and_client_browser_rules() {
    let profile = SessionSource::ProfileDirectory {
        path: "/home/alice/chrome-profile".to_owned(),
        browser: BrowserName::Chrome,
        chromium_profile: None,
        domains: vec!["example.com".to_owned()],
    };
    let profile_key = machine_bound_source_key(&profile).unwrap();

    let loopback_cdp = SessionSource::CdpUrl {
        url: "ws://127.0.0.1:9222/devtools/browser/abc".to_owned(),
        headers: None,
        domains: None,
        exclusive: None,
    };
    assert!(is_loopback_cdp_url(
        "ws://127.0.0.1:9222/devtools/browser/abc"
    ));
    assert!(is_loopback_cdp_url("http://localhost:9222"));
    assert!(is_loopback_cdp_url("ws://[::1]:9222"));
    assert!(!is_loopback_cdp_url("wss://cloud-browser.example.com/cdp"));
    let loopback_key = machine_bound_source_key(&loopback_cdp).unwrap();

    let m1 = member("member-a", 2, 40, 40); // highest headroom, no local sources
    let mut m2 = member("member-b", 2, 5, 5); // lower headroom, holds profile + loopback CDP
    m2.capabilities.session_sources = vec![profile_key, loopback_key];
    let members = vec![m1, m2];

    // Profile directory goes ONLY to member-b despite member-a having more headroom.
    let req_profile = PlacementRequest {
        receiving_member: "member-a".to_owned(),
        session_source: Some(profile.clone()),
        ..PlacementRequest::default()
    };
    assert_eq!(
        select_member(&members, &req_profile).unwrap().member,
        "member-b"
    );

    // Served by a Browser host, the same profile can be placed on member-a (most headroom).
    let req_browser_host = PlacementRequest {
        receiving_member: "member-a".to_owned(),
        session_source: Some(profile),
        browser_host_served: true,
        ..PlacementRequest::default()
    };
    assert_eq!(
        select_member(&members, &req_browser_host).unwrap().member,
        "member-a"
    );

    // Loopback cdp_url goes ONLY to member-b.
    let req_loopback = PlacementRequest {
        receiving_member: "member-a".to_owned(),
        session_source: Some(loopback_cdp),
        ..PlacementRequest::default()
    };
    assert_eq!(
        select_member(&members, &req_loopback).unwrap().member,
        "member-b"
    );

    // Machine-bound import goes ONLY to its bound Group member; conflicting pin fails.
    let req_import = PlacementRequest {
        receiving_member: "member-a".to_owned(),
        machine_bound_member: Some("member-b".to_owned()),
        ..PlacementRequest::default()
    };
    assert_eq!(
        select_member(&members, &req_import).unwrap().member,
        "member-b"
    );
    let req_import_conflict = PlacementRequest {
        receiving_member: "member-a".to_owned(),
        machine_bound_member: Some("member-b".to_owned()),
        group_member: Some("member-a".to_owned()),
        ..PlacementRequest::default()
    };
    assert_eq!(
        select_member(&members, &req_import_conflict)
            .unwrap_err()
            .code(),
        SESSION_BOUND_TO_MACHINE.name
    );

    // `client_browser` goes ONLY to `receiving_member` (member-b here, even though member-a has more headroom).
    let req_client_browser = PlacementRequest {
        receiving_member: "member-b".to_owned(),
        client_browser: true,
        ..PlacementRequest::default()
    };
    assert_eq!(
        select_member(&members, &req_client_browser).unwrap().member,
        "member-b"
    );
    let req_client_browser_conflict = PlacementRequest {
        receiving_member: "member-b".to_owned(),
        client_browser: true,
        group_member: Some("member-a".to_owned()),
        ..PlacementRequest::default()
    };
    assert_eq!(
        select_member(&members, &req_client_browser_conflict)
            .unwrap_err()
            .code(),
        SESSION_BOUND_TO_MACHINE.name
    );
}
