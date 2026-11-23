// The testing kit's isolations (ADR 0105): the test engine's settings and
// environment. Rust's unit tests of `goodfetch-collie-client`'s and
// `goodfetch-collie`'s `testing` modules and Python's `test_testing_kit.py`
// are their twins.
import { join } from "node:path";
import { describe, expect, test } from "vitest";
import type { Settings } from "./generated/settings.ts";
import { engineEnvironment, isolated } from "./test-engine.ts";

/** Collie's engine-path variable. */
const ENGINE_PATH = "GOODFETCH_COLLIE_ENGINE_PATH";

/** The kit's five durations, in milliseconds. */
const SHORT_TIMEOUTS = {
  action: 5_000,
  navigation: 5_000,
  wait: 5_000,
  firstByte: 5_000,
  close: 5_000,
};

describe("the test engine's settings", () => {
  test("allow and reach only the hosts resolve maps to loopback alone", () => {
    const settings = isolated({
      network: {
        resolve: {
          "app.test": ["127.0.0.1"],
          "api.test": ["::1"],
          "mapped.test": ["::ffff:127.0.0.1"],
          "mixed.test": ["127.0.0.1", "192.0.2.1"],
          "public.test": ["192.0.2.7"],
          "empty.test": [],
        },
      },
    });

    const loopback = ["api.test", "app.test", "mapped.test"];
    expect(settings.egress).toEqual({
      offline: true,
      allowedDomains: loopback,
      privateRanges: loopback,
    });
  });

  test("allow no host with no resolve, and make the five durations short", () => {
    expect(isolated({})).toEqual({
      egress: { offline: true, allowedDomains: [], privateRanges: [] },
      timeout: SHORT_TIMEOUTS,
    });
  });

  test("take a test's own key in place of the kit's, its settings left unchanged", () => {
    const given: Settings = {
      egress: { offline: false, allowedDomains: ["*"] },
      timeout: { navigation: 60_000 },
    };

    const settings = isolated(given);

    expect(settings).toEqual({
      egress: { offline: false, allowedDomains: ["*"], privateRanges: [] },
      timeout: { ...SHORT_TIMEOUTS, navigation: 60_000 },
    });
    expect(given).toEqual({
      egress: { offline: false, allowedDomains: ["*"] },
      timeout: { navigation: 60_000 },
    });
  });
});

describe("the test engine's environment", () => {
  test("holds its home and the caller's browsers and engine alone", () => {
    const caller = {
      GOODFETCH_HOME: "/caller/home",
      [ENGINE_PATH]: "/opt/goodfetch-collie",
      GOODFETCH_URL: "ws://127.0.0.1:1",
      PATH: "/usr/bin",
    };

    expect(engineEnvironment(caller, "/tmp/test-home", ENGINE_PATH)).toEqual({
      GOODFETCH_HOME: "/tmp/test-home",
      GOODFETCH_BROWSERS_PATH: join("/caller/home", "browsers"),
      [ENGINE_PATH]: "/opt/goodfetch-collie",
    });
  });

  test("names the caller's GOODFETCH_BROWSERS_PATH in place of its home's cache", () => {
    const caller = {
      GOODFETCH_HOME: "/caller/home",
      GOODFETCH_BROWSERS_PATH: "/prepared/browsers",
    };

    expect(engineEnvironment(caller, "/tmp/test-home", ENGINE_PATH)).toEqual({
      GOODFETCH_HOME: "/tmp/test-home",
      GOODFETCH_BROWSERS_PATH: "/prepared/browsers",
    });
  });
});
