// Host preflight, carried over from the lab's `host` tests: a class is granted
// only when every gate of it passes, a gate it cannot measure fails, and the
// agent app driving the run is not counted as foreign load.
import { describe, expect, test } from "vitest";
import {
  assessHost,
  blockNoisy,
  calibrateNetworkProfile,
  classifyForPublication,
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
  turbo_enabled: null,
  smt_enabled: null,
};

const quietReferenceLinux: HostSnapshot = {
  os: "linux",
  arch: "x64",
  cpu: "AMD EPYC 9454P",
  logical_cpus: 32,
  memory_bytes: 128e9,
  load_average_1m: 0.8,
  memory_free_percent: 80,
  thermal_warning: false,
  on_ac: true,
  low_power_mode: false,
  foreign_cpu_percent: 2,
  controller_cpu_percent: 0,
  foreign_top: [],
  cpu_governor: "performance",
  virtualized: false,
  turbo_enabled: false,
  smt_enabled: true,
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

describe("reference host class gates and publication classification", () => {
  test("grants reference to a clean bare-metal Linux host with turbo off and SMT on", () => {
    const assessment = assessHost({
      snapshot: quietReferenceLinux,
      requested: "reference",
      allowDowngrade: false,
      gitClean: true,
      classes,
    });
    expect(assessment.refused).toBe(false);
    expect(assessment.class_granted).toBe("reference");

    const pub = classifyForPublication({
      assessment,
      machineLabel: "reference-bare-metal (Ubuntu 24.04)",
    });
    expect(pub).toEqual({
      publishable: true,
      publish_as: "reference",
      class_granted: "reference",
      machine_label: "reference-bare-metal (Ubuntu 24.04)",
      compare_scope: "reference-series",
    });
  });

  test.each([
    { name: "dirty git", patch: {}, gitClean: false, gate: "git_clean" },
    { name: "non-linux os", patch: { os: "darwin" }, gitClean: true, gate: "os" },
    { name: "virtualized", patch: { virtualized: true }, gitClean: true, gate: "virtualized" },
    {
      name: "wrong cpu governor",
      patch: { cpu_governor: "powersave" },
      gitClean: true,
      gate: "cpu_governor",
    },
    {
      name: "turbo enabled",
      patch: { turbo_enabled: true },
      gitClean: true,
      gate: "turbo_enabled",
    },
    { name: "smt disabled", patch: { smt_enabled: false }, gitClean: true, gate: "smt_enabled" },
    { name: "not on ac", patch: { on_ac: false }, gitClean: true, gate: "on_ac" },
    {
      name: "high load per cpu",
      patch: { load_average_1m: 10 },
      gitClean: true,
      gate: "load_per_cpu_1m",
    },
    {
      name: "low free memory",
      patch: { memory_free_percent: 15 },
      gitClean: true,
      gate: "memory_free_percent",
    },
    {
      name: "thermal warning",
      patch: { thermal_warning: true },
      gitClean: true,
      gate: "thermal_warning",
    },
    {
      name: "foreign cpu too high",
      patch: { foreign_cpu_percent: 25 },
      gitClean: true,
      gate: "foreign_cpu_percent",
    },
  ])("refuses reference host when $name fails $gate", ({ patch, gitClean, gate }) => {
    const assessment = assessHost({
      snapshot: { ...quietReferenceLinux, ...patch },
      requested: "reference",
      allowDowngrade: false,
      gitClean,
      classes,
    });
    expect(assessment.refused).toBe(true);
    expect(assessment.class_granted).toBeNull();
    expect(
      assessment.gates.some((one) => one.class === "reference" && one.id === gate && !one.ok),
    ).toBe(true);
    expect(classifyForPublication({ assessment, machineLabel: "ref-test" }).publishable).toBe(
      false,
    );
  });

  test("downgraded host carries its own class and is never published as reference", () => {
    const assessment = assessHost({
      snapshot: { ...quietReferenceLinux, turbo_enabled: true },
      requested: "reference",
      allowDowngrade: true,
      gitClean: true,
      classes,
    });
    expect(assessment.refused).toBe(false);
    expect(assessment.class_granted).toBe("local-controlled");

    const pub = classifyForPublication({
      assessment,
      machineLabel: "hetzner-ccx53",
    });
    expect(pub).toEqual({
      publishable: true,
      publish_as: "non-reference-labelled",
      class_granted: "local-controlled",
      machine_label: "hetzner-ccx53",
      compare_scope: "within-report-only",
      downgraded_from_reference: true,
    });
  });

  test("calibrates tc netem profiles and invalidates time metrics when outside tolerance", () => {
    const ok = calibrateNetworkProfile({
      mechanism: "tc_netem",
      declaredRttMs: 100,
      declaredLossPercent: 0,
      measuredRttMs: 103,
      toleranceMs: 5,
    });
    expect(ok.within_tolerance).toBe(true);
    expect(ok.time_metrics_valid).toBe(true);
    expect(ok.correctness_metrics_valid).toBe(true);

    const drifted = calibrateNetworkProfile({
      mechanism: "tc_netem",
      declaredRttMs: 100,
      declaredLossPercent: 1,
      measuredRttMs: 125,
      toleranceMs: 5,
    });
    expect(drifted.within_tolerance).toBe(false);
    expect(drifted.time_metrics_valid).toBe(false);
    expect(drifted.correctness_metrics_valid).toBe(true);
  });
});
