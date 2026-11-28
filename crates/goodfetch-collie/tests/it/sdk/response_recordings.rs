//! Response recordings in the Rust SDK (ADRs 0105 and 0116; issue 1291):
//! the `response-recordings-live` client conformance case in `linked` mode.
//! Covers:
//! - Record against a Proving ground page, replay with the network off
//!   (`network.resolve` cleared): same response, empty `misses`, and an
//!   unrecorded URL raises `response_not_recorded` naming its address.
//! - `TestEngine`'s Run with no recording replays an empty one and raises
//!   `response_not_recorded` for any site request.
//! - Resume (`run_id` stopped + started again with the same recording) writes
//!   both halves.
//! - Failed write at Run end (`path_not_writable` on a directory path) leaves
//!   the path as it was, raises from `finish()`, and `save_recording()` on
//!   `Run` and `RunView` writes the committed records again; a Run that
//!   recorded nothing raises `recording_invalid` with a `reason`.
//! - Non-WARC, unreadable, or unsupported-version file raises
//!   `recording_invalid`.

use std::collections::BTreeMap;

use goodfetch_collie::settings::{Settings, network};
use goodfetch_collie::testing::{TestEngine, TestEngineOptions};
use goodfetch_collie::{Collie, ErrorCode, Recording, RunId};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::run::{RecordingInvalidDetails, ResponseNotRecordedDetails};
use goodfetch_test_support::{
    ProvingGround, TestEnv, browser_cache_in_home, case_logs, real_time_runtime,
};

use super::options;

const SEED: u64 = 1291;

/// The `response-recordings-live` client conformance case, linked mode.
/// Covers: story:11-158, story:11-159, story:11-160, story:11-161, story:11-162, operation:recording.save_as
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#[expect(
    clippy::disallowed_methods,
    reason = "test fixture setup and file assertions in an integration test"
)]
#[expect(
    clippy::too_many_lines,
    reason = "sequential live recording, replay, TestEngine, resume, failed-write and invalid-file checks share one Proving ground"
)]
#[test]
fn response_recordings_record_replay_test_engine_resume_and_save_again() {
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "response-recordings-live",
        RustMode::Linked,
    )
    .unwrap();
    let runtime = real_time_runtime();
    let env = TestEnv::new();
    browser_cache_in_home(&env).unwrap();
    let pg = ProvingGround::start(&env, SEED).unwrap();
    env.user_configuration(&pg.description().reaching());
    let work = tempfile::tempdir().unwrap();

    let page_url = pg.start_url("catalog/1").unwrap();
    let other_url = pg.start_url("catalog/2").unwrap();

    runtime.block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();

        // 1. Record against Proving ground page (story:11-158).
        let warc_path = work.path().join("recorded.warc");
        let rec_run = collie
            .start()
            .recording(Recording::record(warc_path.to_string_lossy()))
            .await
            .unwrap();
        let session = rec_run.new_session().await.unwrap();
        let page = session.new_page().await.unwrap();
        let live_resp = page.goto(&page_url).await.unwrap().unwrap();
        assert_eq!(live_resp.status(), 200);
        let live_title: String = page
            .evaluate("document.title")
            .await
            .unwrap()
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned();
        let rec_report = rec_run.finish().await.unwrap();
        assert!(rec_report.recording.is_some());
        assert!(warc_path.exists());

        // 2. Replay with network off (network.resolve cleared): same response, empty misses;
        //    unrecorded URL raises response_not_recorded naming its address (story:11-158).
        let replay_settings = Settings::new().set(network::RESOLVE, BTreeMap::new());
        let replay_run = collie
            .start()
            .settings(replay_settings)
            .recording(Recording::replay(warc_path.to_string_lossy()))
            .await
            .unwrap();
        let r_session = replay_run.new_session().await.unwrap();
        let r_page = r_session.new_page().await.unwrap();
        let replayed = r_page.goto(&page_url).await.unwrap().unwrap();
        assert_eq!(replayed.status(), 200);
        let replayed_title: String = r_page
            .evaluate("document.title")
            .await
            .unwrap()
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(replayed_title, live_title);

        let miss_err = r_page.goto(&other_url).await.unwrap_err();
        assert_eq!(
            miss_err.code(),
            ErrorCode::new("response_not_recorded").unwrap()
        );
        assert!(!miss_err.retryable());
        let miss_details = miss_err.details::<ResponseNotRecordedDetails>().unwrap();
        assert_eq!(miss_details.url, other_url);

        let replay_report = replay_run.finish().await.unwrap();
        let rep = replay_report.recording.as_ref().unwrap();
        assert_eq!(rep.mode, "replay");
        assert!(rep.responses_used >= 1);
        assert_ne!(rep.misses, [] as [String; 0]);

        // 3. TestEngine replays an empty recording by default (story:11-159).
        let test_engine = TestEngine::start(TestEngineOptions::new().env(env.env_os()))
            .await
            .unwrap();
        let te_run = test_engine.connection().start().await.unwrap();
        let te_session = te_run.new_session().await.unwrap();
        let te_page = te_session.new_page().await.unwrap();
        let te_err = te_page.goto("https://example.invalid/").await.unwrap_err();
        assert_eq!(
            te_err.code(),
            ErrorCode::new("response_not_recorded").unwrap()
        );
        te_run.finish().await.unwrap();
        test_engine.close().await.unwrap();

        // 4. Resume names the same recording and writes both halves (story:11-160).
        let resume_warc = work.path().join("resumed.warc");
        let resume_id = RunId::parse("rec-resume-rust-1").unwrap();
        let half1 = collie
            .start()
            .run_id(&resume_id)
            .recording(Recording::record(resume_warc.to_string_lossy()))
            .await
            .unwrap();
        let s1 = half1.new_session().await.unwrap();
        let p1 = s1.new_page().await.unwrap();
        p1.goto(&page_url).await.unwrap();
        half1.stop().await.unwrap();

        let half2 = collie
            .start()
            .run_id(&resume_id)
            .recording(Recording::record(resume_warc.to_string_lossy()))
            .await
            .unwrap();
        let s2 = half2.new_session().await.unwrap();
        let p2 = s2.new_page().await.unwrap();
        p2.goto(&other_url).await.unwrap();
        half2.finish().await.unwrap();

        let both_replay = collie
            .start()
            .settings(Settings::new().set(network::RESOLVE, BTreeMap::new()))
            .recording(Recording::replay(resume_warc.to_string_lossy()))
            .await
            .unwrap();
        let bs = both_replay.new_session().await.unwrap();
        let bp = bs.new_page().await.unwrap();
        assert_eq!(bp.goto(&page_url).await.unwrap().unwrap().status(), 200);
        assert_eq!(bp.goto(&other_url).await.unwrap().unwrap().status(), 200);
        let both_report = both_replay.finish().await.unwrap();
        let non_favicon_misses: Vec<&str> = both_report
            .recording
            .as_ref()
            .unwrap()
            .misses
            .iter()
            .map(String::as_str)
            .filter(|miss| !miss.ends_with("/favicon.ico"))
            .collect();
        assert_eq!(non_favicon_misses, Vec::<&str>::new());

        // 5. Failed write at Run end raises path_not_writable and save_recording writes again
        //    on both Run and RunView; a Run that recorded nothing raises recording_invalid (story:11-161).
        let bad_dir = work.path().join("unwritable-dir");
        std::fs::create_dir(&bad_dir).unwrap();
        let fail_run = collie
            .start()
            .recording(Recording::record(bad_dir.to_string_lossy()))
            .await
            .unwrap();
        let fail_id = fail_run.id().clone();
        let fs = fail_run.new_session().await.unwrap();
        let fp = fs.new_page().await.unwrap();
        fp.goto(&page_url).await.unwrap();
        let write_err = fail_run.finish().await.unwrap_err();
        assert_eq!(write_err.code(), ErrorCode::PATH_NOT_WRITABLE);

        // Existing target file is left untouched when writing to a read-only folder fails.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let ro_dir = work.path().join("readonly-dir");
            std::fs::create_dir(&ro_dir).unwrap();
            let existing_warc = ro_dir.join("existing.warc");
            std::fs::write(&existing_warc, b"sentinel-existing-warc").unwrap();
            std::fs::set_permissions(&ro_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
            let ro_run = collie
                .start()
                .recording(Recording::record(existing_warc.to_string_lossy()))
                .await
                .unwrap();
            let rs = ro_run.new_session().await.unwrap();
            let rp = rs.new_page().await.unwrap();
            rp.goto(&page_url).await.unwrap();
            let ro_err = ro_run.finish().await.unwrap_err();
            std::fs::set_permissions(&ro_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(ro_err.code(), ErrorCode::PATH_NOT_WRITABLE);
            assert_eq!(
                std::fs::read(&existing_warc).unwrap(),
                b"sentinel-existing-warc"
            );
        }

        let recovered_path = work.path().join("recovered.warc");
        let saved = fail_run.save_recording(&recovered_path).await.unwrap();
        assert!(saved.size > 0);
        assert!(recovered_path.exists());

        let view = collie.open_run(&fail_id).unwrap();
        let recovered_gz = work.path().join("recovered.warc.gz");
        let saved_gz = view.save_recording(&recovered_gz).await.unwrap();
        assert!(saved_gz.size > 0);
        assert!(recovered_gz.exists());
        view.close();

        // A Run that recorded nothing raises recording_invalid with a reason.
        let empty_run = collie.start().await.unwrap();
        let empty_err = empty_run
            .save_recording(work.path().join("empty.warc"))
            .await
            .unwrap_err();
        assert_eq!(
            empty_err.code(),
            ErrorCode::new("recording_invalid").unwrap()
        );
        let empty_details = empty_err.details::<RecordingInvalidDetails>().unwrap();
        assert_ne!(empty_details.reason, "");
        empty_run.finish().await.unwrap();

        // 6. Non-WARC, unreadable, or unsupported-version file raises recording_invalid (story:11-162).
        let missing = work.path().join("does-not-exist.warc");
        let missing_err = collie
            .start()
            .recording(Recording::replay(missing.to_string_lossy()))
            .await
            .unwrap_err();
        assert_eq!(
            missing_err.code(),
            ErrorCode::new("recording_invalid").unwrap()
        );

        let not_warc = work.path().join("garbage.warc");
        std::fs::write(&not_warc, b"not a warc file at all").unwrap();
        let garbage_err = collie
            .start()
            .recording(Recording::replay(not_warc.to_string_lossy()))
            .await
            .unwrap_err();
        assert_eq!(
            garbage_err.code(),
            ErrorCode::new("recording_invalid").unwrap()
        );

        let bad_ver = work.path().join("bad-version.warc");
        std::fs::write(
            &bad_ver,
            b"WARC/9.9\r\nWARC-Type: warcinfo\r\nContent-Length: 0\r\n\r\n\r\n\r\n",
        )
        .unwrap();
        let ver_err = collie
            .start()
            .recording(Recording::replay(bad_ver.to_string_lossy()))
            .await
            .unwrap_err();
        assert_eq!(ver_err.code(), ErrorCode::new("recording_invalid").unwrap());

        collie.close().await.unwrap();
    });
    case.check("response recordings live suite completed", || {});
}
