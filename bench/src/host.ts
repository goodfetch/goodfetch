// Host preflight (ADR 0025): the snapshot of the machine a run is on, and the
// host classes whose gates grant it a class or refuse it. A gate whose value
// cannot be measured fails: a quiet host must be shown, never assumed.
import { execFile } from "node:child_process";
import { readFileSync } from "node:fs";
import { readdir, readFile } from "node:fs/promises";
import { arch, cpus, loadavg, platform, totalmem } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { parserOf } from "./schema.ts";

const exec = promisify(execFile);

/** The host classes, from the least controlled to the most. */
export const hostClasses = ["local-dev", "local-controlled", "reference"] as const;
/** A host class a campaign asks for or a run is granted. */
export type HostClass = (typeof hostClasses)[number];

/** One process using the CPU, as a snapshot names it. */
export interface BusyProcess {
  readonly command: string;
  readonly cpu_percent: number;
}

/** What preflight reads of the host. A value this platform cannot give is `null`. */
export interface HostSnapshot {
  readonly os: string;
  readonly arch: string;
  readonly cpu: string | null;
  readonly logical_cpus: number;
  readonly memory_bytes: number;
  readonly load_average_1m: number;
  readonly memory_free_percent: number | null;
  readonly thermal_warning: boolean | null;
  readonly on_ac: boolean | null;
  readonly low_power_mode: boolean | null;
  /** CPU of every process outside the run's own groups and the agent app driving it. */
  readonly foreign_cpu_percent: number | null;
  readonly foreign_top: readonly BusyProcess[];
  /** CPU of the agent app driving the run: part of the setup, not a foreign load. */
  readonly controller_cpu_percent: number | null;
  readonly cpu_governor: string | null;
  readonly virtualized: boolean | null;
  /** Whether CPU turbo boost is enabled (`false` required for `reference`, ADR 0025). */
  readonly turbo_enabled: boolean | null;
  /** Whether simultaneous multithreading (SMT) is enabled (`true` required for `reference`, ADR 0025). */
  readonly smt_enabled: boolean | null;
}

const gateIds = [
  "git_clean",
  "os",
  "virtualized",
  "cpu_governor",
  "turbo_enabled",
  "smt_enabled",
  "load_per_cpu_1m",
  "memory_free_percent",
  "thermal_warning",
  "on_ac",
  "low_power_mode",
  "foreign_cpu_percent",
] as const;
/** What a gate reads. */
export type GateId = (typeof gateIds)[number];

/** One gate of a host class: a value it must equal, or a bound it must keep. */
export type Gate =
  | { readonly id: GateId; readonly equals: string | boolean }
  | { readonly id: GateId; readonly max: number }
  | { readonly id: GateId; readonly min: number };

/** A host class above `local-dev`, read from `bench/hosts/<class>.json`. */
export interface HostClassSpec {
  readonly class: Exclude<HostClass, "local-dev">;
  readonly description: string;
  readonly gates: readonly Gate[];
}

/** One gate as preflight judged it. */
export interface GateResult {
  readonly class: HostClass;
  readonly id: GateId;
  readonly ok: boolean;
  readonly value: string | number | boolean | null;
  readonly limit: string;
}

/** The class a run is granted, or its refusal, with every gate judged on the way. */
export type HostAssessment = {
  readonly class_requested: HostClass;
  readonly gates: readonly GateResult[];
} & (
  | { readonly refused: false; readonly class_granted: HostClass }
  | { readonly refused: true; readonly class_granted: null }
);

const hostClassFile = parserOf<HostClassSpec>({
  type: "object",
  required: ["class", "description", "gates"],
  additionalProperties: false,
  properties: {
    class: { enum: ["local-controlled", "reference"] },
    description: { type: "string" },
    gates: {
      type: "array",
      items: {
        type: "object",
        required: ["id"],
        additionalProperties: false,
        properties: {
          id: { enum: gateIds },
          equals: { anyOf: [{ type: "string" }, { type: "boolean" }] },
          max: { type: "number" },
          min: { type: "number" },
        },
        oneOf: [{ required: ["equals"] }, { required: ["max"] }, { required: ["min"] }],
      },
    },
  },
});

/** Reads the classes above `local-dev` from `bench/hosts/`. */
export function loadHostClasses(): HostClassSpec[] {
  return (["local-controlled", "reference"] as const).map((name) => {
    const file = new URL(`../hosts/${name}.json`, import.meta.url);
    const spec = hostClassFile.parse(JSON.parse(readFileSync(file, "utf8")), file.pathname);
    if (spec.class !== name) {
      throw new Error(`invariant: ${file.pathname} declares the class ${spec.class}`);
    }
    return spec;
  });
}

function gateValue(id: GateId, snapshot: HostSnapshot, gitClean: boolean): GateResult["value"] {
  switch (id) {
    case "git_clean":
      return gitClean;
    case "os":
      return snapshot.os;
    case "virtualized":
      return snapshot.virtualized;
    case "cpu_governor":
      return snapshot.cpu_governor;
    case "turbo_enabled":
      return snapshot.turbo_enabled;
    case "smt_enabled":
      return snapshot.smt_enabled;
    case "load_per_cpu_1m":
      return snapshot.logical_cpus > 0 ? snapshot.load_average_1m / snapshot.logical_cpus : null;
    case "memory_free_percent":
      return snapshot.memory_free_percent;
    case "thermal_warning":
      return snapshot.thermal_warning;
    case "on_ac":
      return snapshot.on_ac;
    case "low_power_mode":
      return snapshot.low_power_mode;
    case "foreign_cpu_percent":
      return snapshot.foreign_cpu_percent;
    default: {
      const unknownGate: never = id;
      throw new Error(`invariant: unknown gate ${String(unknownGate)}`);
    }
  }
}

function judge(gate: Gate, value: GateResult["value"]): { ok: boolean; limit: string } {
  if ("equals" in gate) {
    return { ok: value !== null && value === gate.equals, limit: `= ${String(gate.equals)}` };
  }
  if ("max" in gate) {
    return { ok: typeof value === "number" && value <= gate.max, limit: `<= ${gate.max}` };
  }
  return { ok: typeof value === "number" && value >= gate.min, limit: `>= ${gate.min}` };
}

function evaluate(spec: HostClassSpec, snapshot: HostSnapshot, gitClean: boolean): GateResult[] {
  return spec.gates.map((gate) => {
    const value = gateValue(gate.id, snapshot, gitClean);
    return { class: spec.class, id: gate.id, value, ...judge(gate, value) };
  });
}

/**
 * Grants the requested class when all its gates pass. Otherwise, with
 * `allowDowngrade`, it tries each class below it, down to `local-dev`, which
 * has no gates; without, it refuses.
 */
export function assessHost(input: {
  readonly snapshot: HostSnapshot;
  readonly requested: HostClass;
  readonly allowDowngrade: boolean;
  readonly gitClean: boolean;
  readonly classes: readonly HostClassSpec[];
}): HostAssessment {
  const { snapshot, requested, allowDowngrade, gitClean, classes } = input;
  const gates: GateResult[] = [];
  const order = hostClasses.slice(0, hostClasses.indexOf(requested) + 1).toReversed();
  for (const candidate of order) {
    if (candidate === "local-dev") {
      break;
    }
    const spec = classes.find((item) => item.class === candidate);
    if (spec === undefined) {
      throw new Error(`invariant: no specification of the host class ${candidate}`);
    }
    const results = evaluate(spec, snapshot, gitClean);
    gates.push(...results);
    if (results.every((result) => result.ok)) {
      return { class_requested: requested, gates, refused: false, class_granted: candidate };
    }
    if (!allowDowngrade) {
      return { class_requested: requested, gates, refused: true, class_granted: null };
    }
  }
  return { class_requested: requested, gates, refused: false, class_granted: "local-dev" };
}

/**
 * Whether a measured block ran on a noisy host: foreign CPU past the granted
 * class's bound, or unmeasured. The load average counts the run's own
 * processes, so it gates the host at rest, before the run, and never a block.
 */
export function blockNoisy(input: {
  readonly gates: readonly Gate[];
  readonly foreignCpuPercent: number | null;
}): boolean {
  const gate = input.gates.find((item) => item.id === "foreign_cpu_percent");
  if (gate === undefined || !("max" in gate)) {
    return false;
  }
  return input.foreignCpuPercent === null || input.foreignCpuPercent > gate.max;
}

/** One row of the process table. */
export interface ProcessRow {
  readonly pid: number;
  readonly ppid: number;
  readonly pgid: number;
  readonly cpu: number;
  readonly command: string;
}

// The agent app that launches a campaign (the Claude desktop app and CLI)
// runs throughout every run it drives.
const controller = /(^|\/)(Claude|Claude Helper( \([A-Za-z]+\))?|claude)$/;

/**
 * Splits the busy processes outside the run's own process groups, and their
 * descendants, into the agent app driving the run and foreign load.
 */
export function classifyProcesses(
  rows: readonly ProcessRow[],
  ownProcessGroups: readonly number[],
): Pick<HostSnapshot, "foreign_cpu_percent" | "controller_cpu_percent" | "foreign_top"> {
  const own = new Set(ownProcessGroups);
  let changed = true;
  while (changed) {
    changed = false;
    for (const row of rows) {
      if ((own.has(row.ppid) || own.has(row.pgid)) && !own.has(row.pid)) {
        own.add(row.pid);
        changed = true;
      }
    }
  }
  const busy = rows.filter((row) => !own.has(row.pid) && Number.isFinite(row.cpu) && row.cpu > 0);
  const foreign = busy
    .filter((row) => !controller.test(row.command))
    .toSorted((left, right) => right.cpu - left.cpu);
  const sum = (selected: readonly ProcessRow[]): number =>
    selected.reduce((total, row) => total + row.cpu, 0);
  return {
    foreign_cpu_percent: sum(foreign),
    controller_cpu_percent: sum(busy.filter((row) => controller.test(row.command))),
    foreign_top: foreign.slice(0, 5).map((row) => ({
      command: row.command.split("/").at(-1) ?? row.command,
      cpu_percent: row.cpu,
    })),
  };
}

async function output(command: string, args: readonly string[]): Promise<string | null> {
  try {
    return (await exec(command, args, { timeout: 3000, maxBuffer: 4_000_000 })).stdout;
  } catch {
    // A tool this host lacks leaves its values unmeasured, and their gates fail.
    return null;
  }
}

async function contents(path: string): Promise<string | null> {
  try {
    return await readFile(path, "utf8");
  } catch {
    // A file this host lacks leaves its value unmeasured.
    return null;
  }
}

function processRows(table: string): ProcessRow[] {
  return table
    .trim()
    .split("\n")
    .map((line) => {
      const [pid, ppid, pgid, cpu, ...command] = line.trim().split(/\s+/);
      return {
        pid: Number(pid),
        ppid: Number(ppid),
        pgid: Number(pgid),
        cpu: Number(cpu),
        command: command.join(" "),
      };
    });
}

type PlatformValues = Pick<
  HostSnapshot,
  | "memory_free_percent"
  | "thermal_warning"
  | "on_ac"
  | "low_power_mode"
  | "cpu_governor"
  | "virtualized"
  | "turbo_enabled"
  | "smt_enabled"
>;

const unmeasured: PlatformValues = {
  memory_free_percent: null,
  thermal_warning: null,
  on_ac: null,
  low_power_mode: null,
  cpu_governor: null,
  virtualized: null,
  turbo_enabled: null,
  smt_enabled: null,
};

async function darwinValues(): Promise<PlatformValues> {
  const pressure = await output("memory_pressure", ["-Q"]);
  const free = pressure?.match(/free percentage:\s*(\d+)%/i);
  const thermal = await output("pmset", ["-g", "therm"]);
  const battery = await output("pmset", ["-g", "batt"]);
  const settings = await output("pmset", ["-g"]);
  const low = settings?.match(/lowpowermode\s+(\d)/);
  const hypervisor = await output("sysctl", ["-n", "kern.hv_vmm_present"]);
  return {
    memory_free_percent: free?.[1] === undefined ? null : Number(free[1]),
    thermal_warning:
      thermal === null
        ? null
        : !/No thermal warning level has been recorded/i.test(thermal) ||
          !/No performance warning level has been recorded/i.test(thermal),
    on_ac: battery === null ? null : /AC Power/.test(battery),
    low_power_mode: low ? low[1] === "1" : settings === null ? null : false,
    cpu_governor: null,
    virtualized: hypervisor === null ? null : hypervisor.trim() === "1",
    turbo_enabled: null,
    smt_enabled: null,
  };
}

async function linuxTurboEnabled(): Promise<boolean | null> {
  const noTurbo = await contents("/sys/devices/system/cpu/intel_pstate/no_turbo");
  if (noTurbo !== null) {
    const trimmed = noTurbo.trim();
    if (trimmed === "1") {
      return false;
    }
    if (trimmed === "0") {
      return true;
    }
  }
  const boost = await contents("/sys/devices/system/cpu/cpufreq/boost");
  if (boost !== null) {
    const trimmed = boost.trim();
    if (trimmed === "0") {
      return false;
    }
    if (trimmed === "1") {
      return true;
    }
  }
  return null;
}

async function linuxSmtEnabled(): Promise<boolean | null> {
  const active = await contents("/sys/devices/system/cpu/smt/active");
  if (active === null) {
    return null;
  }
  const trimmed = active.trim();
  if (trimmed === "1") {
    return true;
  }
  if (trimmed === "0") {
    return false;
  }
  return null;
}

/**
 * Reads Linux `/sys/class/power_supply/*` to determine whether the host is on
 * AC power. Reports `false` when a Mains/AC supply reads `0` or a Battery
 * reports `Discharging`; reports `true` when an AC/Mains supply reads `1`, or
 * on a mains-only server/VM with no power supply entries or batteries.
 */
export async function linuxOnAc(
  powerSupplyDir = "/sys/class/power_supply",
): Promise<boolean | null> {
  let entries: string[];
  try {
    entries = await readdir(powerSupplyDir);
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") {
      // Mains-only servers and VMs often expose no /sys/class/power_supply.
      return true;
    }
    return null;
  }
  if (entries.length === 0) {
    return true;
  }
  let sawMainsOnline = false;
  let sawBattery = false;
  for (const entry of entries) {
    const dir = join(powerSupplyDir, entry);
    const kind = (await contents(join(dir, "type")))?.trim();
    if (kind === "Mains" || kind === "USB" || kind === "AC") {
      const online = (await contents(join(dir, "online")))?.trim();
      if (online === "0") {
        return false;
      }
      if (online === "1") {
        sawMainsOnline = true;
      }
    } else if (kind === "Battery") {
      sawBattery = true;
      const status = (await contents(join(dir, "status")))?.trim();
      if (status === "Discharging") {
        return false;
      }
    }
  }
  if (sawMainsOnline || !sawBattery) {
    return true;
  }
  // Batteries exist and none are discharging, with no explicit mains online file.
  return true;
}

async function linuxValues(): Promise<PlatformValues> {
  const meminfo = await contents("/proc/meminfo");
  const total = meminfo?.match(/MemTotal:\s+(\d+)/)?.[1];
  const available = meminfo?.match(/MemAvailable:\s+(\d+)/)?.[1];
  const governor = await contents("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor");
  const temperature = await contents("/sys/class/thermal/thermal_zone0/temp");
  return {
    memory_free_percent:
      total === undefined || available === undefined
        ? null
        : Math.round((Number(available) * 100) / Number(total)),
    thermal_warning: temperature === null ? null : Number(temperature) > 90_000,
    on_ac: await linuxOnAc(),
    low_power_mode: false,
    cpu_governor: governor?.trim() ?? null,
    virtualized: await linuxVirtualized(),
    turbo_enabled: await linuxTurboEnabled(),
    smt_enabled: await linuxSmtEnabled(),
  };
}

async function linuxVirtualized(): Promise<boolean | null> {
  try {
    const { stdout } = await exec("systemd-detect-virt", [], { timeout: 3000 });
    return stdout.trim() !== "none";
  } catch (error) {
    // It exits 1 on bare metal, after printing `none`; any other failure
    // leaves the value unmeasured.
    const printed =
      error instanceof Error && "stdout" in error && typeof error.stdout === "string"
        ? error.stdout.trim()
        : null;
    return printed === "none" ? false : null;
  }
}

/** Reads the host now; `ownProcessGroups` are the run's, left out of foreign CPU. */
export async function snapshotHost(ownProcessGroups: readonly number[]): Promise<HostSnapshot> {
  const os = platform();
  const table = await output("ps", ["-axo", "pid=,ppid=,pgid=,pcpu=,comm="]);
  const load =
    table === null
      ? { foreign_cpu_percent: null, controller_cpu_percent: null, foreign_top: [] }
      : classifyProcesses(processRows(table), ownProcessGroups);
  const values =
    os === "darwin" ? await darwinValues() : os === "linux" ? await linuxValues() : unmeasured;
  return {
    os,
    arch: arch(),
    cpu: cpus()[0]?.model ?? null,
    logical_cpus: cpus().length,
    memory_bytes: totalmem(),
    load_average_1m: loadavg()[0] ?? 0,
    ...load,
    ...values,
  };
}

/** How a run's host class governs publication (ADR 0025 notes of 2026-11-10). */
export type PublishedHostClassification =
  | {
      readonly publishable: false;
      readonly reason: string;
    }
  | {
      readonly publishable: true;
      readonly publish_as: "reference";
      readonly class_granted: "reference";
      readonly machine_label: string;
      readonly compare_scope: "reference-series";
    }
  | {
      readonly publishable: true;
      readonly publish_as: "non-reference-labelled";
      readonly class_granted: Exclude<HostClass, "reference">;
      readonly machine_label: string;
      readonly compare_scope: "within-report-only";
      readonly downgraded_from_reference: boolean;
    };

/**
 * Classifies a host assessment for publication: only a host that passed every
 * `reference` gate may be published as the `reference` class; any downgraded
 * or non-reference run carries its own class and machine label and is compared
 * within one report only.
 */
export function classifyForPublication(input: {
  readonly assessment: HostAssessment;
  readonly machineLabel: string;
}): PublishedHostClassification {
  const { assessment, machineLabel } = input;
  if (assessment.refused) {
    const failed = assessment.gates
      .filter((gate) => !gate.ok)
      .map((gate) => gate.id)
      .join(", ");
    return {
      publishable: false,
      reason: `host refused for ${assessment.class_requested} (failed gates: ${failed || "unknown"})`,
    };
  }
  if (assessment.class_granted === "reference") {
    return {
      publishable: true,
      publish_as: "reference",
      class_granted: "reference",
      machine_label: machineLabel,
      compare_scope: "reference-series",
    };
  }
  return {
    publishable: true,
    publish_as: "non-reference-labelled",
    class_granted: assessment.class_granted,
    machine_label: machineLabel,
    compare_scope: "within-report-only",
    downgraded_from_reference: assessment.class_requested === "reference",
  };
}

/** Declared network profiles from ADR 0025 (added RTT in ms). */
export const NETWORK_PROFILE_RTT_MS = [0, 20, 100, 300] as const;
export type NetworkProfileRttMs = (typeof NETWORK_PROFILE_RTT_MS)[number];

/** Calibration record for a `tc netem` or user-space latency proxy profile (ADR 0025). */
export interface NetworkProfileCalibration {
  readonly mechanism: "tc_netem" | "userspace_proxy";
  readonly declared_rtt_ms: NetworkProfileRttMs;
  readonly declared_loss_percent: number;
  readonly measured_rtt_ms: number;
  readonly tolerance_ms: number;
  readonly within_tolerance: boolean;
  /** Outside tolerance keeps correctness results but loses time metrics (ADR 0025). */
  readonly time_metrics_valid: boolean;
  readonly correctness_metrics_valid: true;
}

/** Checks a network profile's measured added delay against its declared RTT and tolerance. */
export function calibrateNetworkProfile(input: {
  readonly mechanism: "tc_netem" | "userspace_proxy";
  readonly declaredRttMs: NetworkProfileRttMs;
  readonly declaredLossPercent: number;
  readonly measuredRttMs: number;
  readonly toleranceMs: number;
}): NetworkProfileCalibration {
  const within = Math.abs(input.measuredRttMs - input.declaredRttMs) <= input.toleranceMs;
  return {
    mechanism: input.mechanism,
    declared_rtt_ms: input.declaredRttMs,
    declared_loss_percent: input.declaredLossPercent,
    measured_rtt_ms: input.measuredRttMs,
    tolerance_ms: input.toleranceMs,
    within_tolerance: within,
    time_metrics_valid: within,
    correctness_metrics_valid: true,
  };
}
