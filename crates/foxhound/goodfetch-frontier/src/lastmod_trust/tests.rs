use goodfetch_protocol::foxhound::continuous::RevisitPolicy;
use rusqlite::Connection;

use crate::sitemap::{SitemapDate, SitemapEntry};

use super::{
    ClaimKind, DAY_MS, TABLES, TrustThresholds, brought_forward_due, evaluate_trust,
    is_claim_later, is_host_trusted, is_stamped_sitemap, judge_visit, open_claim, summary,
};

fn memory_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    for sql in TABLES.statements {
        conn.execute_batch(sql).unwrap();
    }
    conn
}

fn entry_with_lastmod(loc: &str, ms: u64, date_only: bool) -> SitemapEntry {
    SitemapEntry {
        loc: loc.to_owned(),
        lastmod: Some(SitemapDate { ms, date_only }),
        news_publication_date: None,
        changefreq: None,
        priority: None,
    }
}

#[test]
fn date_only_compares_by_whole_utc_day_and_future_over_24h_is_ignored() {
    let day_1 = 10 * DAY_MS;
    let day_2 = 11 * DAY_MS;
    let now = day_2 + 3_600_000;

    // Date-only on same UTC day as last_fetched_at is NOT later.
    assert!(!is_claim_later(
        SitemapDate {
            ms: day_1,
            date_only: true
        },
        day_1 + 12 * 3_600_000,
        now
    ));
    // Date-only on next UTC day IS later.
    assert!(is_claim_later(
        SitemapDate {
            ms: day_2,
            date_only: true
        },
        day_1 + 23 * 3_600_000,
        now
    ));
    // More than 24h ahead of now is ignored.
    assert!(!is_claim_later(
        SitemapDate {
            ms: now + DAY_MS + 1,
            date_only: false
        },
        day_1,
        now
    ));
    // Exactly 24h ahead of now is accepted.
    assert!(is_claim_later(
        SitemapDate {
            ms: now + DAY_MS,
            date_only: false
        },
        day_1,
        now
    ));
}

#[test]
fn stamped_sitemap_requires_at_least_10_entries_and_90_percent_same_date() {
    let stamp = 100 * DAY_MS;
    // 9 entries with same date -> NOT stamped (< 10 entries).
    let nine: Vec<SitemapEntry> = (0..9)
        .map(|i| entry_with_lastmod(&format!("https://a.test/{i}"), stamp, false))
        .collect();
    assert!(!is_stamped_sitemap(&nine));

    // 10 entries: 9 same + 1 different -> 90% -> stamped!
    let mut ten = nine;
    ten.push(entry_with_lastmod(
        "https://a.test/9",
        stamp + 60_000,
        false,
    ));
    assert!(is_stamped_sitemap(&ten));

    // 10 entries: 8 same + 2 different -> 80% -> NOT stamped.
    ten[8] = entry_with_lastmod("https://a.test/8", stamp + 120_000, false);
    assert!(!is_stamped_sitemap(&ten));
}

#[test]
fn brought_forward_respects_now_floors_min_and_never() {
    let adaptive = RevisitPolicy::Adaptive {
        initial: 86_400_000,
        min: 3_600_000,
        max: 30 * DAY_MS,
        rate_changed: 0.5,
        rate_unchanged: 0.5,
    };
    assert_eq!(
        brought_forward_due(10_000, 1_000, None, &RevisitPolicy::Never),
        None
    );
    // last_fetched + min (1_000 + 3_600_000) beats now (10_000).
    assert_eq!(
        brought_forward_due(10_000, 1_000, None, &adaptive),
        Some(3_601_000)
    );
    // Floor beats both now and last_fetched + min.
    assert_eq!(
        brought_forward_due(10_000, 1_000, Some(5_000_000), &adaptive),
        Some(5_000_000)
    );
    // now beats last_fetched + min and floor.
    assert_eq!(
        brought_forward_due(9_000_000, 1_000, Some(5_000_000), &adaptive),
        Some(9_000_000)
    );
}

#[expect(
    clippy::too_many_lines,
    reason = "exercises earn, withdraw, re-earn, miss, and news false-claim lifecycle in one sequence"
)]
#[test]
fn trust_earned_withdrawn_re_earned_misses_do_not_hurt_and_news_false_claims_counted() {
    let conn = memory_db();
    let thresholds = TrustThresholds {
        min_claims: 4,
        min_precision: 0.75,
        window: 4,
    };

    // 3 confirmed claims: below min_claims (4) -> not trusted yet.
    for i in 0..3 {
        let item = format!("w-{i}");
        open_claim(
            &conn,
            &item,
            "honest.test",
            "honest.test",
            ClaimKind::Lastmod,
            1_000,
        )
        .unwrap();
        judge_visit(
            &conn,
            &item,
            "honest.test",
            "honest.test",
            true,
            true,
            thresholds,
        )
        .unwrap();
    }
    assert!(!is_host_trusted(&conn, "honest.test").unwrap());

    // 4th confirmed claim -> 4/4 = 1.0 >= 0.75 -> trusted!
    open_claim(
        &conn,
        "w-3",
        "honest.test",
        "honest.test",
        ClaimKind::Lastmod,
        2_000,
    )
    .unwrap();
    judge_visit(
        &conn,
        "w-3",
        "honest.test",
        "honest.test",
        true,
        true,
        thresholds,
    )
    .unwrap();
    assert!(is_host_trusted(&conn, "honest.test").unwrap());

    // A miss (changed revisit with no open claim) is counted and does NOT hurt trust.
    judge_visit(
        &conn,
        "w-miss",
        "honest.test",
        "honest.test",
        true,
        true,
        thresholds,
    )
    .unwrap();
    assert!(is_host_trusted(&conn, "honest.test").unwrap());

    // Two false claims drop the 4-claim window to 2/4 = 0.50 < 0.75 -> trust withdrawn!
    for i in 4..6 {
        let item = format!("w-{i}");
        open_claim(
            &conn,
            &item,
            "honest.test",
            "honest.test",
            ClaimKind::Lastmod,
            3_000,
        )
        .unwrap();
        judge_visit(
            &conn,
            &item,
            "honest.test",
            "honest.test",
            false,
            true,
            thresholds,
        )
        .unwrap();
    }
    assert!(!is_host_trusted(&conn, "honest.test").unwrap());

    // Three more confirmed claims push window to [0, 1, 1, 1] = 3/4 = 0.75 -> trust earned again!
    for i in 6..9 {
        let item = format!("w-{i}");
        open_claim(
            &conn,
            &item,
            "honest.test",
            "honest.test",
            ClaimKind::Lastmod,
            4_000,
        )
        .unwrap();
        judge_visit(
            &conn,
            &item,
            "honest.test",
            "honest.test",
            true,
            true,
            thresholds,
        )
        .unwrap();
    }
    assert!(is_host_trusted(&conn, "honest.test").unwrap());

    // news:publication_date false claim does NOT affect trust, counted in news_false_claims.
    open_claim(
        &conn,
        "w-news",
        "honest.test",
        "honest.test",
        ClaimKind::News,
        5_000,
    )
    .unwrap();
    judge_visit(
        &conn,
        "w-news",
        "honest.test",
        "honest.test",
        false,
        true,
        thresholds,
    )
    .unwrap();
    assert!(is_host_trusted(&conn, "honest.test").unwrap());

    let report = summary(&conn).unwrap();
    assert_eq!(report.trusted_hosts, vec!["honest.test"]);
    assert_eq!(report.hosts.len(), 1);
    assert_eq!(report.hosts[0].claims_judged, 9);
    assert_eq!(report.hosts[0].confirmed, 7);
    assert_eq!(report.hosts[0].false_claims, 2);
    assert_eq!(report.hosts[0].misses, 1);
    assert_eq!(report.news_false_claims.get("honest.test"), Some(&1));

    // Direct evaluate_trust edge check.
    assert!(!evaluate_trust(9, "111111111", TrustThresholds::default()));
    assert!(evaluate_trust(10, "1111111110", TrustThresholds::default()));
}
