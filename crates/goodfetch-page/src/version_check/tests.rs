//! The version check's grades at fixed dates, from a table in the release
//! table's shape (spec #599's Version check cases), and the shipped table.

use jiff::civil::{Date, date};
use rstest::rstest;

use super::{Channel, ChromeChannel, Gap, Grade, Level, ReleaseTable, describe, grade, of_product};

/// Chrome every four weeks to 104, then every two, as Chrome went in
/// 2026; Firefox every four weeks, 91 patched until 2026-03-15.
const TABLE: &str = r#"
[[chrome]]
major = 100
stable = "2026-01-06"

[[chrome]]
major = 101
stable = "2026-02-03"

[[chrome]]
major = 102
stable = "2026-03-03"

[[chrome]]
major = 103
stable = "2026-03-31"

[[chrome]]
major = 104
stable = "2026-04-28"

[[chrome]]
major = 105
stable = "2026-05-12"

[[chrome]]
major = 106
stable = "2026-05-26"

[[firefox]]
major = 90
stable = "2026-01-13"

[[firefox]]
major = 91
stable = "2026-02-10"
patched_until = "2026-03-15"

[[firefox]]
major = 92
stable = "2026-03-10"
"#;

fn table() -> ReleaseTable {
    ReleaseTable::parse(TABLE).expect("the test's table parses")
}

fn behind(level: Level, versions: u32, weeks: u32, current: u32) -> Grade {
    Grade::Behind(Gap {
        level,
        versions,
        weeks,
        current,
        unpatched_since: None,
    })
}

/// A day Chrome 107 is Stable: two weeks after 106, from 2026-06-09.
const JUNE_16: Date = date(2026, 6, 16);

/// Covers: story:8-46, story:8-47
#[rstest]
#[case::current(107, Grade::Current)]
#[case::ahead_of_stable(108, Grade::Current)]
#[case::one_behind_is_info(106, behind(Level::Info, 1, 1, 107))]
#[case::two_behind_is_a_notice(105, behind(Level::Notice, 2, 3, 107))]
#[case::three_behind_is_a_notice(104, behind(Level::Notice, 3, 5, 107))]
#[case::four_behind_is_outdated(103, behind(Level::Outdated, 4, 7, 107))]
#[case::older_than_the_table(99, behind(Level::Outdated, 8, 23, 107))]
fn a_build_is_graded_against_chromes_stable(#[case] major: u32, #[case] expected: Grade) {
    assert_eq!(grade(major, Channel::Chrome, &table(), JUNE_16), expected);
}

/// Covers: story:8-46, story:8-47
#[test]
fn past_the_table_majors_come_at_its_last_cadence() {
    // 107 on 2026-06-09, 108 on 06-23, 109 on 07-07.
    assert_eq!(
        grade(106, Channel::Chrome, &table(), date(2026, 7, 8)),
        behind(Level::Notice, 3, 4, 109)
    );
}

/// Covers: story:8-46
#[test]
fn a_day_before_the_table_grades_nothing() {
    assert_eq!(
        grade(99, Channel::Chrome, &table(), date(2025, 12, 1)),
        Grade::NotGraded
    );
}

/// Covers: story:8-47
#[rstest]
#[case::still_patched(date(2026, 3, 12), behind(Level::Info, 1, 0, 92))]
#[case::no_longer_patched(
    date(2026, 3, 20),
    Grade::Behind(Gap {
        level: Level::Outdated,
        versions: 1,
        weeks: 1,
        current: 92,
        unpatched_since: Some(date(2026, 3, 15)),
    })
)]
fn a_firefox_build_its_vendor_no_longer_patches_is_outdated(
    #[case] today: Date,
    #[case] expected: Grade,
) {
    assert_eq!(grade(91, Channel::Firefox, &table(), today), expected);
}

/// Covers: story:8-48
#[rstest]
#[case::current_beta(ChromeChannel::Beta, 108, Grade::Current)]
#[case::current_extended_stable(ChromeChannel::ExtendedStable, 106, Grade::Current)]
#[case::supported_dev_never_above_a_notice(
    ChromeChannel::Dev,
    105,
    behind(Level::Notice, 4, 7, 109)
)]
#[case::supported_stable_major_of_the_last_eight_weeks(
    ChromeChannel::Stable,
    104,
    behind(Level::Notice, 3, 5, 107)
)]
#[case::extended_stable_one_release_behind(
    ChromeChannel::ExtendedStable,
    104,
    behind(Level::Notice, 2, 3, 106)
)]
#[case::unsupported_stable_is_outdated(
    ChromeChannel::Stable,
    103,
    behind(Level::Outdated, 4, 7, 107)
)]
#[case::unsupported_extended_stable_is_outdated(
    ChromeChannel::ExtendedStable,
    100,
    behind(Level::Outdated, 6, 15, 106)
)]
fn an_installed_chrome_is_graded_against_its_own_channel(
    #[case] channel: ChromeChannel,
    #[case] major: u32,
    #[case] expected: Grade,
) {
    assert_eq!(
        grade(major, Channel::InstalledChrome(channel), &table(), JUNE_16),
        expected
    );
}

/// Covers: story:8-48
#[test]
fn webkit_is_not_graded() {
    assert_eq!(
        grade(1, Channel::WebKit, &table(), JUNE_16),
        Grade::NotGraded
    );
}

#[rstest]
#[case::headless_chrome("HeadlessChrome/156.0.8078.4", Some((156, Channel::Chrome)))]
#[case::chrome("Chrome/157.0.7900.12", Some((157, Channel::Chrome)))]
#[case::firefox("Firefox/160.0", Some((160, Channel::Firefox)))]
#[case::another_browser("Scripted/1.0", None)]
#[case::no_major("Chrome/x.0", None)]
fn a_product_names_its_build(#[case] product: &str, #[case] expected: Option<(u32, Channel)>) {
    assert_eq!(of_product(product), expected);
}

/// Covers: story:8-47
#[test]
fn an_outdated_build_is_told_its_gap_in_versions_and_weeks() {
    let Grade::Behind(gap) = grade(103, Channel::Chrome, &table(), JUNE_16) else {
        panic!("Chrome 103 is behind on 2026-06-16");
    };
    assert_eq!(
        describe(103, Channel::Chrome, &gap),
        "The browser build, Chrome 103, is outdated: 4 versions and 7 weeks behind Chrome's Stable, 107. Update it, or pin a newer build (ADR 0035)."
    );
}

/// Covers: story:8-47
#[test]
fn an_unpatched_build_is_told_since_when() {
    let Grade::Behind(gap) = grade(91, Channel::Firefox, &table(), date(2026, 3, 20)) else {
        panic!("Firefox 91 is behind on 2026-03-20");
    };
    assert_eq!(
        describe(91, Channel::Firefox, &gap),
        "The browser build, Firefox 91, is outdated: 1 version and 1 week behind Firefox's release, 92, and its vendor stopped patching it on 2026-03-15. Update it, or pin a newer build (ADR 0035)."
    );
}

/// Covers: story:8-46
#[test]
fn the_shipped_table_is_embedded_and_reads() {
    let shipped = ReleaseTable::shipped();
    assert!(
        shipped.chrome.len() > 1 && shipped.firefox.len() > 1,
        "the shipped table holds Chrome's and Firefox's releases"
    );
}

#[rstest]
#[case::majors_out_of_order(
    "[[chrome]]\nmajor = 2\nstable = \"2026-01-01\"\n[[chrome]]\nmajor = 1\nstable = \"2026-02-01\"\n[[firefox]]\nmajor = 1\nstable = \"2026-01-01\"\n[[firefox]]\nmajor = 2\nstable = \"2026-02-01\"\n"
)]
#[case::one_release_has_no_cadence(
    "[[chrome]]\nmajor = 1\nstable = \"2026-01-01\"\n[[firefox]]\nmajor = 1\nstable = \"2026-01-01\"\n[[firefox]]\nmajor = 2\nstable = \"2026-02-01\"\n"
)]
#[case::an_unknown_field(
    "[[chrome]]\nmajor = 1\nstable = \"2026-01-01\"\nbeta = \"2025-12-01\"\n[[chrome]]\nmajor = 2\nstable = \"2026-02-01\"\n[[firefox]]\nmajor = 1\nstable = \"2026-01-01\"\n[[firefox]]\nmajor = 2\nstable = \"2026-02-01\"\n"
)]
fn a_table_out_of_shape_is_refused(#[case] text: &str) {
    assert!(ReleaseTable::parse(text).is_err());
}
