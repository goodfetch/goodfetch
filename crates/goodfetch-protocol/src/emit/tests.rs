use serde_json::json;

use super::emit;

#[test]
fn the_bundle_holds_the_schema_of_the_releases_file() {
    let emitted = emit();

    assert_eq!(
        emitted.bundle["files"]["compat/releases.toml"],
        json!({ "$ref": "#/$defs/ReleasesFile" })
    );
    assert_eq!(
        emitted.bundle["$defs"]["ReleasesFile"]["properties"]["release"]["items"],
        json!({ "$ref": "#/$defs/Release" })
    );
}
