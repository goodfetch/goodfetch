"""Response recordings in the Python SDK (ADRs 0105 and 0116; issue 1291).

- `response-recordings-live` (attached, three ways, Proving ground + TestEngine):
  record and replay offline, TestEngine empty replay default, resume writes both
  halves, failed write + save_recording on Run and RunView, empty recording
  refusal, and invalid/unreadable WARC refusal.
- `response-recordings-daemon` (Scripted engine, attached and daemon, three ways):
  replay reads file and sends `recording_bytes`, and `save_recording` streams
  `$/chunk` bytes atomically on Run and RunView.
"""

import json
from pathlib import Path

import anyio.to_thread
import pytest
from collie_calls import exit_code, failure, open_collie
from goodfetch_collie import (
    PathNotWritableError,
    Recording,
    RecordingInvalidError,
    RecordingMode,
    ResponseNotRecordedError,
)
from goodfetch_collie.testing import AsyncTestEngine, TestEngine
from goodfetch_test_support import (
    ClientCaseRecord,
    ProvingGround,
    TestEnv,
    Way,
    scripted_engine_env,
    start_scripted_daemon,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

COVERS_LIVE = pytest.mark.covers(
    "story:11-158",
    "story:11-159",
    "story:11-160",
    "story:11-161",
    "story:11-162",
    "operation:recording.save_as",
)

COVERS_DAEMON = pytest.mark.covers(
    "story:11-161",
    "story:11-162",
    "operation:recording.save_as",
)


CORPUS = (
    Path(__file__).resolve().parents[5] / "test-support" / "fixtures" / "engine-scripts"
)


def _script_with_recording_paths(test_env: TestEnv, script: str, out_dir: Path) -> str:
    input_path = str((out_dir / "input.warc").resolve())
    saved_path = str((out_dir / "saved.warc").resolve())
    view_path = str((out_dir / "saved-view.warc").resolve())
    text = (
        (CORPUS / script)
        .read_text("utf-8")
        .replace(json.dumps("/work/out/input.warc"), json.dumps(input_path))
        .replace(json.dumps("/work/out/saved.warc"), json.dumps(saved_path))
        .replace(json.dumps("/work/out/saved-view.warc"), json.dumps(view_path))
    )
    target = test_env.folder / "recording-scripts" / out_dir.name / script
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(text, "utf-8")
    return str(target)


@COVERS_DAEMON
@pytest.mark.client_case(id="response-recordings-daemon", mode="attached")
@three_ways
async def test_response_recordings_daemon_attached(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    tmp_path: Path,
    client_case: ClientCaseRecord,
) -> None:
    """Replay sends recording_bytes and save_recording streams chunks (attached)."""
    out_dir = tmp_path / way.value
    out_dir.mkdir(parents=True, exist_ok=True)
    input_warc = (out_dir / "input.warc").resolve()
    input_warc.write_bytes(b"WARC/1.1\r\n")
    script = _script_with_recording_paths(
        test_env, "sdk-python/run/response-recordings-daemon.json", out_dir
    )
    env = scripted_engine_env(test_env, product, script)
    collie = await open_collie(way, env)
    try:
        missing_err = await failure(
            collie.start(
                recording=Recording(
                    mode=RecordingMode.replay, path=str(out_dir / "missing.warc")
                )
            )
        )
        with client_case.check("missing replay file raises recording_invalid"):
            assert isinstance(missing_err, RecordingInvalidError)

        run = await collie.start(
            recording=Recording(mode=RecordingMode.replay, path=str(input_warc))
        )
        saved_path = (out_dir / "saved.warc").resolve()
        saved = await run.save_recording(saved_path)
        with client_case.check("Run.save_recording streams bytes atomically"):
            assert not saved.written
            assert saved_path.read_bytes() == b"WARC/1.1\r\n"

        view = await collie.open_run(run.id)
        view_path = (out_dir / "saved-view.warc").resolve()
        saved_view = await view.save_recording(view_path)
        with client_case.check("RunView.save_recording streams bytes atomically"):
            assert not saved_view.written
            assert view_path.read_bytes() == b"WARC/1.1\r\n"
        await view.close()

        await run.finish()
    finally:
        await collie.close()


@COVERS_DAEMON
@pytest.mark.client_case(id="response-recordings-daemon", mode="daemon")
@three_ways
async def test_response_recordings_daemon_mode(
    way: Way,
    test_env: TestEnv,
    tmp_path: Path,
    client_case: ClientCaseRecord,
) -> None:
    """Replay sends recording_bytes and save_recording streams chunks (daemon)."""
    out_dir = tmp_path / way.value
    out_dir.mkdir(parents=True, exist_ok=True)
    input_warc = (out_dir / "input.warc").resolve()
    input_warc.write_bytes(b"WARC/1.1\r\n")
    script = _script_with_recording_paths(
        test_env,
        "sdk-python/run/response-recordings-daemon-from-a-daemon.json",
        out_dir,
    )
    daemon = start_scripted_daemon(test_env, script, "socket")
    collie = await open_collie(way, test_env.env, engine="daemon")
    try:
        run = await collie.start(
            recording=Recording(mode=RecordingMode.replay, path=str(input_warc))
        )
        saved_path = (out_dir / "saved.warc").resolve()
        saved = await run.save_recording(saved_path)
        with client_case.check("Daemon Run.save_recording streams bytes atomically"):
            assert not saved.written
            assert saved_path.read_bytes() == b"WARC/1.1\r\n"

        view = await collie.open_run(run.id)
        view_path = (out_dir / "saved-view.warc").resolve()
        saved_view = await view.save_recording(view_path)
        with client_case.check(
            "Daemon RunView.save_recording streams bytes atomically"
        ):
            assert not saved_view.written
            assert view_path.read_bytes() == b"WARC/1.1\r\n"
        await view.close()

        await run.finish()
    finally:
        await collie.close()
    code = await exit_code(daemon.wait)
    with client_case.check("the Daemon played its script through"):
        assert code == 0


class TestResponseRecordingsOnProvingGround:
    """Response recordings against a real Proving ground and TestEngine."""

    @pytest.fixture
    def proving_ground_seed(self) -> int:
        """The Proving ground's own seed for these tests."""
        return 1291

    @pytest.fixture
    def with_browser(self) -> bool:
        """Launch a real browser from the caller's cache."""
        return True

    @COVERS_LIVE
    @pytest.mark.client_case(id="response-recordings-live", mode="attached")
    @three_ways
    async def test_response_recordings_live(
        self,
        way: Way,
        env: dict[str, str],
        proving_ground: ProvingGround,
        tmp_path: Path,
        client_case: ClientCaseRecord,
    ) -> None:
        """Record, replay offline, TestEngine empty replay, resume, and failed write."""
        page_url = await anyio.to_thread.run_sync(proving_ground.start_url, "catalog/1")
        other_url = await anyio.to_thread.run_sync(
            proving_ground.start_url, "catalog/2"
        )
        collie = await open_collie(way, env)
        try:
            # 1. Record against Proving ground page.
            warc_path = tmp_path / "recorded.warc"
            rec_run = await collie.start(
                recording=Recording(mode=RecordingMode.record, path=str(warc_path))
            )
            rec_session = await rec_run.new_session()
            rec_page = await rec_session.new_page()
            live_resp = await rec_page.goto(page_url)
            assert live_resp is not None and live_resp.status == 200
            live_title = await rec_page.evaluate("document.title")
            assert isinstance(live_title, str)
            rec_report = await rec_run.finish()
            with client_case.check("record mode writes WARC and populates report"):
                assert rec_report.recording is not None
                assert rec_report.recording.mode == "record"

            # 2. Replay with network off (network.resolve cleared).
            replay_run = await collie.start(
                settings={"network": {"resolve": {}}},
                recording=Recording(mode=RecordingMode.replay, path=str(warc_path)),
            )
            r_session = await replay_run.new_session()
            r_page = await r_session.new_page()
            replayed = await r_page.goto(page_url)
            assert replayed is not None and replayed.status == 200
            replayed_title = await r_page.evaluate("document.title")
            assert replayed_title == live_title
            miss_err = await failure(r_page.goto(other_url))
            with client_case.check("unrecorded request raises response_not_recorded"):
                assert isinstance(miss_err, ResponseNotRecordedError)
                assert not miss_err.retryable
            replay_report = await replay_run.finish()
            with client_case.check("replay report lists responses used and misses"):
                assert replay_report.recording is not None
                assert replay_report.recording.mode == "replay"
                assert replay_report.recording.misses

            # 3. TestEngine replays an empty recording by default.
            if way == Way.SYNC:

                def check_sync_test_engine() -> None:
                    with TestEngine.start(env=env) as te:
                        with te.connection.start() as te_run:
                            te_page = te_run.new_session().new_page()
                            with pytest.raises(ResponseNotRecordedError):
                                te_page.goto("https://example.invalid/")
                            te_run.finish()

                await anyio.to_thread.run_sync(check_sync_test_engine)
            else:
                async with AsyncTestEngine.start(env=env) as te:
                    async with await te.connection.start() as te_run:
                        te_page = await (await te_run.new_session()).new_page()
                        te_err = await failure(te_page.goto("https://example.invalid/"))
                        with client_case.check(
                            "TestEngine replays empty recording by default"
                        ):
                            assert isinstance(te_err, ResponseNotRecordedError)
                        await te_run.finish()

            # 4. Resume names the same file and writes both halves.
            resume_warc = tmp_path / "resumed.warc"
            resume_id = f"rec-resume-py-{way.value}"
            half1 = await collie.start(
                run_id=resume_id,
                recording=Recording(mode=RecordingMode.record, path=str(resume_warc)),
            )
            p1 = await (await half1.new_session()).new_page()
            await p1.goto(page_url)
            await half1.stop()

            half2 = await collie.start(
                run_id=resume_id,
                recording=Recording(mode=RecordingMode.record, path=str(resume_warc)),
            )
            p2 = await (await half2.new_session()).new_page()
            await p2.goto(other_url)
            await half2.finish()

            both_run = await collie.start(
                settings={"network": {"resolve": {}}},
                recording=Recording(mode=RecordingMode.replay, path=str(resume_warc)),
            )
            bp = await (await both_run.new_session()).new_page()
            r1 = await bp.goto(page_url)
            r2 = await bp.goto(other_url)
            assert r1 is not None and r1.status == 200
            assert r2 is not None and r2.status == 200
            both_report = await both_run.finish()
            with client_case.check(
                "resumed recording replays both halves with empty misses"
            ):
                assert both_report.recording is not None
                assert not both_report.recording.misses

            # 5. Failed write at Run end raises path_not_writable;
            #    save_recording writes again.
            bad_dir = tmp_path / "unwritable-dir"
            bad_dir.mkdir()
            fail_run = await collie.start(
                recording=Recording(mode=RecordingMode.record, path=str(bad_dir))
            )
            await (await (await fail_run.new_session()).new_page()).goto(page_url)
            write_err = await failure(fail_run.finish())
            with client_case.check("failed write at Run end raises path_not_writable"):
                assert isinstance(write_err, PathNotWritableError)

            recovered = tmp_path / "recovered.warc"
            saved = await fail_run.save_recording(recovered)
            view = await collie.open_run(fail_run.id)
            recovered_gz = tmp_path / "recovered.warc.gz"
            saved_gz = await view.save_recording(recovered_gz)
            await view.close()
            with client_case.check(
                "save_recording recovers committed records on Run and RunView"
            ):
                assert saved.size > 0
                assert saved_gz.size > 0

            # Empty recording raises recording_invalid.
            empty_run = await collie.start()
            empty_err = await failure(empty_run.save_recording(tmp_path / "empty.warc"))
            with client_case.check(
                "save_recording on empty Run raises recording_invalid"
            ):
                assert isinstance(empty_err, RecordingInvalidError)
            await empty_run.finish()

            # 6. Invalid / unsupported-version WARC raises recording_invalid.
            garbage = tmp_path / "garbage.warc"
            garbage.write_text("not a warc")
            garbage_err = await failure(
                collie.start(
                    recording=Recording(mode=RecordingMode.replay, path=str(garbage))
                )
            )
            bad_ver = tmp_path / "bad-ver.warc"
            bad_ver.write_text(
                "WARC/9.9\r\nWARC-Type: warcinfo\r\nContent-Length: 0\r\n\r\n\r\n\r\n"
            )
            bad_ver_err = await failure(
                collie.start(
                    recording=Recording(mode=RecordingMode.replay, path=str(bad_ver))
                )
            )
            with client_case.check(
                "invalid or unsupported-version WARC raises recording_invalid"
            ):
                assert isinstance(garbage_err, RecordingInvalidError)
                assert isinstance(bad_ver_err, RecordingInvalidError)
        finally:
            await collie.close()
