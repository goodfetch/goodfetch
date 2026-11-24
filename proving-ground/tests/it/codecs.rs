//! The codec samples (#328, ADR 0074) on `codecs.test`: one small sample per
//! codec that `canPlayType` or Media Capabilities can claim, each with its
//! expected decode result. A plain HTTP client reads the samples and their
//! expected results; a stock Chromium, never Goodfetch, decodes them and
//! shows what it finds. Each sample file is at most 256 KiB with a
//! `.source.toml` naming how it was generated (ADR 0062).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::browser;
use crate::proving_ground::ProvingGround;

const SEED: u64 = 328;

const HOST: &str = "codecs.test";

/// The codecs every Chromium build decodes, royalty-free or out of patent.
const ROYALTY_FREE: [&str; 8] = ["vp8", "vp9", "av1", "opus", "vorbis", "flac", "pcm", "mp3"];

/// Every codec a sample is for.
const CODECS: [&str; 13] = [
    "h264", "hevc", "vp8", "vp9", "av1", "aac", "mp3", "opus", "vorbis", "flac", "pcm", "ac3",
    "eac3",
];

fn samples_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/codecs")
}

/// The samples the server lists, by name.
fn listed(proving_ground: &ProvingGround) -> Vec<Value> {
    let answer = proving_ground.get(&proving_ground.url(HOST, "/samples.json"));
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.json()["samples"]
        .as_array()
        .expect("the samples")
        .clone()
}

/// Covers: story:2-91
#[test]
fn each_sample_is_served_as_its_file_with_its_media_type_and_expected_result() {
    let proving_ground = ProvingGround::start(SEED);

    let samples = listed(&proving_ground);
    let page = proving_ground.get(&proving_ground.url(HOST, "/"));
    let script = proving_ground.get_raw(&proving_ground.url(HOST, "/codecs.js"));

    let names: Vec<&str> = samples
        .iter()
        .map(|sample| sample["name"].as_str().expect("a name"))
        .collect();
    assert_eq!(names, CODECS);
    for sample in &samples {
        let file = sample["file"].as_str().expect("a file name");
        let served = proving_ground.get_raw(&proving_ground.url(HOST, &format!("/samples/{file}")));
        let path = samples_folder().join(file);
        let on_disk = std::fs::read(&path).expect("the sample's file exists");
        let source = std::fs::read_to_string(path.with_extension("source.toml"))
            .expect("the sample has its `.source.toml`");

        assert_eq!(served.status, 200, "{file}");
        assert_eq!(
            served.content_type.as_deref(),
            sample["media_type"].as_str(),
            "{file}"
        );
        assert!(served.body == on_disk, "{file} is served as generated");
        assert!(
            on_disk.len() <= 256 * 1024,
            "{file}: {} bytes",
            on_disk.len()
        );
        assert!(source.contains("command = "), "{file}: {source}");
        assert!(
            sample["type"]
                .as_str()
                .is_some_and(|kind| kind.starts_with(sample["media_type"].as_str().unwrap_or("?"))),
            "{sample}"
        );
        assert!(sample["expected"].is_object(), "{sample}");
    }
    assert_eq!(page.status, 200);
    assert!(page.body.contains("id=\"results\""), "{}", page.body);
    assert_eq!(
        (script.status, script.content_type.as_deref()),
        (200, Some("application/javascript"))
    );
    assert_eq!(
        proving_ground
            .get(&proving_ground.url(HOST, "/samples/missing.mp4"))
            .status,
        404
    );
}

/// Covers: story:2-91
#[test]
fn every_codec_a_stock_chromium_claims_decodes_its_sample_to_the_expected_result() {
    let proving_ground = ProvingGround::start(SEED);

    let results = browser::run(
        "codecs",
        "codecs",
        &proving_ground.url(HOST, "/"),
        json!({}),
    );

    let claimed = |name: &str| {
        let result = &results[name];
        result["can_play_type"]
            .as_str()
            .is_some_and(|answer| !answer.is_empty())
            || result["media_capabilities"]["supported"] == json!(true)
    };
    for name in CODECS {
        let result = &results[name];
        assert!(result.is_object(), "{name} has a result: {results}");
        if claimed(name) {
            assert_eq!(
                result["decoded"],
                json!(true),
                "{name} is claimed and decodes to its expected result: {result}"
            );
        }
    }
    for name in ROYALTY_FREE {
        assert!(claimed(name), "{name} is claimed: {}", results[name]);
    }
}
