import { describe, expect, test } from "vitest";
import { daemonOf } from "./daemon.ts";
import type { TestEnvDescription } from "./test-env.ts";

const DESCRIPTION: TestEnvDescription = {
  folder: "/tmp/a test é x",
  home: "/tmp/a test é x/home",
  env: { GOODFETCH_HOME: "/tmp/a test é x/home", TZ: "UTC" },
  seed: 1,
  start_time: "2026-01-05T09:00:00Z",
  tools: {},
};

describe("daemonOf", () => {
  test("fails when the description names no Daemon", () => {
    expect(() => daemonOf(DESCRIPTION)).toThrow(
      expect.objectContaining({ code: "daemon_not_requested" }),
    );
  });
});
