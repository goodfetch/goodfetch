use serde_json::json;

use super::{LimitSetting, ResponseLimitExceededDetails};

#[test]
fn response_limit_exceeded_names_the_url_the_setting_and_its_value_on_the_wire() {
    let details = ResponseLimitExceededDetails {
        url: "http://bombs.test/".to_owned(),
        setting: LimitSetting::MaxDecompressionRatio,
        limit: 100,
    };

    assert_eq!(
        serde_json::to_value(&details).unwrap(),
        json!({
            "url": "http://bombs.test/",
            "setting": "network.max_decompression_ratio",
            "limit": 100,
        })
    );
}
