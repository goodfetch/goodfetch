// Engine & settings (spec #591, stories 94 to 96, 98 and 102; ADRs 0031 and
// 0087): its first section shows every effective value with its layer and
// lock, as `config explain` does, from `config.explain` on the engine the
// Console reached. A person may explain with a project configuration and
// run values, as the command's `--project` and `--set` do, and edit, row by
// row, a user-level value (`config.user_write`) or a run's own value, never
// a project configuration's or a Recipe manifest's, which are files under
// version control. The Operator policy section reads and writes the policy
// against its digest; Usage against each Limit, by scope, follows (story
// 99). Later sections (tokens, people and the Daemon group) join this
// screen.
import type { SettingsExplanation, UserChange } from "@goodfetch/collie-client/generated";
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
import { ErrorMessage } from "#errors/error-message.tsx";
import { Usage } from "#usage/usage-section.tsx";
import { EffectiveSettings, type SettingEdit } from "./effective-settings.tsx";
import {
  explainWith,
  withRunValue,
  type ExplainRefusal,
  type ExplainWith,
  type ProjectFile,
} from "./explain-with.ts";
import { OperatorPolicy } from "./operator-policy.tsx";
import { Form } from "#form.tsx";

/** Where the explanation stands. */
export type Explained =
  | { readonly kind: "reading" }
  | { readonly kind: "explained"; readonly explanation: SettingsExplanation }
  /** The engine refused, with this error. */
  | { readonly kind: "failed"; readonly error: unknown };

/** What a person asks to explain with. */
export interface ExplainRequest {
  readonly project: File | undefined;
  readonly runValues: string;
}

/** What the last edit of a row came to. */
export type EditNotice =
  | { readonly kind: "user_written"; readonly key: string }
  | { readonly kind: "refused"; readonly error: unknown };

/** What Engine & settings shows. */
export interface EngineSettingsViewProps {
  readonly explained: Explained;
  /** Why the last request sent nothing, if it did not. */
  readonly refusal?: ExplainRefusal | undefined;
  /** Whether the Console has no Connection now, which disables explaining and editing. */
  readonly readOnly?: boolean | undefined;
  /** The run values, as TOML. */
  readonly runValues?: string | undefined;
  readonly onRunValuesChange?: ((runValues: string) => void) | undefined;
  readonly onExplain: (request: ExplainRequest) => void;
  /** Where a row's edit goes; without it, no row offers an edit. */
  readonly onEdit?: ((edit: SettingEdit) => void) | undefined;
  readonly editNotice?: EditNotice | undefined;
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

/** What the last edit came to, shown. */
function EditResult({ notice }: { readonly notice: EditNotice }): ReactNode {
  if (notice.kind === "user_written") {
    const key = notice.key;
    return (
      <output className="block">
        <Trans>
          <code translate="no">{key}</code> is written to user configuration, for new runs.
        </Trans>
      </output>
    );
  }
  return (
    <div role="alert">
      <p>
        <Trans>The engine did not write the value:</Trans>
      </p>
      <ErrorMessage error={notice.error} />
    </div>
  );
}

/** Where the explanation stands, shown. */
function Explanation({
  explained,
  readOnly,
  onEdit,
}: {
  readonly explained: Explained;
  readonly readOnly: boolean;
  readonly onEdit: ((edit: SettingEdit) => void) | undefined;
}): ReactNode {
  if (explained.kind === "explained") {
    return (
      <EffectiveSettings
        settings={explained.explanation.settings}
        readOnly={readOnly}
        onEdit={onEdit}
      />
    );
  }
  if (explained.kind === "failed") {
    return (
      <div role="alert">
        <p>
          <Trans>The engine could not explain the settings:</Trans>
        </p>
        <ErrorMessage error={explained.error} />
      </div>
    );
  }
  return (
    <output className="block">
      <Trans>Reading the effective settings…</Trans>
    </output>
  );
}

/** Engine & settings, as it shows `explained`. */
export function EngineSettingsView({
  explained,
  refusal,
  readOnly = false,
  runValues,
  onRunValuesChange,
  onExplain,
  onEdit,
  editNotice,
}: EngineSettingsViewProps): ReactNode {
  const { t } = useLingui();
  const project = useId();
  const runValuesField = useId();
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
            <code translate="no">goodfetch-collie config explain</code> shows them. Edit a
            user-level value or a run's own value here; a project configuration and a Recipe's
            manifest are files under version control, edited there.
          </Trans>
        </p>
        <Form
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
            <label htmlFor={runValuesField} className="font-medium">
              <Trans>Run values</Trans>
            </label>
            <textarea
              id={runValuesField}
              name="run_values"
              translate="no"
              rows={3}
              aria-describedby={runValuesHint}
              {...(runValues === undefined
                ? {}
                : {
                    value: runValues,
                    onChange: (event) => {
                      onRunValuesChange?.(event.currentTarget.value);
                    },
                  })}
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
        </Form>
        {editNotice === undefined ? null : <EditResult notice={editNotice} />}
        <Explanation explained={explained} readOnly={readOnly} onEdit={onEdit} />
      </section>
    </div>
  );
}

/** Engine & settings over `client`, explaining again at each request and after each write. */
export function EngineSettings({ client }: { readonly client: ConsoleClient }): ReactNode {
  const state = useSyncExternalStore(
    (changed) => client.watch(changed),
    () => client.state,
  );
  const [params, setParams] = useState<ExplainWith>({});
  const [project, setProject] = useState<ProjectFile | undefined>(undefined);
  const [runValues, setRunValues] = useState("");
  const [explained, setExplained] = useState<Explained>({ kind: "reading" });
  const [refusal, setRefusal] = useState<ExplainRefusal | undefined>(undefined);
  const [editNotice, setEditNotice] = useState<EditNotice | undefined>(undefined);

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
        next = { kind: "failed", error };
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

  /** Explains with `file` and `values`, or shows why it cannot. */
  const explainAgain = (file: ProjectFile | undefined, values: string): void => {
    const asked = explainWith(file, values);
    if (asked.ok) {
      setRefusal(undefined);
      setParams(asked.with);
    } else {
      setRefusal(asked.refusal);
    }
  };

  const explain = async ({ project: picked, runValues: values }: ExplainRequest): Promise<void> => {
    const file: ProjectFile | undefined =
      picked === undefined ? undefined : { name: picked.name, text: await picked.text() };
    setProject(file);
    setRunValues(values);
    setEditNotice(undefined);
    explainAgain(file, values);
  };

  const edit = async ({ layer, key, value }: SettingEdit): Promise<void> => {
    if (layer === "run") {
      const changed = withRunValue(runValues, key, value);
      if (!changed.ok) {
        setRefusal(changed.refusal);
        return;
      }
      setRunValues(changed.runValues);
      setEditNotice(undefined);
      explainAgain(project, changed.runValues);
      return;
    }
    try {
      await client.call("config.user_write", {
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a setting's value is any JSON value, which the generated type writes as an object.
        changes: [{ key, value: value as UserChangeValue }],
      });
      setEditNotice({ kind: "user_written", key });
      setParams((current) => ({ ...current }));
    } catch (error) {
      setEditNotice({ kind: "refused", error });
    }
  };

  return (
    <div className="flex flex-col gap-10">
      <EngineSettingsView
        explained={explained}
        refusal={refusal}
        readOnly={state !== "connected"}
        runValues={runValues}
        onRunValuesChange={setRunValues}
        onExplain={(request) => {
          void explain(request);
        }}
        onEdit={(request) => {
          void edit(request);
        }}
        editNotice={editNotice}
      />
      <OperatorPolicy
        client={client}
        onWritten={() => {
          setParams((current) => ({ ...current }));
        }}
      />
      <Usage client={client} />
    </div>
  );
}

/** A user-level value as the generated type writes it. */
type UserChangeValue = UserChange["value"];
