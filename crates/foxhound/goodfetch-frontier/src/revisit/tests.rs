//! Table-driven and property tests for the pure revisit schedule function and
//! the `revisits` table's covering-index queries (ADR 0139).

use goodfetch_protocol::foxhound::continuous::RevisitPolicy;
use proptest::prelude::*;
use rstest::rstest;
use rusqlite::Connection;

use super::{
    DueEntry, PreviousState, RevisitRow, SiteHintSwitches, SiteHints, TABLES, VisitResult,
    due_window, earliest_due, get, next_visit, read_all, upsert,
};

/// Table-driven tests of `next_visit` for `never` and `every` policies with
/// and without site floors.
#[rstest]
#[case::never_returns_none(RevisitPolicy::Never, 1_000_000, None, None)]
#[case::never_stays_never_with_floor(RevisitPolicy::Never, 1_000_000, Some(2_000_000), None)]
#[case::every_adds_fixed_interval(
    RevisitPolicy::Every { interval: 60_000 },
    1_000_000,
    None,
    Some((60_000, 1_060_000)),
)]
#[case::every_respects_higher_site_floor(
    RevisitPolicy::Every { interval: 60_000 },
    1_000_000,
    Some(1_500_000),
    Some((60_000, 1_500_000)),
)]
#[case::every_keeps_policy_when_floor_is_earlier(
    RevisitPolicy::Every { interval: 60_000 },
    1_000_000,
    Some(1_010_000),
    Some((60_000, 1_060_000)),
)]
fn next_visit_matches_table(
    #[case] policy: RevisitPolicy,
    #[case] now_ms: u64,
    #[case] floor_at_ms: Option<u64>,
    #[case] expected: Option<(u64, u64)>,
) {
    let scheduled = next_visit(
        &policy,
        PreviousState::default(),
        VisitResult::Changed,
        SiteHints {
            floor_at_ms,
            ..SiteHints::default()
        },
        now_ms,
    );
    assert_eq!(
        scheduled.map(|s| (s.interval_ms, s.next_visit_at)),
        expected
    );
}

proptest! {
    #[test]
    fn never_policy_always_returns_none(
        now_ms in 0_u64..10_000_000_000_000,
        floor in proptest::option::of(0_u64..10_000_000_000_000),
        visits in 0_u64..1000,
        changes in 0_u64..1000,
    ) {
        let scheduled = next_visit(
            &RevisitPolicy::Never,
            PreviousState { interval_ms: Some(60_000), visits, changes },
            VisitResult::Changed,
            SiteHints {
                floor_at_ms: floor,
                ..SiteHints::default()
            },
            now_ms,
        );
        prop_assert_eq!(scheduled, None);
    }

    #[test]
    fn every_policy_never_schedules_before_interval_or_floor(
        interval in 1_u64..2_592_000_000,
        now_ms in 0_u64..10_000_000_000_000,
        floor in proptest::option::of(0_u64..10_000_000_000_000),
    ) {
        let scheduled = next_visit(
            &RevisitPolicy::Every { interval },
            PreviousState::default(),
            VisitResult::Changed,
            SiteHints {
                floor_at_ms: floor,
                ..SiteHints::default()
            },
            now_ms,
        ).expect("every policy always schedules");

        prop_assert_eq!(scheduled.interval_ms, interval);
        prop_assert!(scheduled.next_visit_at >= now_ms.saturating_add(interval));
        if let Some(floor_ms) = floor {
            prop_assert!(scheduled.next_visit_at >= floor_ms);
        }
    }
}

#[test]
fn revisits_table_due_window_uses_covering_index_and_bounds_results() {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection
            .prepare_cached(statement)
            .unwrap()
            .execute([])
            .unwrap();
    }

    // Verify EXPLAIN QUERY PLAN uses the covering index `revisits_due`.
    let plan: String = connection
        .prepare_cached(
            "EXPLAIN QUERY PLAN
             SELECT work_item, domain_key, next_visit_at
             FROM revisits
             WHERE domain_key = 'news.test' AND next_visit_at IS NOT NULL AND next_visit_at <= 5000
             ORDER BY next_visit_at ASC, work_item ASC
             LIMIT 64",
        )
        .unwrap()
        .query_row([], |row| row.get(3))
        .unwrap();
    assert!(
        plan.contains("USING COVERING INDEX revisits_due"),
        "expected covering index scan, got: {plan}"
    );

    for (id, domain, due) in [
        ("1", "news.test", Some(2000)),
        ("2", "news.test", Some(1000)),
        ("3", "news.test", Some(3000)),
        ("4", "news.test", None),
        ("5", "other.test", Some(500)),
    ] {
        upsert(
            &connection,
            &RevisitRow {
                work_item: id.to_owned(),
                identity: format!("https://{domain}/{id}"),
                domain_key: domain.to_owned(),
                next_visit_at: due,
                interval_ms: due.map(|_| 1000),
                last_fetched_at: Some(0),
                last_changed_at: Some(0),
                visits: 1,
                changes: 1,
                ..RevisitRow::default()
            },
        )
        .unwrap();
    }

    // Bounded window of 2 for `news.test` due at 5000 returns IDs 2 and 1 in due order.
    let window = due_window(&connection, Some("news.test"), 5000, 2).unwrap();
    assert_eq!(
        window,
        vec![
            DueEntry {
                work_item: "2".to_owned(),
                domain_key: "news.test".to_owned(),
                next_visit_at: 1000,
            },
            DueEntry {
                work_item: "1".to_owned(),
                domain_key: "news.test".to_owned(),
                next_visit_at: 2000,
            },
        ]
    );
    assert_eq!(earliest_due(&connection).unwrap(), Some(500));
    assert_eq!(read_all(&connection).unwrap().len(), 5);
    assert_eq!(get(&connection, "2").unwrap().unwrap().visits, 1);
}

#[test]
fn adaptive_policy_defaults_shrinks_grows_and_clamps_at_min_and_max() {
    let default = RevisitPolicy::default_adaptive();
    assert_eq!(
        default,
        RevisitPolicy::Adaptive {
            initial: 86_400_000,
            min: 3_600_000,
            max: 2_592_000_000,
            rate_changed: 0.5,
            rate_unchanged: 0.5,
        }
    );

    let policy = RevisitPolicy::Adaptive {
        initial: 10_000,
        min: 3_000,
        max: 20_000,
        rate_changed: 0.5,
        rate_unchanged: 0.5,
    };

    // First visit uses `initial`.
    let first = next_visit(
        &policy,
        PreviousState::default(),
        VisitResult::Changed,
        SiteHints::default(),
        100_000,
    )
    .unwrap();
    assert_eq!(first.interval_ms, 10_000);
    assert_eq!(first.next_visit_at, 110_000);
    assert!(!first.clamped_at_min);

    // Changed revisit shrinks by 0.5 -> 5_000.
    let changed = next_visit(
        &policy,
        PreviousState {
            interval_ms: Some(10_000),
            visits: 1,
            changes: 1,
        },
        VisitResult::Changed,
        SiteHints::default(),
        110_000,
    )
    .unwrap();
    assert_eq!(changed.interval_ms, 5_000);
    assert!(!changed.clamped_at_min);

    // Another changed revisit shrinks 5_000 * 0.5 = 2_500 < min (3_000) -> clamped at min!
    let fast = next_visit(
        &policy,
        PreviousState {
            interval_ms: Some(5_000),
            visits: 2,
            changes: 2,
        },
        VisitResult::Changed,
        SiteHints::default(),
        115_000,
    )
    .unwrap();
    assert_eq!(fast.interval_ms, 3_000);
    assert!(fast.clamped_at_min);

    // Unchanged revisit grows 15_000 * 1.5 = 22_500 -> clamped at max (20_000).
    let slow = next_visit(
        &policy,
        PreviousState {
            interval_ms: Some(15_000),
            visits: 2,
            changes: 1,
        },
        VisitResult::Unchanged,
        SiteHints::default(),
        130_000,
    )
    .unwrap();
    assert_eq!(slow.interval_ms, 20_000);
    assert!(!slow.clamped_at_min);
}

#[test]
fn estimated_policy_uses_cho_garcia_molina_prior_and_clamps_fast_urls() {
    let policy = RevisitPolicy::Estimated {
        initial: 100_000,
        min: 10_000,
        max: 1_000_000,
        target_freshness: 0.9,
    };

    // First visit uses `initial`.
    let first = next_visit(
        &policy,
        PreviousState::default(),
        VisitResult::Changed,
        SiteHints::default(),
        1_000,
    )
    .unwrap();
    assert_eq!(first.interval_ms, 100_000);
    assert!(!first.clamped_at_min);

    // Verify Cho & Garcia-Molina formula with prior of 2 intervals of `initial`:
    // After 1 revisit interval unchanged: n_obs = 1, x_obs = 0 -> n = 3, X = 1.
    // r_hat = -ln((3 - 1 + 0.5)/(3 + 0.5)) = -ln(2.5/3.5) = 0.3364722...
    // I_ref = (2*100_000 + 1*100_000)/3 = 100_000.
    // u for target_freshness = 0.9 solves (1 - e^{-u})/u = 0.9 => u ≈ 0.2145551.
    // T = u * I_ref / r_hat ≈ 63_766 ms.
    let one_unchanged = next_visit(
        &policy,
        PreviousState {
            interval_ms: Some(100_000),
            visits: 1,
            changes: 1,
        },
        VisitResult::Unchanged,
        SiteHints::default(),
        101_000,
    )
    .unwrap();
    assert!(
        one_unchanged.interval_ms.abs_diff(63_766) <= 2,
        "expected ~63_766 ms from Cho & Garcia-Molina estimator, got {}",
        one_unchanged.interval_ms
    );
    assert!(!one_unchanged.clamped_at_min);

    // Non-proportionality check: comparing 10% vs 90% observed change ratio
    // over 20 revisits at the same interval does NOT yield a 9x ratio of intervals
    // (because r̂ = -ln((n-X+0.5)/(n+0.5)) + prior is nonlinear in X/n).
    let low_change = next_visit(
        &policy,
        PreviousState {
            interval_ms: Some(100_000),
            visits: 20,
            changes: 3, // 2 revisit changes
        },
        VisitResult::Unchanged,
        SiteHints::default(),
        500_000,
    )
    .unwrap();
    let high_change = next_visit(
        &policy,
        PreviousState {
            interval_ms: Some(100_000),
            visits: 20,
            changes: 18, // 18 revisit changes after this Changed
        },
        VisitResult::Changed,
        SiteHints::default(),
        500_000,
    )
    .unwrap();
    #[expect(
        clippy::cast_precision_loss,
        reason = "small millisecond intervals fit exactly in f64"
    )]
    let ratio = low_change.interval_ms as f64 / high_change.interval_ms as f64;
    assert!(
        (ratio - 9.0).abs() > 1.0,
        "expected non-proportional ratio (not ~9.0), got {ratio} ({} vs {})",
        low_change.interval_ms,
        high_change.interval_ms
    );

    // A URL changing on every revisit at `min` wants < `min`, gets `min`, and sets `clamped_at_min`.
    let fast = next_visit(
        &policy,
        PreviousState {
            interval_ms: Some(10_000),
            visits: 10,
            changes: 10,
        },
        VisitResult::Changed,
        SiteHints::default(),
        500_000,
    )
    .unwrap();
    assert_eq!(fast.interval_ms, 10_000);
    assert!(fast.clamped_at_min);
}

#[test]
fn revisit_policy_validation_rejects_invalid_bounds_and_rates() {
    use serde_json::json;

    // Valid defaults and explicit forms.
    assert!(RevisitPolicy::read(&json!({ "kind": "adaptive" })).is_ok());
    assert!(RevisitPolicy::read(&json!({ "kind": "estimated" })).is_ok());
    assert!(
        RevisitPolicy::read(&json!({
            "kind": "adaptive",
            "initial": 1000,
            "min": 1000,
            "max": 1000,
            "rate_changed": 1.0,
            "rate_unchanged": 1.0,
        }))
        .is_ok()
    );

    // min > max
    assert!(
        RevisitPolicy::read(&json!({
            "kind": "adaptive",
            "min": 5000,
            "max": 4000,
        }))
        .is_err()
    );
    assert!(
        RevisitPolicy::read(&json!({
            "kind": "estimated",
            "min": 5000,
            "max": 4000,
        }))
        .is_err()
    );

    // rate outside (0, 1]
    for bad_rate in [0.0, -0.1, 1.01] {
        assert!(
            RevisitPolicy::read(&json!({
                "kind": "adaptive",
                "rate_changed": bad_rate,
            }))
            .is_err(),
            "rate_changed={bad_rate} must fail"
        );
        assert!(
            RevisitPolicy::read(&json!({
                "kind": "adaptive",
                "rate_unchanged": bad_rate,
            }))
            .is_err(),
            "rate_unchanged={bad_rate} must fail"
        );
    }

    // target_freshness outside (0, 1)
    for bad_target in [0.0, 1.0, -0.5, 1.5] {
        assert!(
            RevisitPolicy::read(&json!({
                "kind": "estimated",
                "target_freshness": bad_target,
            }))
            .is_err(),
            "target_freshness={bad_target} must fail"
        );
    }
}

proptest! {
    #[test]
    fn adaptive_policy_always_stays_within_min_and_max(
        min in 1_u64..1_000_000,
        span in 0_u64..10_000_000,
        initial in 1_u64..20_000_000,
        rate_changed in 0.001_f64..=1.0,
        rate_unchanged in 0.001_f64..=1.0,
        prev_interval in proptest::option::of(1_u64..20_000_000),
        visits in 0_u64..1000,
        changes in 0_u64..1000,
        changed in any::<bool>(),
        now_ms in 0_u64..10_000_000_000_000,
        floor in proptest::option::of(0_u64..10_000_000_000_000),
    ) {
        let max = min.saturating_add(span);
        let policy = RevisitPolicy::Adaptive {
            initial,
            min,
            max,
            rate_changed,
            rate_unchanged,
        };
        let result = if changed { VisitResult::Changed } else { VisitResult::Unchanged };
        let scheduled = next_visit(
            &policy,
            PreviousState { interval_ms: prev_interval, visits, changes },
            result,
            SiteHints {
                floor_at_ms: floor,
                ..SiteHints::default()
            },
            now_ms,
        ).expect("adaptive policy always schedules");

        prop_assert!(scheduled.interval_ms >= min);
        prop_assert!(scheduled.interval_ms <= max);
        prop_assert!(scheduled.next_visit_at >= now_ms.saturating_add(min));
        if let Some(floor_ms) = floor {
            prop_assert!(scheduled.next_visit_at >= floor_ms);
        }
    }

    #[test]
    fn estimated_policy_always_stays_within_min_and_max(
        min in 1_u64..1_000_000,
        span in 0_u64..10_000_000,
        initial in 1_u64..20_000_000,
        target_freshness in 0.001_f64..0.999,
        prev_interval in proptest::option::of(1_u64..20_000_000),
        visits in 0_u64..1000,
        changes in 0_u64..1000,
        changed in any::<bool>(),
        now_ms in 0_u64..10_000_000_000_000,
        floor in proptest::option::of(0_u64..10_000_000_000_000),
    ) {
        let max = min.saturating_add(span);
        let policy = RevisitPolicy::Estimated {
            initial,
            min,
            max,
            target_freshness,
        };
        let result = if changed { VisitResult::Changed } else { VisitResult::Unchanged };
        let scheduled = next_visit(
            &policy,
            PreviousState { interval_ms: prev_interval, visits, changes },
            result,
            SiteHints {
                floor_at_ms: floor,
                ..SiteHints::default()
            },
            now_ms,
        ).expect("estimated policy always schedules");

        prop_assert!(scheduled.interval_ms >= min);
        prop_assert!(scheduled.interval_ms <= max);
        prop_assert!(scheduled.next_visit_at >= now_ms.saturating_add(min));
        if let Some(floor_ms) = floor {
            prop_assert!(scheduled.next_visit_at >= floor_ms);
        }
    }

    #[test]
    fn site_hint_floors_win_over_every_policy_when_on(
        policy_kind in 0_u8..3,
        max_age_secs in 1_u64..86_400,
        retry_after_secs in 1_u64..86_400,
        expires_delta_secs in 1_u64..86_400,
        use_cc in any::<bool>(),
        use_exp in any::<bool>(),
        use_ra in any::<bool>(),
        now_secs in 1_700_000_000_u64..1_800_000_000,
    ) {
        let now_ms = now_secs * 1_000;
        let cc_header = format!("public, max-age={max_age_secs}");
        let ra_header = retry_after_secs.to_string();
        let date_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(now_secs);
        let exp_time = std::time::UNIX_EPOCH
            + std::time::Duration::from_secs(now_secs + expires_delta_secs);
        // Format HTTP-date via RFC 9110 IMF-fixdate using a simple UTC helper.
        let date_str = format_http_date(date_time);
        let exp_str = format_http_date(exp_time);
        let switches = SiteHintSwitches {
            cache_control: use_cc,
            expires: use_exp,
            retry_after: use_ra,
        };
        let hints = SiteHints::from_headers(
            Some(&cc_header),
            Some(&exp_str),
            Some(&ra_header),
            Some(&date_str),
            switches,
            now_ms,
        );
        let policy = match policy_kind {
            0 => RevisitPolicy::Every { interval: 1_000 },
            1 => RevisitPolicy::Adaptive {
                initial: 1_000,
                min: 1_000,
                max: 10_000,
                rate_changed: 0.5,
                rate_unchanged: 0.5,
            },
            _ => RevisitPolicy::Estimated {
                initial: 1_000,
                min: 1_000,
                max: 10_000,
                target_freshness: 0.9,
            },
        };
        let scheduled = next_visit(
            &policy,
            PreviousState::default(),
            VisitResult::Changed,
            hints,
            now_ms,
        )
        .expect("non-never policy schedules");

        if use_cc {
            prop_assert!(scheduled.next_visit_at >= now_ms + max_age_secs * 1_000);
        }
        if use_exp {
            prop_assert!(scheduled.next_visit_at >= now_ms + expires_delta_secs * 1_000);
        }
        if use_ra {
            prop_assert!(scheduled.next_visit_at >= now_ms + retry_after_secs * 1_000);
        }
        if !use_cc && !use_exp && !use_ra {
            prop_assert_eq!(hints.floor_at_ms, None);
            prop_assert_eq!(scheduled.next_visit_at, now_ms + 1_000);
        }
    }
}

/// Formats a `SystemTime` (seconds precision) as an RFC 9110 IMF-fixdate string.
fn format_http_date(time: std::time::SystemTime) -> String {
    let secs = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let hour = rem / 3_600;
    let min = (rem % 3_600) / 60;
    let sec = rem % 60;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    let dow = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"]
        [usize::try_from(days % 7).expect("0..7 fits in usize")];
    let mon = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][usize::try_from(month - 1).expect("0..12 fits in usize")];
    format!("{dow}, {day:02} {mon} {year:04} {hour:02}:{min:02}:{sec:02} GMT")
}

#[test]
fn site_hints_from_headers_parses_each_header_and_respects_switches() {
    let now_ms = 1_767_603_600_000_u64; // 2026-01-05T09:00:00Z
    let date = "Mon, 05 Jan 2026 09:00:00 GMT";
    let expires = "Mon, 05 Jan 2026 09:10:00 GMT"; // +600s

    // All on: takes the largest floor among max-age (300s), Expires (600s), Retry-After (120s).
    let all = SiteHints::from_headers(
        Some("public, max-age=300"),
        Some(expires),
        Some("120"),
        Some(date),
        SiteHintSwitches::default(),
        now_ms,
    );
    assert_eq!(all.floor_at_ms, Some(now_ms + 600_000));

    // Switching Expires off drops the floor to Cache-Control max-age (300s).
    let no_exp = SiteHints::from_headers(
        Some("public, max-age=300"),
        Some(expires),
        Some("120"),
        Some(date),
        SiteHintSwitches {
            expires: false,
            ..SiteHintSwitches::default()
        },
        now_ms,
    );
    assert_eq!(no_exp.floor_at_ms, Some(now_ms + 300_000));

    // Switching Cache-Control off too leaves Retry-After (120s).
    let only_ra = SiteHints::from_headers(
        Some("public, max-age=300"),
        Some(expires),
        Some("120"),
        Some(date),
        SiteHintSwitches {
            cache_control: false,
            expires: false,
            retry_after: true,
        },
        now_ms,
    );
    assert_eq!(only_ra.floor_at_ms, Some(now_ms + 120_000));

    // Numeric `Expires: 0` is already expired, never a delta-seconds floor.
    let exp_zero = SiteHints::from_headers(
        None,
        Some("0"),
        None,
        Some(date),
        SiteHintSwitches::default(),
        now_ms,
    );
    assert_eq!(exp_zero.floor_at_ms, None);
}

#[test]
fn freshness_aggregates_overdue_lags_outcomes_overall_and_per_domain() {
    use std::time::Duration;
    let conn = Connection::open_in_memory().unwrap();
    for stmt in TABLES.statements {
        conn.execute_batch(stmt).unwrap();
    }
    conn.execute_batch(
        "CREATE TABLE work_items (id TEXT NOT NULL PRIMARY KEY, status TEXT NOT NULL) STRICT;",
    )
    .unwrap();

    let now_ms = 1_700_000_000_000_u64;
    let insert = |id: &str,
                  domain: &str,
                  next_visit_at: Option<u64>,
                  visits: u64,
                  changes: u64,
                  status: &str| {
        conn.prepare_cached("INSERT INTO work_items (id, status) VALUES (?1, ?2)")
            .unwrap()
            .execute(rusqlite::params![id, status])
            .unwrap();
        upsert(
            &conn,
            &RevisitRow {
                work_item: id.to_owned(),
                identity: format!("https://{domain}/{id}"),
                domain_key: domain.to_owned(),
                next_visit_at,
                visits,
                changes,
                ..RevisitRow::default()
            },
        )
        .unwrap();
    };

    // a.example: 2 overdue (lag 1000ms, 3000ms => median 2000ms, max 3000ms),
    // 1 not overdue, 1 gone.
    insert("a1", "a.example", Some(now_ms - 1_000), 3, 1, "unchanged"); // 2 unchanged, 0 changed
    insert("a2", "a.example", Some(now_ms - 3_000), 4, 3, "succeeded"); // 1 unchanged, 2 changed
    insert("a3", "a.example", Some(now_ms + 5_000), 1, 1, "succeeded"); // 0 unchanged, 0 changed
    insert("a4", "a.example", None, 2, 1, "gone"); // 1 unchanged, 0 changed, 1 gone

    // b.example: 1 overdue (lag 9000ms => median 9000ms, max 9000ms).
    insert("b1", "b.example", Some(now_ms - 9_000), 2, 2, "succeeded"); // 0 unchanged, 1 changed

    let report = super::freshness(&conn, now_ms).unwrap();
    // Overall lags: [1000, 3000, 9000] => median 3000ms, largest 9000ms.
    assert_eq!(report.overdue, 3);
    assert_eq!(report.median_lag, Some(Duration::from_millis(3_000)));
    assert_eq!(report.largest_lag, Some(Duration::from_millis(9_000)));
    assert_eq!(report.unchanged, 4);
    assert_eq!(report.changed, 3);
    assert_eq!(report.gone, 1);
    assert_eq!(report.trusted_lastmod_hosts, Vec::<String>::new());
    assert_eq!(report.domains.len(), 2);

    let a = &report.domains[0];
    assert_eq!(a.domain, "a.example");
    assert_eq!(a.overdue, 2);
    assert_eq!(a.median_lag, Some(Duration::from_millis(2_000)));
    assert_eq!(a.largest_lag, Some(Duration::from_millis(3_000)));
    assert_eq!(a.unchanged, 4);
    assert_eq!(a.changed, 2);
    assert_eq!(a.gone, 1);

    let b = &report.domains[1];
    assert_eq!(b.domain, "b.example");
    assert_eq!(b.overdue, 1);
    assert_eq!(b.median_lag, Some(Duration::from_millis(9_000)));
    assert_eq!(b.largest_lag, Some(Duration::from_millis(9_000)));
    assert_eq!(b.unchanged, 0);
    assert_eq!(b.changed, 1);
    assert_eq!(b.gone, 0);
}
