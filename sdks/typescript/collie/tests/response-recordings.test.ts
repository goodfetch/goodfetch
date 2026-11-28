// Response recordings in the TypeScript SDK (ADRs 0105 and 0116; issue 1291):
// - `response-recordings-live` (attached, Proving ground + TestEngine): record
//   and replay offline, TestEngine empty replay default, resume writes both
//   halves, failed write + saveRecording on Run and RunView, empty recording
//   refusal, and invalid/unreadable WARC refusal.
// - `response-recordings-daemon` (Scripted engine, attached and daemon):
//   replay reads file and sends `recording_bytes`, and `saveRecording` streams
//   `$/chunk` bytes atomically on Run and RunView.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import {
  Collie,
  PathNotWritableError,
  RecordingInvalidError,
  ResponseNotRecordedError,
} from "@goodfetch/collie";
import { TestEngine } from "@goodfetch/collie/testing";
import {
  failure,
  recordingLogger,
  scriptedEngineEnv,
  startScriptedDaemon,
  taskStartUrl,
  test,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";

const COVERS_LIVE = [
  "story:11-158",
  "story:11-159",
  "story:11-160",
  "story:11-161",
  "story:11-162",
  "operation:recording.save_as",
] as const;

const COVERS_DAEMON = ["story:11-161", "story:11-162", "operation:recording.save_as"] as const;

function scriptWithRecordingPaths(folder: string, script: string, work: string): string {
  const source = new URL(
    `../../../../test-support/fixtures/engine-scripts/${script}`,
    import.meta.url,
  );
  const inputPath = resolve(join(work, "input.warc"));
  const savedPath = resolve(join(work, "saved.warc"));
  const viewPath = resolve(join(work, "saved-view.warc"));
  const autoSavedPath = resolve(join(work, "auto-saved.warc"));
  const autoClosedPath = resolve(join(work, "auto-closed.warc"));
  const text = readFileSync(source, "utf8")
    .replaceAll(JSON.stringify("/work/out/input.warc"), JSON.stringify(inputPath))
    .replaceAll(JSON.stringify("/work/out/saved.warc"), JSON.stringify(savedPath))
    .replaceAll(JSON.stringify("/work/out/saved-view.warc"), JSON.stringify(viewPath))
    .replaceAll(JSON.stringify("/work/out/auto-saved.warc"), JSON.stringify(autoSavedPath))
    .replaceAll(JSON.stringify("/work/out/auto-closed.warc"), JSON.stringify(autoClosedPath));
  const file = join(folder, "recording-scripts", script);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, text);
  return file;
}

describe("Response recordings (Scripted engine)", () => {
  test(
    "uploads replay bytes and streams saveRecording on an Attached engine",
    { tags: ["client_case/response-recordings-daemon", "attached", ...COVERS_DAEMON] },
    async ({ testEnv, clientCase }) => {
      const work = await mkdtemp(join(tmpdir(), "gf-rec-ts-"));
      try {
        const inputWarc = resolve(join(work, "input.warc"));
        await writeFile(inputWarc, "WARC/1.1\r\n");
        const scriptPath = scriptWithRecordingPaths(
          testEnv.folder,
          "run/response-recordings-daemon.json",
          work,
        );
        const env = scriptedEngineEnv(testEnv, "collie", scriptPath);
        const { logger } = recordingLogger();
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078).
        await using collie = await Collie.connect({ env, engine: "attached", logger });

        const missingError = await failure(
          collie.start({ recording: { mode: "replay", path: join(work, "missing.warc") } }),
        );
        await clientCase.check("missing replay file raises recording_invalid", () =>
          expect(missingError).toBeInstanceOf(RecordingInvalidError),
        );

        await using run = await collie.start({
          recording: { mode: "replay", path: inputWarc },
        });
        const savedPath = resolve(join(work, "saved.warc"));
        const saved = await run.saveRecording(savedPath);
        await clientCase.check("Run.saveRecording streams bytes atomically", async () => {
          expect(saved.written).toBe(false);
          expect(await readFile(savedPath, "utf8")).toBe("WARC/1.1\r\n");
        });

        await using view = await collie.openRun(run.id());
        const viewPath = resolve(join(work, "saved-view.warc"));
        const savedView = await view.saveRecording(viewPath);
        await clientCase.check("RunView.saveRecording streams bytes atomically", async () => {
          expect(savedView.written).toBe(false);
          expect(await readFile(viewPath, "utf8")).toBe("WARC/1.1\r\n");
        });

        await run.finish();
      } finally {
        await rm(work, { recursive: true, force: true });
      }
    },
  );

  test(
    "uploads replay bytes and streams saveRecording on a Daemon",
    { tags: ["client_case/response-recordings-daemon", "daemon", ...COVERS_DAEMON] },
    async ({ testEnv, processes, clientCase }) => {
      const work = await mkdtemp(join(tmpdir(), "gf-rec-ts-d-"));
      const scriptPath = scriptWithRecordingPaths(
        testEnv.folder,
        "run/response-recordings-daemon-from-a-daemon.json",
        work,
      );
      const daemon = await startScriptedDaemon(testEnv, processes, scriptPath, "socket");
      try {
        const inputWarc = resolve(join(work, "input.warc"));
        await writeFile(inputWarc, "WARC/1.1\r\n");
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078).
        await using collie = await Collie.connect({ env: testEnv.env, engine: "daemon" });

        await using run = await collie.start({
          recording: { mode: "replay", path: inputWarc },
        });
        const savedPath = resolve(join(work, "saved.warc"));
        const saved = await run.saveRecording(savedPath);
        await clientCase.check("Daemon Run.saveRecording streams bytes atomically", async () => {
          expect(saved.written).toBe(false);
          expect(await readFile(savedPath, "utf8")).toBe("WARC/1.1\r\n");
        });

        await using view = await collie.openRun(run.id());
        const viewPath = resolve(join(work, "saved-view.warc"));
        const savedView = await view.saveRecording(viewPath);
        await clientCase.check(
          "Daemon RunView.saveRecording streams bytes atomically",
          async () => {
            expect(savedView.written).toBe(false);
            expect(await readFile(viewPath, "utf8")).toBe("WARC/1.1\r\n");
          },
        );

        await run.finish();

        // Daemon mode: "record" automatically streams and saves atomically on finish and on close.
        const autoSavedPath = resolve(join(work, "auto-saved.warc"));
        const recordRun = await collie.start({
          recording: { mode: "record", path: autoSavedPath },
        });
        await recordRun.finish();
        await recordRun.close();
        await clientCase.check(
          "Daemon mode: record auto-saves atomically on finish without re-saving on close",
          async () => {
            expect(await readFile(autoSavedPath, "utf8")).toBe("WARC/1.1\r\n");
          },
        );

        const autoClosedPath = resolve(join(work, "auto-closed.warc"));
        const closedRun = await collie.start({
          recording: { mode: "record", path: autoClosedPath },
        });
        await closedRun.close();
        await clientCase.check("Daemon mode: record auto-saves atomically on close", async () => {
          expect(await readFile(autoClosedPath, "utf8")).toBe("WARC/1.1\r\n");
        });

        await collie.close();
        const exit = await daemon.exit;
        await clientCase.check("the Daemon played its script through", () =>
          expect(exit).toMatchObject({ code: 0 }),
        );
      } finally {
        await rm(work, { recursive: true, force: true });
      }
    },
  );
});

describe("Response recordings (Proving ground + TestEngine)", () => {
  test.override("provingGroundSeed", 1291);
  test.override("withBrowser", true);

  test(
    "records, replays offline, defaults TestEngine to empty replay, resumes, and saves again after failed write",
    { tags: ["client_case/response-recordings-live", "attached", ...COVERS_LIVE] },
    async ({ env, provingGround, clientCase }) => {
      const work = await mkdtemp(join(tmpdir(), "gf-rec-live-ts-"));
      try {
        const pageUrl = await taskStartUrl(provingGround, "catalog/1");
        const otherUrl = await taskStartUrl(provingGround, "catalog/2");

        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078).
        await using collie = await Collie.connect({ env, engine: "attached" });

        // 1. Record against Proving ground page.
        const warcPath = join(work, "recorded.warc");
        await using recRun = await collie.start({
          recording: { mode: "record", path: warcPath },
        });
        const recSession = await recRun.newSession();
        const recPage = await recSession.newPage();
        const liveResp = await recPage.goto(pageUrl);
        expect(liveResp?.status()).toBe(200);
        const liveTitle = (await recPage.evaluate("document.title"))?.value;
        const recReport = await recRun.finish();
        await clientCase.check("record mode writes WARC and populates report", () =>
          expect(recReport.recording?.mode).toBe("record"),
        );

        // 2. Replay with network off (network.resolve cleared).
        await using replayRun = await collie.start({
          settings: { network: { resolve: {} } },
          recording: { mode: "replay", path: warcPath },
        });
        const rSession = await replayRun.newSession();
        const rPage = await rSession.newPage();
        const replayed = await rPage.goto(pageUrl);
        expect(replayed?.status()).toBe(200);
        expect((await rPage.evaluate("document.title"))?.value).toBe(liveTitle);

        const missErr = await failure(rPage.goto(otherUrl));
        await clientCase.check("unrecorded request raises response_not_recorded", () => {
          expect(missErr).toBeInstanceOf(ResponseNotRecordedError);
          expect(missErr).toMatchObject({ retryable: false });
        });
        const replayReport = await replayRun.finish();
        await clientCase.check("replay report lists responses used and misses", () => {
          expect(replayReport.recording?.mode).toBe("replay");
          expect((replayReport.recording?.responsesUsed ?? 0) >= 1).toBe(true);
          expect((replayReport.recording?.misses?.length ?? 0) >= 1).toBe(true);
        });

        // 3. TestEngine replays an empty recording by default.
        await using testEngine = await TestEngine.start({ env });
        await using teRun = await testEngine.connection.start();
        const teSession = await teRun.newSession();
        const tePage = await teSession.newPage();
        const teErr = await failure(tePage.goto("https://example.invalid/"));
        await clientCase.check("TestEngine replays empty recording by default", () =>
          expect(teErr).toBeInstanceOf(ResponseNotRecordedError),
        );
        await teRun.finish();

        // 4. Resume names the same file and writes both halves.
        const resumeWarc = join(work, "resumed.warc");
        const resumeId = "rec-resume-ts-1";
        await using half1 = await collie.start({
          runId: resumeId,
          recording: { mode: "record", path: resumeWarc },
        });
        const s1 = await half1.newSession();
        const p1 = await s1.newPage();
        await p1.goto(pageUrl);
        await half1.stop();

        await using half2 = await collie.start({
          runId: resumeId,
          recording: { mode: "record", path: resumeWarc },
        });
        const s2 = await half2.newSession();
        const p2 = await s2.newPage();
        await p2.goto(otherUrl);
        await half2.finish();

        await using bothRun = await collie.start({
          settings: { network: { resolve: {} } },
          recording: { mode: "replay", path: resumeWarc },
        });
        const bs = await bothRun.newSession();
        const bp = await bs.newPage();
        expect((await bp.goto(pageUrl))?.status()).toBe(200);
        expect((await bp.goto(otherUrl))?.status()).toBe(200);
        const bothReport = await bothRun.finish();
        await clientCase.check("resumed recording replays both halves with empty misses", () => {
          const nonFaviconMisses = (bothReport.recording?.misses ?? []).filter(
            (miss) => !miss.endsWith("/favicon.ico"),
          );
          expect(nonFaviconMisses).toEqual([]);
        });

        // 5. Failed write at Run end raises path_not_writable; saveRecording writes again.
        const badDir = join(work, "unwritable-dir");
        await mkdir(badDir);
        await using failRun = await collie.start({
          recording: { mode: "record", path: badDir },
        });
        const fs = await failRun.newSession();
        const fp = await fs.newPage();
        await fp.goto(pageUrl);
        const writeErr = await failure(failRun.finish());
        await clientCase.check("failed write at Run end raises path_not_writable", () =>
          expect(writeErr).toBeInstanceOf(PathNotWritableError),
        );

        // Existing file is left untouched when writing to a read-only parent folder fails.
        const roDir = join(work, "readonly-dir");
        await mkdir(roDir);
        const existingWarc = join(roDir, "existing.warc");
        await writeFile(existingWarc, "sentinel-existing-warc");
        await chmod(roDir, 0o555);
        try {
          await using roRun = await collie.start({
            recording: { mode: "record", path: existingWarc },
          });
          const rs = await roRun.newSession();
          const rp = await rs.newPage();
          await rp.goto(pageUrl);
          const roErr = await failure(roRun.finish());
          await clientCase.check("failed write leaves existing target file untouched", async () => {
            expect(roErr).toBeInstanceOf(PathNotWritableError);
            expect(await readFile(existingWarc, "utf8")).toBe("sentinel-existing-warc");
          });
        } finally {
          await chmod(roDir, 0o755);
        }

        const recovered = join(work, "recovered.warc");
        const saved = await failRun.saveRecording(recovered);
        await using view = await collie.openRun(failRun.id());
        const recoveredGz = join(work, "recovered.warc.gz");
        const savedGz = await view.saveRecording(recoveredGz);
        await clientCase.check(
          "saveRecording recovers committed records on Run and RunView",
          () => {
            expect(saved.size).toBeGreaterThan(0);
            expect(savedGz.size).toBeGreaterThan(0);
          },
        );

        // Empty recording raises recording_invalid.
        await using emptyRun = await collie.start();
        const emptyErr = await failure(emptyRun.saveRecording(join(work, "empty.warc")));
        await clientCase.check("saveRecording on empty Run raises recording_invalid", () =>
          expect(emptyErr).toBeInstanceOf(RecordingInvalidError),
        );
        await emptyRun.finish();

        // 6. Invalid / unreadable / unsupported-version WARC raises recording_invalid.
        const garbagePath = join(work, "garbage.warc");
        await writeFile(garbagePath, "not a warc");
        const garbageErr = await failure(
          collie.start({ recording: { mode: "replay", path: garbagePath } }),
        );
        const badVerPath = join(work, "bad-ver.warc");
        await writeFile(
          badVerPath,
          "WARC/9.9\r\nWARC-Type: warcinfo\r\nContent-Length: 0\r\n\r\n\r\n\r\n",
        );
        const badVerErr = await failure(
          collie.start({ recording: { mode: "replay", path: badVerPath } }),
        );
        await clientCase.check(
          "invalid or unsupported-version WARC raises recording_invalid",
          () => {
            expect(garbageErr).toBeInstanceOf(RecordingInvalidError);
            expect(badVerErr).toBeInstanceOf(RecordingInvalidError);
          },
        );
      } finally {
        await rm(work, { recursive: true, force: true });
      }
    },
  );
});
