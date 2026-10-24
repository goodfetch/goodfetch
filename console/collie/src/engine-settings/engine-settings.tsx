// Engine & settings (spec #591, stories 94 and 102; ADRs 0031 and 0087):
// its first section shows every effective value with its layer and lock,
// as `config explain` does, from `config.explain` on the engine the
// Console reached. A person may explain with a project configuration and
// run values, as the command's `--project` and `--set` do; the Console
// edits neither. Its second section shows Usage against each Limit, by
// scope (story 99). Later sections (the Operator policy, tokens, people and
// the Daemon group) join this screen.
import type { SettingsExplanation } from "@goodfetch/collie-client/generated";
import { Trans, useLingui } from "@lingui/react/macro";
import {
  useEffect,
  useId,
  useState,
  useSyncExternalStore,
  type SubmitEvent,
  type ReactNode,
} from "react";
import type { ConsoleClient } from "#client/client.ts";
import { Usage } from "#usage/usage-section.tsx";
import { EffectiveSettings } from "./effective-settings.tsx";
import {
  explainWith,
  type ExplainRefusal,
  type ExplainWith,
  type ProjectFile,
} from "./explain-with.ts";

/** Where the explanation stands. */
export type Explained =
  | { readonly kind: "reading" }
  | { readonly kind: "explained"; readonly explanation: SettingsExplanation }
  /** The engine refused; its English message, which the code's catalog message replaces once #912 maps it (ADR 0079). */
  | { readonly kind: "failed"; readonly message: string };

/** What a person asks to explain with. */
export interface ExplainRequest {
  readonly project: File | undefined;
  readonly runValues: string;
}

/** What Engine & settings shows. */
export interface EngineSettingsViewProps {
  readonly explained: Explained;
  /** Why the last request sent nothing, if it did not. */
  readonly refusal?: ExplainRefusal | undefined;
  /** Whether the Console has no Connection now, which disables explaining. */
  readonly readOnly?: boolean | undefined;
  readonly onExplain: (request: ExplainRequest) => void;
}

/** Why nothing was sent. */
function Refusal({ refusal }: { readonly refusal: ExplainRefusal }): ReactNode {
  const reason = refusal.reason;
  if (refusal.kind === "project") {
    const file = refusal.file;
    return (
      <Trans>
        <span translate="no">{file}</span> is no TOML file: <span translate="no">{reason}</span>
      </Trans>
    );
  }
  return (
    <Trans>
      The run values are no TOML: <span translate="no">{reason}</span>
    </Trans>
  );
}

/** Where the explanation stands, shown. */
function Explanation({ explained }: { readonly explained: Explained }): ReactNode {
  if (explained.kind === "explained") {
    return <EffectiveSettings settings={explained.explanation.settings} />;
  }
  if (explained.kind === "failed") {
    const message = explained.message;
    return (
      <p role="alert">
        <Trans>
          The engine could not explain the settings: <span translate="no">{message}</span>
        </Trans>
      </p>
    );
  }
  return (
    <p role="status">
      <Trans>Reading the effective settings…</Trans>
    </p>
  );
}

/** Engine & settings, as it shows `explained`. */
export function EngineSettingsView({
  explained,
  refusal,
  readOnly = false,
  onExplain,
}: EngineSettingsViewProps): ReactNode {
  const { t } = useLingui();
  const project = useId();
  const runValues = useId();
  const runValuesHint = useId();
  const submit = (event: SubmitEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const file = form.get("project");
    const values = form.get("run_values");
    onExplain({
      project: file instanceof File && file.name !== "" ? file : undefined,
      runValues: typeof values === "string" ? values : "",
    });
  };
  return (
    <div className="flex flex-col gap-6">
      <h1 className="font-heading text-3xl font-semibold tracking-tight">
        <Trans>Engine & settings</Trans>
      </h1>
      <section aria-labelledby={`${project}-heading`} className="flex flex-col gap-4">
        <h2 id={`${project}-heading`} className="text-xl font-semibold">
          <Trans>Effective settings</Trans>
        </h2>
        <p className="text-muted-foreground">
          <Trans>
            Each setting's effective value, the layer that set it and the layer that locked it, as{" "}
            <code translate="no">goodfetch-collie config explain</code> shows them.
          </Trans>
        </p>
        <form
          onSubmit={submit}
          aria-label={t`Explain with`}
          className="flex flex-col gap-3 rounded-lg border p-4"
        >
          <div className="flex flex-col gap-1">
            <label htmlFor={project} className="font-medium">
              <Trans>Project configuration</Trans>
            </label>
            <input id={project} name="project" type="file" accept=".toml,application/toml" />
          </div>
          <div className="flex flex-col gap-1">
            <label htmlFor={runValues} className="font-medium">
              <Trans>Run values</Trans>
            </label>
            <textarea
              id={runValues}
              name="run_values"
              rows={3}
              aria-describedby={runValuesHint}
              className="rounded-md border bg-background p-2 font-mono text-sm"
            />
            <p id={runValuesHint} className="text-muted-foreground text-sm">
              <Trans>
                TOML, one setting per line, such as <code translate="no">timeout.run = 600000</code>
                .
              </Trans>
            </p>
          </div>
          <button
            type="submit"
            disabled={readOnly}
            className="self-start rounded-md bg-primary px-3 py-1.5 font-medium text-primary-foreground focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50"
          >
            <Trans>Explain</Trans>
          </button>
          {refusal === undefined ? null : (
            <p role="alert" className="text-destructive">
              <Refusal refusal={refusal} />
            </p>
          )}
        </form>
        <Explanation explained={explained} />
      </section>
    </div>
  );
}

/** A refused call's message. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Engine & settings over `client`, explaining again at each request. */
export function EngineSettings({ client }: { readonly client: ConsoleClient }): ReactNode {
  const state = useSyncExternalStore(
    (changed) => client.watch(changed),
    () => client.state,
  );
  const [params, setParams] = useState<ExplainWith>({});
  const [explained, setExplained] = useState<Explained>({ kind: "reading" });
  const [refusal, setRefusal] = useState<ExplainRefusal | undefined>(undefined);

  useEffect(() => {
    if (state !== "connected") {
      return undefined;
    }
    let current = true;
    setExplained({ kind: "reading" });
    const read = async (): Promise<void> => {
      let next: Explained;
      try {
        next = { kind: "explained", explanation: await client.call("config.explain", params) };
      } catch (error) {
        next = { kind: "failed", message: messageOf(error) };
      }
      if (current) {
        setExplained(next);
      }
    };
    void read();
    return () => {
      current = false;
    };
  }, [client, state, params]);

  const explain = async ({ project, runValues }: ExplainRequest): Promise<void> => {
    const file: ProjectFile | undefined =
      project === undefined ? undefined : { name: project.name, text: await project.text() };
    const asked = explainWith(file, runValues);
    if (asked.ok) {
      setRefusal(undefined);
      setParams(asked.with);
    } else {
      setRefusal(asked.refusal);
    }
  };

  return (
    <div className="flex flex-col gap-10">
      <EngineSettingsView
        explained={explained}
        refusal={refusal}
        readOnly={state !== "connected"}
        onExplain={(request) => {
          void explain(request);
        }}
      />
      <Usage client={client} />
    </div>
  );
}
