// Browser setup as typed `SessionSettings` where a Session opens (ADRs 0083,
// 0085 and 0121; issue 1305):
// - Scripted engine (attached and daemon): all 12 browser setup keys serialize
//   in their exact wire shape (`session-browser-setup-settings`).
// - Proving ground (attached): viewport, device scale factor, locale, time
//   zone, colour scheme, permissions and network throttle are observed on a
//   real Proving ground page (`session-browser-setup-observed-on-a-page`).
// - No `BrowserContext` exists in `@goodfetch/collie`'s public exports.
import * as collieExports from "@goodfetch/collie";
import { Collie, type SessionSettings } from "@goodfetch/collie";
import {
  failure,
  recordingLogger,
  scriptedEngineEnv,
  startScriptedDaemon,
  taskStartUrl,
  test,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";

const ALL_TWELVE_SETTINGS: SessionSettings = {
  browser: {
    viewport: { width: 800, height: 600 },
    deviceScaleFactor: 1.5,
    locale: "fr-FR",
    timezoneId: "Europe/Paris",
    colorScheme: "dark",
    javaScriptEnabled: false,
    geolocation: { latitude: 48.8566, longitude: 2.3522, accuracy: 10.5 },
    permissions: ["geolocation"],
    headless: true,
    blockResources: ["image"],
    serviceWorkers: "block",
  },
  network: {
    throttle: {
      offline: false,
      latency: 40,
      downloadBytesPerSecond: 500_000,
      uploadBytesPerSecond: 250_000,
    },
  },
};

const COVERS_ALL = [
  "story:11-89",
  "setting:browser.viewport",
  "setting:browser.device_scale_factor",
  "setting:browser.locale",
  "setting:browser.timezone_id",
  "setting:browser.color_scheme",
  "setting:browser.java_script_enabled",
  "setting:browser.geolocation",
  "setting:browser.permissions",
  "setting:browser.headless",
  "setting:browser.block_resources",
  "setting:browser.service_workers",
  "setting:network.throttle",
] as const;

describe("Browser setup as typed SessionSettings (Scripted engine)", () => {
  test(
    "sends all 12 browser setup keys on an Attached engine",
    { tags: ["client_case/session-browser-setup-settings", "attached", ...COVERS_ALL] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/session-browser-setup-settings.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078).
      const collie = await Collie.connect({ env, logger });
      const run = await collie.start();
      const session = await run.newSession({ settings: ALL_TWELVE_SETTINGS });
      await session.close();
      await run.finish();
      await collie.close();

      await clientCase.check("all 12 browser setup keys matched the script wire shape", () =>
        expect(warned).toEqual([]),
      );
      await clientCase.check("no BrowserContext exists in the SDK's public items", () =>
        expect("BrowserContext" in collieExports).toBe(false),
      );
    },
  );

  test(
    "sends all 12 browser setup keys against a Daemon",
    { tags: ["client_case/session-browser-setup-settings", "daemon", ...COVERS_ALL] },
    async ({ testEnv, processes, clientCase }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "run/session-browser-setup-settings-from-a-daemon.json",
        "socket",
      );
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078).
      const collie = await Collie.connect({ env: testEnv.env, engine: "daemon" });
      const run = await collie.start();
      const session = await run.newSession({ settings: ALL_TWELVE_SETTINGS });
      await session.close();
      await run.finish();
      await collie.close();
      const exit = await daemon.exit;

      await clientCase.check("the Daemon played its script through", () =>
        expect(exit).toMatchObject({ code: 0 }),
      );
      await clientCase.check("no BrowserContext exists in the SDK's public items", () =>
        expect("BrowserContext" in collieExports).toBe(false),
      );
    },
  );
});

describe("Browser setup observed on a Proving ground page", () => {
  test.override("provingGroundSeed", 1305);
  test.override("withBrowser", true);

  test(
    "observes viewport, scale factor, locale, time zone, colour scheme, permissions and throttle",
    {
      tags: [
        "client_case/session-browser-setup-observed-on-a-page",
        "attached",
        "story:11-89",
        "setting:browser.viewport",
        "setting:browser.device_scale_factor",
        "setting:browser.locale",
        "setting:browser.timezone_id",
        "setting:browser.color_scheme",
        "setting:browser.geolocation",
        "setting:browser.permissions",
        "setting:browser.headless",
        "setting:browser.block_resources",
        "setting:browser.service_workers",
        "setting:network.throttle",
      ],
    },
    async ({ env, provingGround, clientCase }) => {
      const product = await taskStartUrl(provingGround, "catalog/2");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078).
      await using collie = await Collie.connect({ env });
      const run = await collie.start();
      const session = await run.newSession({
        settings: {
          browser: {
            viewport: { width: 800, height: 600 },
            deviceScaleFactor: 2,
            locale: "fr-FR",
            timezoneId: "Europe/Paris",
            colorScheme: "dark",
            geolocation: { latitude: 48.8566, longitude: 2.3522, accuracy: 10 },
            permissions: ["geolocation"],
            headless: true,
            blockResources: ["image"],
            serviceWorkers: "block",
          },
          network: {
            throttle: {
              offline: false,
              latency: 40,
              downloadBytesPerSecond: 500_000,
              uploadBytesPerSecond: 250_000,
            },
          },
        },
      });
      const page = await session.newPage();
      await page.goto(product);

      const observed = await page.evaluate(
        `async () => ({
          width: window.innerWidth,
          height: window.innerHeight,
          dpr: window.devicePixelRatio,
          locale: navigator.language,
          timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
          dark: window.matchMedia("(prefers-color-scheme: dark)").matches,
          geoPermission: (await navigator.permissions.query({ name: "geolocation" })).state,
        })`,
      );

      const offlineSession = await run.newSession({
        settings: { network: { throttle: { offline: true, latency: 0 } } },
      });
      const offlinePage = await offlineSession.newPage();
      const offlineError = await failure(offlinePage.goto(product));

      await offlineSession.close();
      await session.close();
      await run.finish();

      await clientCase.check(
        "viewport, scale factor, locale, time zone, colour scheme and permissions are observed",
        () =>
          expect(observed).toEqual({
            width: 800,
            height: 600,
            dpr: 2,
            locale: "fr-FR",
            timeZone: "Europe/Paris",
            dark: true,
            geoPermission: "granted",
          }),
      );
      await clientCase.check("network.throttle offline prevents navigation", () =>
        expect(offlineError).toBeInstanceOf(Error),
      );
      await clientCase.check("no BrowserContext exists in the SDK's public items", () =>
        expect("BrowserContext" in collieExports).toBe(false),
      );
    },
  );
});
