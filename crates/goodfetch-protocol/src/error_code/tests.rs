use serde_json::json;

use super::ErrorCode;

/// Covers: story:11-36, story:11-187
#[test]
fn a_code_read_from_the_wire_equals_its_constant_and_matches_it() {
    let read: ErrorCode = serde_json::from_value(json!("target_not_unique")).unwrap();

    assert_eq!(read, ErrorCode::TARGET_NOT_UNIQUE);
    assert!(matches!(read, ErrorCode::TARGET_NOT_UNIQUE));
    assert_eq!(read.as_str(), "target_not_unique");
}

/// Covers: story:11-36, story:11-187
#[test]
fn a_code_the_program_does_not_know_stays_a_value() {
    let read: ErrorCode = serde_json::from_value(json!("added_by_a_newer_engine")).unwrap();

    assert_eq!(read.as_str(), "added_by_a_newer_engine");
    assert_eq!(read.to_string(), "added_by_a_newer_engine");
    assert_eq!(
        serde_json::to_value(read).unwrap(),
        json!("added_by_a_newer_engine")
    );
}

#[test]
fn a_code_longer_than_sixty_three_bytes_is_refused() {
    let longest = "a".repeat(ErrorCode::MAX_LEN);
    let longer = "a".repeat(ErrorCode::MAX_LEN + 1);

    assert_eq!(ErrorCode::new(&longest).unwrap().as_str(), longest);
    assert_eq!(ErrorCode::new(&longer), None);
    assert!(serde_json::from_value::<ErrorCode>(json!(longer)).is_err());
}
