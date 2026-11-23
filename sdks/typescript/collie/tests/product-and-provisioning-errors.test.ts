// What a Collie engine lacks and a browser build it cannot fetch (ADRs 0087,
// 0090, 0095, 0098 and 0115), on the Scripted engine as a Collie engine: a
// Foxhound operation or setting raises `NotInProductError`, naming both
// Products and Foxhound's install command on the engine's channel, and a
// page whose build is missing raises the provisioning code's class, naming
// the build, the Product version that pins it and its `browser install`.
// Played attached, the engine installed from npm, and as a Daemon installed
// by the archive installer, whose script exits 0 only once every step
// played. Then on Collie's executable of the same commit, attached, on an
// empty Browser cache whose `browser.mirror` is the Proving ground's
// `browser-builds.test`, never the internet (#3100): the first page
// downloads the pinned build and opens, and a cache the engine cannot
// write, `egress.offline` or a host that answers 404 raises the
// provisioning code's class, while `connect()` succeeds; the cache the
// engine cannot write is the `browser-cache-not-writable` case's, on Unix.
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
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
  taskStartUrl,
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

/** The Proving ground's seed for the first-use tests. */
const FIRST_USE_SEED = 3100;

/** The case each first-use test plays, and its mode. */
const CASE = ["client_case/product-and-provisioning-errors", "attached"] as const;

/** The Proving ground's host that serves the pinned builds, the tests' `browser.mirror`. */
const BUILDS_HOST = "browser-builds.test";

/** Collie's pins, which name the Chrome for Testing build it pins. */
const PINS = join(
  import.meta.dirname,
  "..",
  "..",
  "..",
  "..",
  "crates",
  "goodfetch-collie",
  "src",
  "pins.json",
);

/** The Chrome for Testing build Collie pins, from its pins. */
function pinnedBuild(): string {
  const pins: unknown = JSON.parse(readFileSync(PINS, "utf8"));
  if (
    typeof pins === "object" &&
    pins !== null &&
    "chrome_for_testing" in pins &&
    typeof pins.chrome_for_testing === "object" &&
    pins.chrome_for_testing !== null &&
    "build" in pins.chrome_for_testing &&
    typeof pins.chrome_for_testing.build === "string"
  ) {
    return pins.chrome_for_testing.build;
  }
  throw new Error(`${PINS} names no Chrome for Testing build`);
}

/** The Chrome for Testing build Collie pins. */
const PINNED_BUILD = pinnedBuild();

/** The pinned build, as the real engine's provisioning errors name it. */
const REAL_BUILD = { kind: "browser", name: "chrome-for-testing", build: PINNED_BUILD };

/** The workspace's manifest, whose version the real engine's Product has. */
const WORKSPACE_MANIFEST = join(import.meta.dirname, "..", "..", "..", "..", "Cargo.toml");

/** The real engine's Product version, the workspace's `[workspace.package]` version. */
function engineVersion(): string {
  const manifest = readFileSync(WORKSPACE_MANIFEST, "utf8");
  const version = /^\[workspace\.package\]\r?\nversion = "([^"]+)"/mu.exec(manifest)?.[1];
  if (version === undefined) {
    throw new Error(`${WORKSPACE_MANIFEST} names no [workspace.package] version`);
  }
  return version;
}

/** The real engine's Product version, which pins the build. */
const ENGINE_VERSION = engineVersion();

/** The Product version that pins the build, as the real engine's errors name it. */
const REAL_PINNED_BY = { product: "collie", product_version: ENGINE_VERSION };

/** The build and what pins it, as each provisioning error's message names them. */
const DESCRIBED = `the browser build chrome-for-testing ${PINNED_BUILD}, pinned by Collie ${ENGINE_VERSION}`;

/** The empty Browser cache `goodfetch-test-env --browser-download` points the engine at. */
function cacheOf(env: Readonly<Record<string, string>>): string {
  const cache = env["GOODFETCH_BROWSERS_PATH"];
  if (cache === undefined) {
    throw new Error("the test's environment names no GOODFETCH_BROWSERS_PATH");
  }
  return cache;
}

/** What the first page of a Run started with `settings` raises, once `connect()` succeeded. */
async function firstPageFailure(
  env: Readonly<Record<string, string>>,
  settings?: Parameters<Collie["start"]>[0],
): Promise<unknown> {
  // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
  await using collie = await Collie.connect({ env });
  const run = await collie.start(settings);
  const failed = await failure(run.newPage());
  await run.finish();
  return failed;
}

describe(
  "A Collie Connection's first page on an empty Browser cache, on the real engine",
  {
    tags: ["story:11-13", "story:11-294", "setting:browser.mirror"],
  },
  () => {
    test.override("provingGroundSeed", FIRST_USE_SEED);

    describe("from a mirror that serves the pinned build", () => {
      test.override("browserDownload", "served");

      test(
        "downloads the pinned build into the cache and opens the page",
        { tags: [...CASE] },
        async ({ env, provingGround, clientCase }) => {
          const start = await taskStartUrl(provingGround, "statuses/1");
          // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
          await using collie = await Collie.connect({ env });
          const run = await collie.start();
          const page = await run.newPage();
          const navigated = await page.goto(start);
          await run.finish();

          await clientCase.check("the first page opened on the downloaded build", () =>
            expect(navigated?.status()).toBe(200),
          );
          await clientCase.check("the build is in the cache GOODFETCH_BROWSERS_PATH names", () =>
            expect(
              existsSync(
                join(cacheOf(env), "browser", "chrome-for-testing", PINNED_BUILD, "entry.json"),
              ),
            ).toBe(true),
          );
        },
      );
    });

    describe("from a cache the engine cannot write", () => {
      test.override("browserDownload", "read-only-cache");

      test(
        "raises BrowserNotInstalledError, not retryable, naming the build, what pins it and its browser install",
        {
          tags: [
            "client_case/browser-cache-not-writable",
            "attached",
            "error:browser_not_installed",
          ],
        },
        async ({ env, clientCase }) => {
          const refused = await firstPageFailure(env);
          const cache = cacheOf(env);
          const fix =
            "Run `goodfetch-collie browser install` where the Browser cache is writable, such as in the image's build, or point GOODFETCH_BROWSERS_PATH at a cache that holds it";

          await clientCase.check("the real engine raises BrowserNotInstalledError, whole", () => {
            expect(refused).toBeInstanceOf(BrowserNotInstalledError);
            expect(refused).toBeInstanceOf(UnavailableError);
            expect(refused).toMatchObject({
              code: "browser_not_installed",
              group: "unavailable",
              retryable: false,
              message: `${DESCRIBED} is not in the Browser cache at ${cache}, which this engine cannot write`,
              fix,
            });
            expect(refused).toHaveProperty("details", {
              missing: { kind: "pinned", artifact: REAL_BUILD, pinned_by: REAL_PINNED_BY, cache },
            });
          });
        },
      );
    });

    describe("under egress.offline", () => {
      // A mirror it never reaches: the refusal comes before any download.
      test.override("browserDownload", "not-found");

      test(
        "raises ArtifactUnavailableOfflineError, before any download",
        { tags: [...CASE, "error:artifact_unavailable_offline", "setting:egress.offline"] },
        async ({ env, clientCase }) => {
          const refused = await firstPageFailure(env, { settings: { egress: { offline: true } } });
          const cache = cacheOf(env);
          const fix =
            "Run `goodfetch-collie browser install` beforehand where the Browser cache is writable, or lift `egress.offline` where it was set";

          await clientCase.check(
            "the real engine raises ArtifactUnavailableOfflineError, whole",
            () => {
              expect(refused).toBeInstanceOf(ArtifactUnavailableOfflineError);
              expect(refused).toBeInstanceOf(RefusedError);
              expect(refused).toMatchObject({
                code: "artifact_unavailable_offline",
                group: "refused",
                retryable: false,
                message: `${DESCRIBED} is not in the Browser cache at ${cache}, and \`egress.offline\` refuses its download`,
                fix,
                limit: { key: "egress.offline", value: true, layer: "run" },
              });
              expect(refused).toHaveProperty("details", {
                artifact: REAL_BUILD,
                pinned_by: REAL_PINNED_BY,
                cache,
              });
            },
          );
        },
      );
    });

    describe("from a mirror that answers 404", () => {
      test.override("browserDownload", "not-found");

      test(
        "raises ArtifactDownloadFailedError, retryable, naming the host and its status",
        { tags: [...CASE, "error:artifact_download_failed"] },
        async ({ env, clientCase }) => {
          const refused = await firstPageFailure(env);

          await clientCase.check(
            "the real engine raises ArtifactDownloadFailedError, whole",
            () => {
              expect(refused).toBeInstanceOf(ArtifactDownloadFailedError);
              expect(refused).toBeInstanceOf(UnavailableError);
              expect(refused).toMatchObject({
                code: "artifact_download_failed",
                group: "unavailable",
                retryable: true,
                message: `downloading ${DESCRIBED} from ${BUILDS_HOST} failed: ${BUILDS_HOST} answered 404`,
                fix: DOWNLOAD_FIX,
              });
              expect(refused).toHaveProperty("details", {
                artifact: REAL_BUILD,
                pinned_by: REAL_PINNED_BY,
                host: BUILDS_HOST,
                failure: "http_status",
                status: 404,
              });
            },
          );
        },
      );
    });
  },
);
