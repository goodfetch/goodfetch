// Host preflight, carried over from the lab's `host` tests: a class is granted
// only when every gate of it passes, a gate it cannot measure fails, and the
// agent app driving the run is not counted as foreign load.
import { describe, expect, test } from "vitest";
import {
  assessHost,
  blockNoisy,
  classifyProcesses,
  loadHostClasses,
  type HostSnapshot,
} from "./host.ts";

const quietMac: HostSnapshot = {
  os: "darwin",
  arch: "arm64",
  cpu: "Apple M3 Max",
  logical_cpus: 14,
  memory_bytes: 36e9,
  load_average_1m: 1.2,
  memory_free_percent: 60,
  thermal_warning: false,
  on_ac: true,
  low_power_mode: false,
  foreign_cpu_percent: 8,
  controller_cpu_percent: 0,
  foreign_top: [],
  cpu_governor: null,
  virtualized: false,
};
// The host the lab recorded during its runtime-http-backends campaign: load
// average 25 on 14 CPUs, with ffmpeg running.
const loadedMac: HostSnapshot = {
  ...quietMac,
  load_average_1m: 25.14,
  memory_free_percent: 34,
  foreign_cpu_percent: 310,
  foreign_top: [{ command: "ffmpeg", cpu_percent: 280 }],
};
const classes = loadHostClasses();

describe("assessHost", () => {
  test("grants local-controlled to a quiet, clean macOS host", () => {
    const assessment = assessHost({
      snapshot: quietMac,
      requested: "local-controlled",
      allowDowngrade: false,
      gitClean: true,
      classes,
    });
    expect(assessment.refused).toBe(false);
    expect(assessment.class_granted).toBe("local-controlled");
  });

  test("refuses a loaded host for local-controlled, or downgrades it when allowed", () => {
    const refused = assessHost({
      snapshot: loadedMac,
      requested: "local-controlled",
      allowDowngrade: false,
      gitClean: true,
      classes,
    });
    expect(refused.refused).toBe(true);
    expect(refused.class_granted).toBeNull();
    const failed = refused.gates
      .filter((gate) => gate.class === "local-controlled" && !gate.ok)
      .map((gate) => gate.id)
      .toSorted();
    expect(failed).toEqual(["foreign_cpu_percent", "load_per_cpu_1m"]);

    const downgraded = assessHost({
      snapshot: loadedMac,
      requested: "local-controlled",
      allowDowngrade: true,
      gitClean: true,
      classes,
    });
    expect(downgraded.refused).toBe(false);
    expect(downgraded.class_granted).toBe("local-dev");
  });

  test("never grants the reference class to a macOS host", () => {
    const assessment = assessHost({
      snapshot: quietMac,
      requested: "reference",
      allowDowngrade: true,
      gitClean: true,
      classes,
    });
    expect(assessment.class_granted).toBe("local-controlled");
    expect(
      assessment.gates.some((gate) => gate.class === "reference" && gate.id === "os" && !gate.ok),
    ).toBe(true);
  });

  test("caps any host at local-dev while the sources are uncommitted", () => {
    const assessment = assessHost({
      snapshot: quietMac,
      requested: "local-controlled",
      allowDowngrade: true,
      gitClean: false,
      classes,
    });
    expect(assessment.class_granted).toBe("local-dev");
  });

  test("fails a gate it cannot measure instead of passing it", () => {
    const assessment = assessHost({
      snapshot: { ...quietMac, memory_free_percent: null },
      requested: "local-controlled",
      allowDowngrade: false,
      gitClean: true,
      classes,
    });
    expect(assessment.refused).toBe(true);
    expect(assessment.gates).toContainEqual(
      expect.objectContaining({ id: "memory_free_percent", ok: false, value: null }),
    );
  });
});

test("reports the agent app that drives the run apart from foreign CPU", () => {
  const rows = [
    {
      pid: 10,
      ppid: 1,
      pgid: 10,
      cpu: 20,
      command:
        "/Applications/Claude.app/Contents/Frameworks/Claude Helper.app/Contents/MacOS/Claude Helper",
    },
    { pid: 11, ppid: 1, pgid: 11, cpu: 7, command: "Claude Helper (Renderer)" },
    { pid: 12, ppid: 1, pgid: 12, cpu: 3, command: "/Users/x/.local/bin/claude" },
    {
      pid: 13,
      ppid: 1,
      pgid: 13,
      cpu: 75,
      command: "/System/Library/Frameworks/CoreServices.framework/mds_stores",
    },
    { pid: 14, ppid: 1, pgid: 14, cpu: 40, command: "node" },
    { pid: 15, ppid: 14, pgid: 14, cpu: 90, command: "chrome" },
  ];
  const split = classifyProcesses(rows, [14]);
  expect(split.controller_cpu_percent).toBe(30);
  expect(split.foreign_cpu_percent).toBe(75);
  expect(split.foreign_top.map((row) => row.command)).toEqual(["mds_stores"]);
});

test("judges a measured block on foreign CPU, not on the load average its own run raises", () => {
  const gates = [
    { id: "load_per_cpu_1m", max: 0.35 },
    { id: "foreign_cpu_percent", max: 150 },
  ] as const;
  // The run's own load may push the load average past 0.35.
  expect(blockNoisy({ gates, foreignCpuPercent: 120 })).toBe(false);
  expect(blockNoisy({ gates, foreignCpuPercent: 160 })).toBe(true);
  // An unmeasured block is not assumed quiet.
  expect(blockNoisy({ gates, foreignCpuPercent: null })).toBe(true);
});
