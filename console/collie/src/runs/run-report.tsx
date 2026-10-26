// A run's Run report on its page (spec #591, story 15; ADRs 0029, 0041,
// 0102, 0105, 0106, 0107, 0116 and 0122), rendered as the engine produced
// it and never recomputed: the three axes with the reasons the run is not
// `verified` or not `complete`, its stop reason (a `resources` stop naming
// memory or storage and its folder), its Usage against its Limits, its
// Model allowance apart from its Limits, its Plan report, its warnings with
// their codes, its uncertain Effects, the settings that weakened a
// safeguard, its policy version, its settings with their layers, its
// Plugins, its last Checkpoint, its start and end, why nothing was done,
// the error its ending raised and, when it replayed a Response recording,
// that mark with the recording's mode, file, responses used, fetch times
// and misses. Each section is a typed part of the report, so a part the
// report does not carry reads as absent, never as zero. Every Error code
// the report names reads through the Console's message catalog, falling
// back to the engine's English message.
import type {
  AxisReason,
  ModelAllowance,
  NothingDone,
  PlanStatus,
  RecordingReport,
  RunReport,
  StopReason,
  UncertainEffect,
  WeakenedSetting,
} from "@goodfetch/collie-client/generated";
import type { MessageDescriptor } from "@lingui/core";
import { msg } from "@lingui/core/macro";
import { Trans, useLingui } from "@lingui/react/macro";
import type { ReactNode } from "react";
import { EffectiveSettings } from "#engine-settings/effective-settings.tsx";
import { EndingErrorMessage } from "#errors/error-message.tsx";
import { catalogSummary } from "#errors/errors.ts";
import { formatAmount, formatCount, formatInstant } from "#format.ts";
import { LAYERS, RUN_ORIGINS } from "#settings-layers.ts";
import { RunUsageView } from "#usage/run-usage.tsx";
import { COMPLETENESS_NAMES, LIFECYCLE_NAMES, VERDICT_NAMES } from "./names.ts";

const sectionClassName = "flex flex-col gap-3 rounded-lg border p-4";
const listClassName = "grid grid-cols-[max-content_1fr] gap-x-6 gap-y-2";

const PLAN_STATUS_NAMES: Readonly<Record<PlanStatus, MessageDescriptor>> = {
  unchanged: msg`unchanged: nothing changed`,
  promoted: msg`promoted: its new versions were written to the Plan file`,
  pending_review: msg`pending review: its new versions wait for a person`,
  frozen: msg`frozen: the Plan file is frozen`,
};

/** Each kind of a Model allowance (ADR 0107); a kind a newer engine names reads as it came. */
const ALLOWANCE_KIND_NAMES: Readonly<Record<string, MessageDescriptor>> = {
  price: msg`Price`,
  tokens: msg`Tokens`,
  calls: msg`Calls`,
};

/** A point in time, with its machine form beside it. */
function Instant({ value }: { readonly value: string }): ReactNode {
  return (
    <time dateTime={value} translate="no">
      {formatInstant(value)}
    </time>
  );
}

/** A count, with its machine form beside it. */
function Count({ value }: { readonly value: number }): ReactNode {
  return (
    <data value={String(value)} translate="no">
      {formatCount(value)}
    </data>
  );
}

/** One term of a description list and its value. */
function Fact({
  term,
  children,
}: {
  readonly term: ReactNode;
  readonly children: ReactNode;
}): ReactNode {
  return (
    <>
      <dt className="font-medium">{term}</dt>
      <dd>{children}</dd>
    </>
  );
}

/** A section of the report, under its own heading. */
function Part({
  id,
  title,
  children,
}: {
  readonly id: string;
  readonly title: ReactNode;
  readonly children: ReactNode;
}): ReactNode {
  return (
    <section aria-labelledby={id} className={sectionClassName}>
      <h3 id={id} className="text-lg font-semibold">
        {title}
      </h3>
      {children}
    </section>
  );
}

/** What a part the report does not carry shows. */
function Absent({ children }: { readonly children: ReactNode }): ReactNode {
  return <p className="text-muted-foreground">{children}</p>;
}

/** A JSON value as text. */
function json(value: unknown): string {
  return JSON.stringify(value) ?? "null";
}

/** The field `name` of `value` when it is text. */
function textOf(value: object, name: string): string | undefined {
  const found = Object.entries(value).find(([key]) => key === name)?.[1];
  return typeof found === "string" ? found : undefined;
}

/** A reason's code and message, with its count when it counts some. */
function Reasons({
  label,
  reasons,
}: {
  readonly label: string;
  readonly reasons: readonly AxisReason[] | undefined;
}): ReactNode {
  if (reasons === undefined || reasons.length === 0) {
    return null;
  }
  return (
    <ul aria-label={label} className="ms-4 list-disc">
      {reasons.map((reason, index) => (
        <li key={`${reason.code} ${String(index)}`}>
          <code translate="no">{reason.code}</code>: <span translate="no">{reason.message}</span>
          {reason.count === undefined || reason.count === null ? null : (
            <>
              {" "}
              <Trans>
                (<Count value={reason.count} />)
              </Trans>
            </>
          )}
        </li>
      ))}
    </ul>
  );
}

/** The run's three axes, with the reasons it is not `verified` or not `complete` (ADR 0029). */
function Axes({ report }: { readonly report: RunReport }): ReactNode {
  const { i18n, t } = useLingui();
  return (
    <>
      <dl className={listClassName}>
        <Fact term={<Trans>Ended as</Trans>}>{i18n._(LIFECYCLE_NAMES[report.lifecycle])}</Fact>
        <Fact term={<Trans>Its Verdict</Trans>}>{i18n._(VERDICT_NAMES[report.verdict])}</Fact>
        <Fact term={<Trans>Its Completeness</Trans>}>
          {i18n._(COMPLETENESS_NAMES[report.completeness])}
        </Fact>
      </dl>
      <Reasons label={t`Why it is not verified`} reasons={report.verdict_reasons} />
      <Reasons label={t`Why it is not complete`} reasons={report.completeness_reasons} />
    </>
  );
}

/** Why the run stopped (ADRs 0029, 0041, 0080, 0102 and 0122). */
function Stop({ reason }: { readonly reason: StopReason }): ReactNode {
  switch (reason.kind) {
    case "abort":
      switch (reason.by) {
        case "client":
          return <Trans>Aborted by a Client.</Trans>;
        case "program":
          return <Trans>Aborted by its program.</Trans>;
        case "browser":
          return <Trans>Aborted by its browser.</Trans>;
        default:
          return reason.by satisfies never;
      }
    case "pause":
      return <Trans>Paused: checkpointed and stopped, to be resumed by its Run ID.</Trans>;
    case "migrate":
      return <Trans>Migrated: checkpointed and stopped, to be resumed on another machine.</Trans>;
    case "timeout":
      return <Trans>Its run timeout passed.</Trans>;
    case "drain":
      return <Trans>Its Daemon drained.</Trans>;
    case "limit":
    case "limit_not_understood": {
      const usageKind = textOf(reason, "usage_kind") ?? "";
      const scope = Object.entries(reason).find(([key]) => key === "scope")?.[1];
      const scopeKind =
        typeof scope === "object" && scope !== null ? (textOf(scope, "kind") ?? "") : "";
      const currency = textOf(reason, "currency");
      const capped = currency === undefined ? usageKind : `${usageKind} (${currency})`;
      return reason.kind === "limit" ? (
        <Trans>
          A Limit on <code translate="no">{scopeKind}</code> capping{" "}
          <code translate="no">{capped}</code> was reached.
        </Trans>
      ) : (
        <Trans>
          A Limit on <code translate="no">{scopeKind}</code> capping{" "}
          <code translate="no">{capped}</code> that this engine cannot evaluate came to cover it.
        </Trans>
      );
    }
    case "engine_lost":
      return <Trans>Its engine was lost while it ran.</Trans>;
    case "resources": {
      const folder = reason.folder;
      if (reason.resource === "memory") {
        return <Trans>The engine ran out of memory for it.</Trans>;
      }
      return folder === undefined || folder === null ? (
        <Trans>The engine ran out of storage for it.</Trans>
      ) : (
        <Trans>
          The engine ran out of storage for it: the filesystem of{" "}
          <code translate="no">{folder}</code> filled.
        </Trans>
      );
    }
    default:
      return reason satisfies never;
  }
}

/** A Model allowance's amount: a price in its currency, else a count. */
function AllowanceAmount({
  allowance,
  amount,
}: {
  readonly allowance: ModelAllowance;
  readonly amount: number;
}): ReactNode {
  return (
    <data value={String(amount)} translate="no">
      {formatAmount(amount, allowance.currency)}
    </data>
  );
}

/** The run's Model allowance, apart from its Limits (ADR 0107). */
function Allowance({ allowances }: { readonly allowances: readonly ModelAllowance[] }): ReactNode {
  const { i18n, t } = useLingui();
  if (allowances.length === 0) {
    return (
      <Absent>
        <Trans>No Model allowance is set for this run.</Trans>
      </Absent>
    );
  }
  return (
    <>
      <p className="text-sm text-muted-foreground">
        <Trans>
          A Model allowance is not a Limit: once reached, the run's model calls are refused while
          the run goes on.
        </Trans>
      </p>
      <ul aria-label={t`Model allowance`} className="flex flex-col gap-2">
        {allowances.map((allowance) => {
          const kind = ALLOWANCE_KIND_NAMES[allowance.kind];
          const { source } = allowance;
          return (
            <li key={allowance.kind} className="rounded-md border p-3">
              <dl className={listClassName}>
                <Fact term={<Trans>Kind</Trans>}>
                  {kind === undefined ? <code translate="no">{allowance.kind}</code> : i18n._(kind)}
                </Fact>
                <Fact term={<Trans>Allows</Trans>}>
                  <AllowanceAmount allowance={allowance} amount={allowance.value} />
                </Fact>
                <Fact term={<Trans>Remains</Trans>}>
                  <AllowanceAmount allowance={allowance} amount={allowance.remains} />
                </Fact>
                <Fact term={<Trans>Reached</Trans>}>
                  {allowance.reached_at === undefined || allowance.reached_at === null ? (
                    <span>
                      <Trans>not reached</Trans>
                    </span>
                  ) : (
                    <Instant value={allowance.reached_at} />
                  )}
                </Fact>
                <Fact term={<Trans>Set by</Trans>}>
                  <Trans>
                    <code translate="no">{source.key}</code> ={" "}
                    <code translate="no">{json(source.value)}</code>, at the{" "}
                    {i18n._(LAYERS[source.layer])} layer
                  </Trans>
                </Fact>
              </dl>
            </li>
          );
        })}
      </ul>
    </>
  );
}

/** What the run did with its Plan file (ADR 0091). */
function Plan({ report }: { readonly report: RunReport }): ReactNode {
  const { i18n, t } = useLingui();
  const { plan } = report;
  if (plan === undefined || plan === null) {
    return (
      <Absent>
        <Trans>The report has no Plan part: the run used no Plan file.</Trans>
      </Absent>
    );
  }
  return (
    <>
      <dl className={listClassName}>
        <Fact term={<Trans>Plan file</Trans>}>
          {plan.file === undefined || plan.file === null ? (
            <span>
              <Trans>none</Trans>
            </span>
          ) : (
            <code translate="no">{plan.file}</code>
          )}
        </Fact>
        <Fact term={<Trans>Version used</Trans>}>
          {plan.version === undefined || plan.version === null ? (
            <span>
              <Trans>none</Trans>
            </span>
          ) : (
            <Count value={plan.version} />
          )}
        </Fact>
        <Fact term={<Trans>New versions</Trans>}>{i18n._(PLAN_STATUS_NAMES[plan.status])}</Fact>
      </dl>
      {plan.steps_changed === undefined || plan.steps_changed.length === 0 ? null : (
        <ul aria-label={t`Steps that changed`} className="ms-4 list-disc">
          {plan.steps_changed.map((step) => (
            <li key={step}>
              <code translate="no">{step}</code>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

/** What the run warns of, each with its code. */
function Warnings({ report }: { readonly report: RunReport }): ReactNode {
  const { i18n, t } = useLingui();
  const warnings = report.warnings ?? [];
  if (warnings.length === 0) {
    return (
      <Absent>
        <Trans>The run warns of nothing.</Trans>
      </Absent>
    );
  }
  return (
    <ul aria-label={t`Warnings`} className="flex flex-col gap-2">
      {warnings.map((warning, index) => (
        <li key={`${warning.code} ${String(index)}`}>
          <p>
            <code translate="no">{warning.code}</code>:{" "}
            {catalogSummary(warning.code, i18n) ?? <span translate="no">{warning.message}</span>}
          </p>
          {warning.folder === undefined || warning.folder === null ? null : (
            <p className="text-sm">
              <Trans>
                Folder: <code translate="no">{warning.folder}</code>
              </Trans>
            </p>
          )}
          {warning.sessions === undefined || warning.sessions.length === 0 ? null : (
            <p className="text-sm">
              <Trans>
                Sessions: <code translate="no">{warning.sessions.join(", ")}</code>
              </Trans>
            </p>
          )}
        </li>
      ))}
    </ul>
  );
}

/** One uncertain Effect: by its key or its instruction, why, its first command and since when (ADR 0106). */
function Effect({ effect }: { readonly effect: UncertainEffect }): ReactNode {
  const named =
    effect.key === undefined || effect.key === null ? (
      effect.instruction === undefined || effect.instruction === null ? (
        <Trans>
          Entry <code translate="no">{effect.entry}</code>
        </Trans>
      ) : (
        <Trans>
          The AI action <q translate="no">{effect.instruction}</q>
        </Trans>
      )
    ) : (
      <Trans>
        The Effect <code translate="no">{effect.key}</code>
      </Trans>
    );
  return (
    <li className="rounded-md border p-3">
      <p>
        {named}{" "}
        <Trans>
          is uncertain (<code translate="no">{effect.reason}</code>) since{" "}
          <Instant value={effect.since} />.
        </Trans>
      </p>
      {effect.first_command === undefined || effect.first_command === null ? null : (
        <p className="text-sm">
          <Trans>
            First command sent: <code translate="no">{effect.first_command}</code>
          </Trans>
        </p>
      )}
      {effect.crawl_request === undefined || effect.crawl_request === null ? null : (
        <p className="text-sm">
          <Trans>
            Crawl request: <code translate="no">{effect.crawl_request}</code>
          </Trans>
        </p>
      )}
    </li>
  );
}

/** The run's uncertain Effects. */
function Effects({ report }: { readonly report: RunReport }): ReactNode {
  const { t } = useLingui();
  const effects = report.uncertain_effects;
  if (effects === undefined || effects === null) {
    return (
      <Absent>
        <Trans>The report has no uncertain Effects part.</Trans>
      </Absent>
    );
  }
  if (effects.length === 0) {
    return (
      <p>
        <Trans>No Effect is uncertain.</Trans>
      </p>
    );
  }
  return (
    <ul aria-label={t`Uncertain Effects`} className="flex flex-col gap-2">
      {effects.map((effect) => (
        <Effect key={effect.entry} effect={effect} />
      ))}
    </ul>
  );
}

/** The layer that set a weakening setting, with its origin for the run layer. */
function layerName(setting: WeakenedSetting): MessageDescriptor {
  return setting.layer === "run" && setting.origin !== undefined && setting.origin !== null
    ? RUN_ORIGINS[setting.origin]
    : LAYERS[setting.layer];
}

/** Every setting that weakened a safeguard for the run (ADR 0028). */
function Weakened({ report }: { readonly report: RunReport }): ReactNode {
  const { i18n, t } = useLingui();
  const weakened = report.weakened ?? [];
  if (weakened.length === 0) {
    return (
      <Absent>
        <Trans>No setting weakened a safeguard for this run.</Trans>
      </Absent>
    );
  }
  return (
    <ul aria-label={t`Settings that weakened a safeguard`} className="flex flex-col gap-1">
      {weakened.map((setting) => {
        const layer = i18n._(layerName(setting));
        return (
          <li key={setting.key}>
            <Trans>
              <code translate="no">{setting.key}</code> ={" "}
              <code translate="no">{json(setting.value)}</code>, set by {layer}
            </Trans>
          </li>
        );
      })}
    </ul>
  );
}

/** The settings the run ran with, with their layers, and its Plugins. */
function Configuration({ report }: { readonly report: RunReport }): ReactNode {
  const { t } = useLingui();
  const { settings, plugins } = report;
  return (
    <>
      <dl className={listClassName}>
        <Fact term={<Trans>Policy version</Trans>}>
          {report.policy_version === undefined || report.policy_version === null ? (
            <span>
              <Trans>no Operator policy applied</Trans>
            </span>
          ) : (
            <code translate="no">{report.policy_version}</code>
          )}
        </Fact>
      </dl>
      <h4 className="font-semibold">
        <Trans>Settings</Trans>
      </h4>
      {settings === undefined || settings === null ? (
        <Absent>
          <Trans>The report has no settings part.</Trans>
        </Absent>
      ) : (
        <EffectiveSettings settings={settings} />
      )}
      <h4 className="font-semibold">
        <Trans>Plugins</Trans>
      </h4>
      {plugins === undefined || plugins === null ? (
        <Absent>
          <Trans>The report has no Plugins part.</Trans>
        </Absent>
      ) : plugins.length === 0 ? (
        <p>
          <Trans>No Plugin ran.</Trans>
        </p>
      ) : (
        <ul aria-label={t`Plugins`} className="flex flex-col gap-1">
          {plugins.map((plugin) => (
            <li key={plugin.name}>
              <Trans>
                <code translate="no">{plugin.name}</code> at the network level{" "}
                <code translate="no">{plugin.network_level}</code>
              </Trans>
              {plugin.confinement_missing === undefined ||
              plugin.confinement_missing.length === 0 ? null : (
                <>
                  {" "}
                  <Trans>
                    without <code translate="no">{plugin.confinement_missing.join(", ")}</code>
                  </Trans>
                </>
              )}
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

/** Why the call that ended the run did nothing (ADR 0102's G1). */
function Nothing({ nothing }: { readonly nothing: NothingDone }): ReactNode {
  switch (nothing.reason) {
    case "already_ended":
      return (
        <Trans>
          Nothing was done: the start found the run ended at <Instant value={nothing.ended_at} />.
        </Trans>
      );
    case "nothing_pending":
      return <Trans>Nothing was done: a resume or a retry found nothing left.</Trans>;
    case "no_seed_handled":
      return (
        <>
          <Trans>Nothing was done: no seed reached a handler.</Trans>
          <ul className="ms-4 list-disc">
            {(nothing.seeds ?? []).map((kept) => (
              <li key={kept.reason}>
                <code translate="no">{kept.reason}</code>: <Count value={kept.count} />
              </li>
            ))}
          </ul>
        </>
      );
    default:
      return nothing satisfies never;
  }
}

/** The mark of a run that replayed a Response recording, so no one reads it as a live run (ADR 0105). */
export function ReplayedMark(): ReactNode {
  return (
    <span className="rounded-sm border border-primary px-1.5 text-sm font-medium">
      <Trans>Replayed</Trans>
    </span>
  );
}

/** What the run did with its Response recording (ADR 0105). */
function Recording({ recording }: { readonly recording: RecordingReport }): ReactNode {
  const { t } = useLingui();
  const misses = recording.misses ?? [];
  return (
    <>
      {recording.mode === "replay" ? (
        <p>
          <ReplayedMark />{" "}
          <Trans>
            This run replayed a Response recording: no request reached a site, so it is no live run.
          </Trans>
        </p>
      ) : null}
      <dl className={listClassName}>
        <Fact term={<Trans>Mode</Trans>}>
          <code translate="no">{recording.mode}</code>
        </Fact>
        <Fact term={<Trans>Recording file</Trans>}>
          <code translate="no">{recording.file}</code>
        </Fact>
        <Fact term={<Trans>Responses used</Trans>}>
          <Count value={recording.responses_used} />
        </Fact>
        <Fact term={<Trans>Fetched</Trans>}>
          {recording.recorded_from === undefined ||
          recording.recorded_from === null ||
          recording.recorded_to === undefined ||
          recording.recorded_to === null ? (
            <span>
              <Trans>unknown</Trans>
            </span>
          ) : (
            <Trans>
              from <Instant value={recording.recorded_from} /> to{" "}
              <Instant value={recording.recorded_to} />
            </Trans>
          )}
        </Fact>
        <Fact term={<Trans>Misses</Trans>}>
          <Count value={misses.length} />
        </Fact>
      </dl>
      {misses.length === 0 ? null : (
        <ul aria-label={t`Requests the recording missed`} className="ms-4 list-disc">
          {misses.map((miss) => (
            <li key={miss}>
              <span translate="no">{miss}</span>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

/** A run's Run report, as the engine produced it. */
export function RunReportView({ report }: { readonly report: RunReport }): ReactNode {
  const { checkpoint, recording, nothing_done: nothing, error } = report;
  return (
    <section aria-labelledby="run-report" className="flex flex-col gap-4">
      <h2 id="run-report" className="text-xl font-semibold">
        <Trans>Run report</Trans>
      </h2>
      <Part id="report-ending" title={<Trans>How it ended</Trans>}>
        <Axes report={report} />
        <dl className={listClassName}>
          <Fact term={<Trans>Stop reason</Trans>}>
            {report.stop_reason === undefined || report.stop_reason === null ? (
              <span>
                <Trans>it did not stop early</Trans>
              </span>
            ) : (
              <Stop reason={report.stop_reason} />
            )}
          </Fact>
          <Fact term={<Trans>Started</Trans>}>
            <Instant value={report.started_at} />
          </Fact>
          <Fact term={<Trans>Ended</Trans>}>
            <Instant value={report.ended_at} />
          </Fact>
          <Fact term={<Trans>Checkpoint</Trans>}>
            {checkpoint === undefined || checkpoint === null ? (
              <span>
                <Trans>none written</Trans>
              </span>
            ) : (
              <Instant value={checkpoint.time} />
            )}
          </Fact>
        </dl>
        {nothing === undefined || nothing === null ? null : (
          <p>
            <Nothing nothing={nothing} />
          </p>
        )}
        {error === undefined || error === null ? null : (
          <div>
            <p className="font-medium">
              <Trans>The error its ending raised:</Trans>
            </p>
            <EndingErrorMessage ending={error} />
          </div>
        )}
      </Part>
      <Part id="report-usage" title={<Trans>Usage against its Limits</Trans>}>
        <RunUsageView usage={report.usage} recording={recording} />
      </Part>
      <Part id="report-allowance" title={<Trans>Model allowance</Trans>}>
        <Allowance allowances={report.usage.model_allowance ?? []} />
      </Part>
      <Part id="report-recording" title={<Trans>Response recording</Trans>}>
        {recording === undefined || recording === null ? (
          <Absent>
            <Trans>The run named no Response recording: it fetched live.</Trans>
          </Absent>
        ) : (
          <Recording recording={recording} />
        )}
      </Part>
      <Part id="report-plan" title={<Trans>Plan report</Trans>}>
        <Plan report={report} />
      </Part>
      <Part id="report-warnings" title={<Trans>Warnings</Trans>}>
        <Warnings report={report} />
      </Part>
      <Part id="report-effects" title={<Trans>Uncertain Effects</Trans>}>
        <Effects report={report} />
      </Part>
      <Part id="report-weakened" title={<Trans>Weakened safeguards</Trans>}>
        <Weakened report={report} />
      </Part>
      <Part id="report-configuration" title={<Trans>Settings and Plugins</Trans>}>
        <Configuration report={report} />
      </Part>
    </section>
  );
}
