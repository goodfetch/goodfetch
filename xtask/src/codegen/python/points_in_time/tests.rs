//! Each `date-time` of a generator input mapped to the client package's
//! point in time, as a function of the input.

use serde_json::json;

use super::{POINT_IN_TIME, map};

/// The protocol's point in time leaves `$defs`, and every reference to it,
/// at any depth and beside a description of its own, and every inline
/// `date-time` become the client package's type; any other schema stays.
#[test]
fn every_date_time_becomes_the_client_packages_point_in_time() {
    let mut input = json!({
        "$defs": {
            "PointInTime": {
                "description": "A point in time in UTC, as RFC 3339.",
                "type": "string",
                "format": "date-time",
            },
            "Progress": {
                "type": "object",
                "properties": {
                    "at": { "$ref": "#/$defs/PointInTime" },
                    "pauses": {
                        "type": "array",
                        "items": {
                            "anyOf": [
                                { "$ref": "#/$defs/PointInTime", "description": "Since." },
                                { "type": "null" },
                            ],
                        },
                    },
                    "seen": { "type": "string", "format": "date-time" },
                    "day": { "type": "string", "format": "date" },
                    "run_id": { "$ref": "#/$defs/RunId" },
                },
            },
            "RunId": { "type": "string" },
        },
    });

    map(&mut input).unwrap();

    assert_eq!(
        input,
        json!({
            "$defs": {
                "Progress": {
                    "type": "object",
                    "properties": {
                        "at": { "customTypePath": POINT_IN_TIME },
                        "pauses": {
                            "type": "array",
                            "items": {
                                "anyOf": [
                                    { "customTypePath": POINT_IN_TIME, "description": "Since." },
                                    { "type": "null" },
                                ],
                            },
                        },
                        "seen": { "customTypePath": POINT_IN_TIME },
                        "day": { "type": "string", "format": "date" },
                        "run_id": { "$ref": "#/$defs/RunId" },
                    },
                },
                "RunId": { "type": "string" },
            },
        })
    );
}

/// A `date-time` entry under another name is refused: Foxhound's layer
/// imports Collie's point in time by the protocol's name alone.
#[test]
fn a_date_time_entry_under_another_name_is_refused() {
    let mut input = json!({
        "$defs": {
            "Deadline": { "type": "string", "format": "date-time" },
        },
    });

    let refused = map(&mut input).unwrap_err().to_string();

    assert!(refused.contains("`Deadline`"), "{refused}");
}
