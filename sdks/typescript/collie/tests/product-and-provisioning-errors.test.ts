// What a Collie engine lacks and a browser build it cannot fetch (ADRs 0087,
// 0090, 0095, 0098 and 0115), on the Scripted engine as a Collie engine: a
// Foxhound operation or setting raises `NotInProductError`, naming both
// Products and Foxhound's install command on the engine's channel, and a
// page whose build is missing raises the provisioning code's class, naming
// the build, the Product version that pins it and its `browser install`.
import {
  ArtifactDownloadFailedError,
  ArtifactUnavailableOfflineError,
  BrowserNotInstalledError,
  Collie,
  NotInProductError,
  RefusedError,
  UnavailableError,
} from "@goodfetch/collie";
import { failure, recordingLogger, scriptedEngineEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** Foxhound's install command on the engine's channel, npm. */
const INSTALL = "npm install @goodfetch/foxhound";

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

describe("A Collie Connection on the Scripted engine as a Collie engine", () => {
  test(
    "raises NotInProductError for Foxhound's operation and setting, and each missing build's class",
    {
      tags: [
        "client_case/product-and-provisioning-errors",
        "attached",
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
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/product-and-provisioning-errors.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });

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
      await run.finish();
      await collie.close();

      await clientCase.check(
        "a Foxhound operation raises NotInProductError, an UnavailableError naming both Products and Foxhound's install command",
        () => {
          expect(operation).toBeInstanceOf(NotInProductError);
          expect(operation).toBeInstanceOf(UnavailableError);
          expect(operation).toMatchObject({
            code: "not_in_product",
            group: "unavailable",
            retryable: false,
            fix: `Run it with Foxhound, which contains everything Collie does (\`${INSTALL}\`), or connect to a Foxhound engine.`,
            details: {
              product: "collie",
              product_version: "0.0.0",
              needs: "foxhound",
              requested: "crawl.start",
              install: INSTALL,
            },
          });
        },
      );
      await clientCase.check("a Foxhound setting raises NotInProductError naming its key", () => {
        expect(setting).toBeInstanceOf(NotInProductError);
        expect(setting).toMatchObject({
          retryable: false,
          details: { needs: "foxhound", requested: "crawl.max_requests", install: INSTALL },
        });
      });
      await clientCase.check(
        "a build absent from a cache the engine cannot write raises BrowserNotInstalledError, not retryable, naming the build, what pins it and its browser install",
        () => {
          expect(notInstalled).toBeInstanceOf(BrowserNotInstalledError);
          expect(notInstalled).toBeInstanceOf(UnavailableError);
          expect(notInstalled).toMatchObject({
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
          expect(offline).toBeInstanceOf(ArtifactUnavailableOfflineError);
          expect(offline).toBeInstanceOf(RefusedError);
          expect(offline).toMatchObject({
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
          expect(downloadFailed).toBeInstanceOf(ArtifactDownloadFailedError);
          expect(downloadFailed).toBeInstanceOf(UnavailableError);
          expect(downloadFailed).toMatchObject({
            code: "artifact_download_failed",
            group: "unavailable",
            retryable: true,
            fix: "Retry, or run `goodfetch-collie browser install` beforehand, or point GOODFETCH_BROWSERS_PATH at a prepared Browser cache",
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
      await clientCase.check("the script played to its end, so its engine wrote no failure", () =>
        expect(warned).toEqual([]),
      );
    },
  );
});
