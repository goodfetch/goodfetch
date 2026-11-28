// Runtime benchmark campaigns and switchable control cost reporting
// (ADRs 0002, 0004, 0016, 0023, 0025, 0042, 0047, 0048, 0094; issue 188).
import type { Product } from "./evaluations.ts";

/** Every switchable control whose cost is published (ADRs 0023, 0042, 0047, 0048). */
export const SWITCHABLE_CONTROLS = [
  "actionability_guard",
  "filter_list_blocking",
  "live_hiding",
  "consent_prompt_handling",
  "tls_relay_cert_exceptions",
  "politeness_respect_robots",
  "power_cut_safe_run_state",
  "fair_per_domain_admission",
] as const;

export type SwitchableControl = (typeof SWITCHABLE_CONTROLS)[number];

/** Measured comparison for one switchable control (on vs off). */
export interface ControlMeasurementInput {
  readonly control: SwitchableControl;
  readonly product: Product;
  readonly on_ms: number;
  readonly off_ms: number;
  /** Server-side observed response bytes from the Proving ground (#159), when applicable. */
  readonly server_bytes_on?: number;
  readonly server_bytes_off?: number;
  /** Baseline label (e.g. `live_hiding = off`). */
  readonly baseline_setting: string;
}

/** Published cost and savings row for one switchable control. */
export interface ControlCostRow {
  readonly control: SwitchableControl;
  readonly product: Product;
  readonly baseline_setting: string;
  /** Positive means control adds latency; negative means control saves time. */
  readonly time_delta_ms: number;
  readonly time_delta_percent: number;
  /** Positive means control saves bytes (server_bytes_off - server_bytes_on); null when N/A. */
  readonly server_bytes_saved: number | null;
  readonly server_bytes_saved_percent: number | null;
}

/** Computes time cost/savings and Proving ground server-side byte savings for a control. */
export function evaluateControlCost(input: ControlMeasurementInput): ControlCostRow {
  const timeDeltaMs = input.on_ms - input.off_ms;
  const timeDeltaPercent = input.off_ms === 0 ? 0 : (timeDeltaMs / input.off_ms) * 100;

  let bytesSaved: number | null = null;
  let bytesSavedPercent: number | null = null;
  if (input.server_bytes_on !== undefined && input.server_bytes_off !== undefined) {
    bytesSaved = input.server_bytes_off - input.server_bytes_on;
    bytesSavedPercent =
      input.server_bytes_off === 0 ? 0 : (bytesSaved / input.server_bytes_off) * 100;
  }

  return {
    control: input.control,
    product: input.product,
    baseline_setting: input.baseline_setting,
    time_delta_ms: timeDeltaMs,
    time_delta_percent: timeDeltaPercent,
    server_bytes_saved: bytesSaved,
    server_bytes_saved_percent: bytesSavedPercent,
  };
}

/** Architectural decision that a runtime measurement could reopen (ADR 0025). */
export type ReopenableDecision = "ADR 0002" | "ADR 0004";

/** Reported comparison for run-state (ADR 0002) or fair per-domain admission (ADR 0004). */
export interface ReopenDecisionReport {
  readonly family: "run-state" | "fair-per-domain-admission";
  readonly product: Product;
  readonly could_reopen: ReopenableDecision;
  readonly reopen_condition: string;
  readonly measured_gap_percent: number;
  readonly threshold_percent: number;
  readonly triggers_reopen_review: boolean;
}

/** Evaluates whether a run-state or admission gap crosses its reopen review threshold. */
export function evaluateReopenDecision(input: {
  readonly family: "run-state" | "fair-per-domain-admission";
  readonly product: Product;
  readonly measured_gap_percent: number;
  readonly threshold_percent: number;
}): ReopenDecisionReport {
  const isRunState = input.family === "run-state";
  return {
    family: input.family,
    product: input.product,
    could_reopen: isRunState ? "ADR 0002" : "ADR 0004",
    reopen_condition: isRunState
      ? "large gap between power-cut-safe and fast run state reopens separate-log option (ADR 0002)"
      : "measurable slowdown on single-domain crawls reopens fair per-domain admission (ADR 0004)",
    measured_gap_percent: input.measured_gap_percent,
    threshold_percent: input.threshold_percent,
    triggers_reopen_review: input.measured_gap_percent > input.threshold_percent,
  };
}
