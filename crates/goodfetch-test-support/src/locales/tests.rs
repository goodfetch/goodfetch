use jiff::civil::date;

use super::{TimeZoneCase, locales, time_zones};

#[test]
fn the_shared_locales_are_adr_0073s_table() {
    let tags = locales();

    assert_eq!(
        tags,
        [
            "en-US", "de-DE", "fr-FR", "tr-TR", "ar-EG", "hi-IN", "ja-JP"
        ]
    );
}

#[test]
fn the_shared_time_zones_hold_the_dst_days_of_paris_and_new_york() {
    let zones = time_zones();

    let plain = |name: &str| TimeZoneCase {
        name: name.to_owned(),
        dst_gap: None,
        dst_overlap: None,
    };
    assert_eq!(
        zones,
        [
            plain("UTC"),
            plain("America/St_Johns"),
            plain("Asia/Kathmandu"),
            plain("Pacific/Chatham"),
            plain("Australia/Lord_Howe"),
            TimeZoneCase {
                name: "Europe/Paris".to_owned(),
                dst_gap: Some(date(2026, 3, 29)),
                dst_overlap: Some(date(2026, 10, 25)),
            },
            TimeZoneCase {
                name: "America/New_York".to_owned(),
                dst_gap: Some(date(2026, 3, 8)),
                dst_overlap: Some(date(2026, 11, 1)),
            },
        ]
    );
}
