//! `browser-builds.test` (#3100): the files `serve --browser-builds` is
//! given, each at its path below the folder, and `404` for any other path
//! or without the folder.
#![expect(
    clippy::disallowed_methods,
    reason = "the test writes its builds' folder in its own thread"
)]

use goodfetch_test_support::{LOOPBACK_PORT_0, TestEnv, UNSHAPED, serve_arguments};

use crate::proving_ground::ProvingGround;

const SEED: u64 = 3100;

const ARCHIVE: &str = "/156.0.8078.4/linux64/chrome-linux64.zip";

#[test]
fn a_given_build_is_served_at_its_path_and_any_other_is_not_found() {
    let env = TestEnv::new();
    let folder = env.folder().join("builds");
    let archive = folder.join(ARCHIVE.trim_start_matches('/'));
    std::fs::create_dir_all(archive.parent().expect("the archive is in a folder")).unwrap();
    std::fs::write(&archive, b"PK\x03\x04 an archive").unwrap();
    let mut arguments = serve_arguments(SEED, LOOPBACK_PORT_0, UNSHAPED).to_vec();
    arguments.extend(["--browser-builds".to_owned(), folder.display().to_string()]);
    let proving_ground = ProvingGround::start_with(&arguments);

    let served = proving_ground.get_raw(&format!("http://browser-builds.test{ARCHIVE}"));
    let missing =
        proving_ground.get_raw("http://browser-builds.test/156.0.8078.4/linux64/missing.zip");

    assert_eq!(
        (
            served.status,
            served.content_type.as_deref(),
            served.body.as_slice(),
            missing.status,
        ),
        (
            200,
            Some("application/zip"),
            b"PK\x03\x04 an archive".as_slice(),
            404
        )
    );
}

#[test]
fn without_builds_every_path_is_not_found() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.get_raw(&format!("http://browser-builds.test{ARCHIVE}"));

    assert_eq!(answer.status, 404);
}
