// The metrics a run records, the probe each reads on each platform, and the
// rule that a metric the run cannot measure is `null` with its reason, never
// 0 and never an approximation.
import { existsSync } from "node:fs";

/** What a metric is read from: the harness and the adapter protocol everywhere, `ps`, or a Linux file. */
export type Probe = "harness" | "adapter" | "ps" | "proc_stat" | "smaps_rollup";

/** One metric of the catalogue. */
export interface MetricSpec {
  readonly unit: string;
  readonly direction: "higher" | "lower";
  readonly description: string;
  /** The probe on each platform; a platform it is missing from has none. */
  readonly probes: Readonly<Partial<Record<NodeJS.Platform, Probe>>>;
}

const everywhere = (probe: Probe): MetricSpec["probes"] => ({ darwin: probe, linux: probe });

/** A metric of the catalogue. */
export type MetricId =
  | "verified_tasks_per_second"
  | "task_p50_ms_verified"
  | "task_p95_ms_verified"
  | "startup_ms"
  | "cpu_ms_per_verified_task"
  | "pss_peak_bytes"
  | "rss_peak_bytes"
  | "cdp_commands_per_task";

/** The metrics a run records for each cell. */
export const metricCatalog: Readonly<Record<MetricId, MetricSpec>> = {
  verified_tasks_per_second: {
    unit: "task/s",
    direction: "higher",
    description:
      "Measured attempts the oracle verified per second of the measured window, rejected attempts kept in the window",
    probes: everywhere("harness"),
  },
  task_p50_ms_verified: {
    unit: "ms",
    direction: "lower",
    description: "Median duration of the verified measured attempts, the oracle's time excluded",
    probes: everywhere("harness"),
  },
  task_p95_ms_verified: {
    unit: "ms",
    direction: "lower",
    description: "95th percentile (nearest rank) of the verified measured attempts' durations",
    probes: everywhere("harness"),
  },
  startup_ms: {
    unit: "ms",
    direction: "lower",
    description:
      "Median time from starting the adapter to its answer to `prepare`, apart from work",
    probes: everywhere("harness"),
  },
  cpu_ms_per_verified_task: {
    unit: "ms/task",
    direction: "lower",
    description:
      "User plus system CPU of the adapter's process tree over every phase, per verified attempt",
    probes: { linux: "proc_stat" },
  },
  pss_peak_bytes: {
    unit: "B",
    direction: "lower",
    description: "Peak of the summed Pss_Anon plus SwapPss over the adapter's process tree",
    probes: { linux: "smaps_rollup" },
  },
  rss_peak_bytes: {
    unit: "B",
    direction: "lower",
    description:
      "Peak of the summed RSS over the adapter's process tree; shared pages counted several times, diagnostic only",
    probes: everywhere("ps"),
  },
  cdp_commands_per_task: {
    unit: "command/task",
    direction: "lower",
    description: "Median CDP commands the adapter reports for one verified measured attempt",
    probes: everywhere("adapter"),
  },
};

/** Whether this host can measure a metric, and why not. */
export type Availability =
  | { readonly available: true }
  | { readonly available: false; readonly reason: string };

/** A metric's figure, or `null` with the reason it has none. */
export type Measure =
  | { readonly value: number; readonly unit: string; readonly source: string }
  | { readonly value: null; readonly unit: string; readonly reason: string };

/** What a caller read for a metric: a value and where it came from, or the reason there is none. */
export type Reading =
  | { readonly value: number; readonly source: string }
  | { readonly value: null; readonly reason: string };

/** The metrics as one host can measure them. */
export interface MetricBook {
  /** Each metric's availability by its ID, as the run record writes it. */
  readonly availability: Readonly<Record<string, Availability>>;
  /** The metric's figure from `reading`, or `null` with the reason it is unavailable or missing. */
  measure(id: MetricId, reading: Reading): Measure;
}

/** The probes this host has: the harness's, the adapter's and `ps` always, Linux's files when present. */
export function detectProbes(platform: NodeJS.Platform): ReadonlySet<Probe> {
  const probes = new Set<Probe>(["harness", "adapter", "ps"]);
  if (platform === "linux" && existsSync("/proc/self/stat")) {
    probes.add("proc_stat");
  }
  if (platform === "linux" && existsSync("/proc/self/smaps_rollup")) {
    probes.add("smaps_rollup");
  }
  return probes;
}

function availabilityOf(
  spec: MetricSpec,
  platform: NodeJS.Platform,
  probes: ReadonlySet<Probe>,
): Availability {
  const probe = spec.probes[platform];
  if (probe === undefined) {
    return { available: false, reason: `no probe on ${platform}` };
  }
  if (!probes.has(probe)) {
    return { available: false, reason: `probe ${probe} is not available on this host` };
  }
  return { available: true };
}

/** The catalogue's metrics as a host with `probes` on `platform` can measure them. */
export function metricBook(input: {
  readonly platform: NodeJS.Platform;
  readonly probes: ReadonlySet<Probe>;
}): MetricBook {
  const resolved = (id: MetricId): Availability =>
    availabilityOf(metricCatalog[id], input.platform, input.probes);
  const ids = Object.keys(metricCatalog).filter((key): key is MetricId => key in metricCatalog);
  return {
    availability: Object.fromEntries(ids.map((id) => [id, resolved(id)])),
    measure(id, reading) {
      const { unit } = metricCatalog[id];
      const state = resolved(id);
      if (!state.available) {
        return { value: null, unit, reason: state.reason };
      }
      if (reading.value === null) {
        return { value: null, unit, reason: reading.reason };
      }
      if (!Number.isFinite(reading.value)) {
        return { value: null, unit, reason: "not a finite number" };
      }
      return { value: reading.value, unit, source: reading.source };
    },
  };
}
