// What a Collie engine lacks and a browser build it cannot fetch (ADRs 0087,
// 0090, 0095, 0098 and 0115), on the Scripted engine as a Collie engine: a
// Foxhound operation or setting raises `NotInProductError`, naming both
// Products and Foxhound's install command on the engine's channel, and a
// page whose build is missing raises the provisioning code's class, naming
// the build, the Product version that pins it and its `browser install`.
// Played attached, the engine installed from npm, and as a Daemon installed
// by the archive installer, whose script exits 0 only once every step
// played.
import {
  ArtifactDownloadFailedError,
  ArtifactUnavailableOfflineError,
  BrowserNotInstalledError,
  Collie,
  NotInProductError,
  RefusedError,
  UnavailableError,
} from "@goodfetch/collie";
import {
  type ClientCaseRecord,
  failure,
  recordingLogger,
  scriptedEngineEnv,
  startScriptedDaemon,
  test,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** Foxhound's install command on npm, the Attached engine's channel. */
const NPM_INSTALL = "npm install @goodfetch/foxhound";

/** Foxhound's install command on the archive installer, the Daemon's channel. */
const INSTALLER_INSTALL = "curl -fsSL https://goodfetch.dev/install/foxhound.sh | sh";

/** The Browser cache the engine reads. */
const CACHE = "/srv/goodfetch/browsers";

/** The build every refused page needs. */
const BUILD = {
  kind: "browser",
  name: "chrome-for-testing",
  build: "141.0.7390.54-linux64",
};

/** The Product version that pins it. */
const PINNED_BY = { product: "collie", product_version: "0.0.0" };

/** What a failed download's error says to do, whatever failed. */
const DOWNLOAD_FIX =
  "Retry, or run `goodfetch-collie browser install` beforehand, or point GOODFETCH_BROWSERS_PATH at a prepared Browser cache";

/** What each of the script's refused calls raised. */
interface Refused {
  readonly operation: unknown;
  readonly setting: unknown;
  readonly notInstalled: unknown;
  readonly offline: unknown;
  readonly downloadFailed: unknown;
  readonly connectionBroke: unknown;
  readonly wrongDigest: unknown;
}

/** Plays the case's calls on `collie`, then finishes the Run and closes it. */
async function refusedCalls(collie: Collie): Promise<Refused> {
  const operation = await failure(
    collie.call("crawl.start", { seeds: [{ url: "https://shop.test/", label: "page" }] }),
  );
  const setting = await failure(
    collie.call("run.start", { settings: { crawl: { max_requests: 10 } } }),
  );
  const run = await collie.start();
  const notInstalled = await failure(run.newPage());
  const offline = await failure(run.newPage());
  const downloadFailed = await failure(run.newPage());
  const connectionBroke = await failure(run.newPage());
  const wrongDigest = await failure(run.newPage());
  await run.finish();
  await collie.close();
  return {
    operation,
    setting,
    notInstalled,
    offline,
    downloadFailed,
    connectionBroke,
    wrongDigest,
  };
}

/** Checks what each call raised, Foxhound's install command being `install`. */
async function checkRefused(
  clientCase: ClientCaseRecord,
  refused: Refused,
  install: string,
): Promise<void> {
  await clientCase.check(
    "a Foxhound operation raises NotInProductError, an UnavailableError naming both Products and Foxhound's install command",
    () => {
      expect(refused.operation).toBeInstanceOf(NotInProductError);
      expect(refused.operation).toBeInstanceOf(UnavailableError);
      expect(refused.operation).toMatchObject({
        code: "not_in_product",
        group: "unavailable",
        retryable: false,
        fix: `Run it with Foxhound, which contains everything Collie does (\`${install}\`), or connect to a Foxhound engine.`,
        details: {
          product: "collie",
          product_version: "0.0.0",
          needs: "foxhound",
          requested: "crawl.start",
          install,
        },
      });
    },
  );
  await clientCase.check("a Foxhound setting raises NotInProductError naming its key", () => {
    expect(refused.setting).toBeInstanceOf(NotInProductError);
    expect(refused.setting).toMatchObject({
      retryable: false,
      details: { needs: "foxhound", requested: "crawl.max_requests", install },
    });
  });
  await clientCase.check(
    "a build absent from a cache the engine cannot write raises BrowserNotInstalledError, not retryable, naming the build, what pins it and its browser install",
    () => {
      expect(refused.notInstalled).toBeInstanceOf(BrowserNotInstalledError);
      expect(refused.notInstalled).toBeInstanceOf(UnavailableError);
      expect(refused.notInstalled).toMatchObject({
        code: "browser_not_installed",
        group: "unavailable",
        retryable: false,
        fix: "Run `goodfetch-collie browser install` where the Browser cache is writable, such as in the image's build, or point GOODFETCH_BROWSERS_PATH at a cache that holds it",
        details: {
          missing: { kind: "pinned", artifact: BUILD, pinned_by: PINNED_BY, cache: CACHE },
        },
      });
    },
  );
  await clientCase.check(
    "a download offline refuses raises ArtifactUnavailableOfflineError, a RefusedError whose limit names egress.offline",
    () => {
      expect(refused.offline).toBeInstanceOf(ArtifactUnavailableOfflineError);
      expect(refused.offline).toBeInstanceOf(RefusedError);
      expect(refused.offline).toMatchObject({
        code: "artifact_unavailable_offline",
        group: "refused",
        retryable: false,
        fix: "Run `goodfetch-collie browser install` beforehand where the Browser cache is writable, or lift `egress.offline` where it was set",
        limit: { key: "egress.offline", value: true, layer: "user_configuration" },
        details: { artifact: BUILD, pinned_by: PINNED_BY, cache: CACHE },
      });
    },
  );
  await clientCase.check(
    "a failed download raises ArtifactDownloadFailedError, retryable, naming its host and failure",
    () => {
      expect(refused.downloadFailed).toBeInstanceOf(ArtifactDownloadFailedError);
      expect(refused.downloadFailed).toBeInstanceOf(UnavailableError);
      expect(refused.downloadFailed).toMatchObject({
        code: "artifact_download_failed",
        group: "unavailable",
        retryable: true,
        fix: DOWNLOAD_FIX,
        details: {
          artifact: BUILD,
          pinned_by: PINNED_BY,
          host: "storage.googleapis.com",
          failure: "http_status",
          status: 404,
        },
      });
    },
  );
  await clientCase.check(
    "a download whose connection broke raises ArtifactDownloadFailedError naming the network",
    () => {
      expect(refused.connectionBroke).toBeInstanceOf(ArtifactDownloadFailedError);
      expect(refused.connectionBroke).toMatchObject({
        group: "unavailable",
        retryable: true,
        fix: DOWNLOAD_FIX,
      });
      expect(refused.connectionBroke).toHaveProperty("details", {
        artifact: BUILD,
        pinned_by: PINNED_BY,
        host: "storage.googleapis.com",
        failure: "network",
      });
    },
  );
  await clientCase.check(
    "a download whose bytes have another digest raises ArtifactDownloadFailedError naming the digest mismatch",
    () => {
      expect(refused.wrongDigest).toBeInstanceOf(ArtifactDownloadFailedError);
      expect(refused.wrongDigest).toMatchObject({
        group: "unavailable",
        retryable: true,
        fix: DOWNLOAD_FIX,
      });
      expect(refused.wrongDigest).toHaveProperty("details", {
        artifact: BUILD,
        pinned_by: PINNED_BY,
        host: "storage.googleapis.com",
        failure: "digest_mismatch",
      });
    },
  );
}

describe(
  "A Collie Connection on the Scripted engine as a Collie engine",
  {
    tags: [
      "story:11-10",
      "story:11-13",
      "story:11-189",
      "story:11-294",
      "error:not_in_product",
      "error:browser_not_installed",
      "error:artifact_unavailable_offline",
      "error:artifact_download_failed",
    ],
  },
  () => {
    test(
      "raises NotInProductError for Foxhound's operation and setting, and each missing build's class",
      { tags: ["client_case/product-and-provisioning-errors", "attached"] },
      async ({ testEnv, clientCase }) => {
        const env = scriptedEngineEnv(
          testEnv,
          "collie",
          "run/product-and-provisioning-errors.json",
        );
        const { logger, warned } = recordingLogger();
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        const refused = await refusedCalls(await Collie.connect({ env, logger }));

        await checkRefused(clientCase, refused, NPM_INSTALL);
        await clientCase.check("the script played to its end, so its engine wrote no failure", () =>
          expect(warned).toEqual([]),
        );
      },
    );

    test(
      "raises the same classes from a Daemon, naming Foxhound's install command on the Daemon's channel",
      { tags: ["client_case/product-and-provisioning-errors", "daemon"] },
      async ({ testEnv, processes, clientCase }) => {
        const daemon = await startScriptedDaemon(
          testEnv,
          processes,
          "run/product-and-provisioning-errors-from-the-installer.json",
          "socket",
        );
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        const collie = await Collie.connect({ env: testEnv.env, engine: "daemon" });
        const refused = await refusedCalls(collie);
        const exit = await daemon.exit;

        await checkRefused(clientCase, refused, INSTALLER_INSTALL);
        await clientCase.check(
          "the Daemon played its script through, so the Client sent exactly what it holds",
          () => expect(exit).toMatchObject({ code: 0 }),
        );
      },
    );
  },
);
