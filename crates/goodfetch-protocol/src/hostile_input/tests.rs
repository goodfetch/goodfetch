use serde_json::json;

use super::{HeaderCharacter, HeaderValueInvalidDetails, PathHazard, PathUnsafeDetails};

#[test]
fn header_value_invalid_names_the_header_and_the_character_never_the_value() {
    let details = HeaderValueInvalidDetails {
        name: "X-Note".to_owned(),
        character: HeaderCharacter::Lf,
    };

    assert_eq!(
        serde_json::to_value(&details).unwrap(),
        json!({ "name": "X-Note", "character": "lf" })
    );
}

#[test]
fn path_unsafe_names_the_path_and_its_hazard() {
    let details = PathUnsafeDetails {
        name: "../../report.csv".to_owned(),
        hazard: PathHazard::ParentTraversal,
    };

    assert_eq!(
        serde_json::to_value(&details).unwrap(),
        json!({ "name": "../../report.csv", "hazard": "parent_traversal" })
    );
}
