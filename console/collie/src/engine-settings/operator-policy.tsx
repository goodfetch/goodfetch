// The Operator policy, a section of Engine & settings (spec #591, stories
// 96 and 98; ADR 0031): every entry of its `settings`, `lock` and `cap`
// tables and its version, read with `config.policy_read`, and a change to
// one entry, written with `config.policy_write` against the digest read, so
// a hand edit since is never lost: the engine then answers `policy_conflict`,
// and the section offers to read the policy again. A tightened cap reaches
// running runs at their next admission point; every other change applies
// to new runs, each naming the policy version it ran under.
import type {
  OperatorPolicyRead,
  PolicyTable,
  PolicyWritten,
} from "@goodfetch/collie-client/generated";
import { GoodfetchError } from "@goodfetch/collie-client/generated";
import type { MessageDescriptor } from "@lingui/core";
import { msg } from "@lingui/core/macro";
import { Trans, useLingui } from "@lingui/react/macro";
import {
  useEffect,
  useId,
  useState,
  useSyncExternalStore,
  type ReactNode,
  type SubmitEvent,
} from "react";
import type { ConsoleClient } from "#client/client.ts";
import { ErrorMessage } from "#errors/error-message.tsx";
import { Form } from "#form.tsx";

/** Each table's name. */
const TABLES: Readonly<Record<PolicyTable, MessageDescriptor>> = {
  settings: msg`Sets`,
  lock: msg`Sets and locks`,
  cap: msg`Caps`,
};

/** Where the reading of the policy stands. */
export type PolicyRead =
  | { readonly kind: "reading" }
  | { readonly kind: "read"; readonly policy: OperatorPolicyRead }
  /** The engine refused, with this error. */
  | { readonly kind: "failed"; readonly error: unknown };

/** What the last write came to. */
export type PolicyNotice =
  | { readonly kind: "written"; readonly written: PolicyWritten }
  /** The JSON of the value is no JSON. */
  | { readonly kind: "invalid_value" }
  | { readonly kind: "refused"; readonly error: unknown };

/** One change a person asks for: the table, the key, and its value, `null` to remove it. */
export interface PolicyChangeRequest {
  readonly table: PolicyTable;
  readonly key: string;
  readonly value: string;
}

/** What the section shows. */
export interface OperatorPolicyViewProps {
  readonly read: PolicyRead;
  readonly notice?: PolicyNotice | undefined;
  /** Whether the Console has no Connection now, or a write is under way, which disables writing. */
  readonly readOnly?: boolean | undefined;
  readonly onWrite: (change: PolicyChangeRequest) => void;
  readonly onReadAgain: () => void;
}

/** A JSON value as text. */
function json(value: unknown): string {
  return JSON.stringify(value) ?? "null";
}

/** Whether `error` is the engine's `policy_conflict`. */
function isConflict(error: unknown): boolean {
  return error instanceof GoodfetchError && error.code === "policy_conflict";
}

/** The policy's entries, one row each. */
function Entries({ policy }: { readonly policy: OperatorPolicyRead }): ReactNode {
  const { i18n } = useLingui();
  if (policy.entries.length === 0) {
    return (
      <p>
        <Trans>The Operator policy sets, locks and caps nothing.</Trans>
      </p>
    );
  }
  return (
    <table className="w-full border-collapse text-start text-sm">
      <caption className="sr-only">
        <Trans>Operator policy entries</Trans>
      </caption>
      <thead className="border-b">
        <tr>
          <th scope="col" className="py-2 pe-4 font-medium">
            <Trans>Setting</Trans>
          </th>
          <th scope="col" className="py-2 pe-4 font-medium">
            <Trans>Table</Trans>
          </th>
          <th scope="col" className="py-2 font-medium">
            <Trans>Value</Trans>
          </th>
        </tr>
      </thead>
      <tbody>
        {policy.entries.map((entry) => (
          <tr key={`${entry.table} ${entry.key}`} className="border-b align-top">
            <th scope="row" className="py-2 pe-4 font-mono font-normal" translate="no">
              {entry.key}
            </th>
            <td className="py-2 pe-4">{i18n._(TABLES[entry.table])}</td>
            <td className="py-2 font-mono break-all" translate="no">
              {json(entry.value)}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** What the last write came to, shown. */
function Notice({
  notice,
  onReadAgain,
}: {
  readonly notice: PolicyNotice;
  readonly onReadAgain: () => void;
}): ReactNode {
  if (notice.kind === "written") {
    const digest = notice.written.digest;
    return (
      <output className="block">
        <Trans>
          Written. New runs name the policy version <code translate="no">{digest}</code>; a
          tightened cap reaches running runs at their next admission point.
        </Trans>
      </output>
    );
  }
  if (notice.kind === "invalid_value") {
    return (
      <p role="alert" className="text-destructive">
        <Trans>The value is no JSON.</Trans>
      </p>
    );
  }
  return (
    <div role="alert" className="flex flex-col gap-2">
      <ErrorMessage error={notice.error} />
      {isConflict(notice.error) ? (
        <button
          type="button"
          onClick={onReadAgain}
          className="self-start rounded-md border px-3 py-1.5 font-medium focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none"
        >
          <Trans>Read the policy again</Trans>
        </button>
      ) : null}
    </div>
  );
}

/** The Operator policy section, as it shows `read`. */
export function OperatorPolicyView({
  read,
  notice,
  readOnly = false,
  onWrite,
  onReadAgain,
}: OperatorPolicyViewProps): ReactNode {
  const { t, i18n } = useLingui();
  const heading = useId();
  const table = useId();
  const key = useId();
  const value = useId();
  const valueHint = useId();
  const submit = (event: SubmitEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const chosen = form.get("table");
    const named = form.get("key");
    const given = form.get("value");
    onWrite({
      table: chosen === "lock" || chosen === "cap" ? chosen : "settings",
      key: typeof named === "string" ? named.trim() : "",
      value: typeof given === "string" ? given : "",
    });
  };
  return (
    <section aria-labelledby={heading} className="flex flex-col gap-4">
      <h2 id={heading} className="text-xl font-semibold">
        <Trans>Operator policy</Trans>
      </h2>
      {read.kind === "reading" ? (
        <output className="block">
          <Trans>Reading the Operator policy…</Trans>
        </output>
      ) : null}
      {read.kind === "failed" ? (
        <div role="alert">
          <p>
            <Trans>The engine could not read the Operator policy:</Trans>
          </p>
          <ErrorMessage error={read.error} />
        </div>
      ) : null}
      {read.kind === "read" ? (
        <>
          <p className="text-muted-foreground">
            <Trans>
              Read from <code translate="no">{read.policy.path}</code>, version{" "}
              <code translate="no">{read.policy.digest}</code>. A write is made only if the file has
              not changed since.
            </Trans>
          </p>
          <Entries policy={read.policy} />
          <Form
            onSubmit={submit}
            aria-label={t`Change the Operator policy`}
            className="flex flex-col gap-3 rounded-lg border p-4"
          >
            <div className="flex flex-col gap-1">
              <label htmlFor={table} className="font-medium">
                <Trans>Table</Trans>
              </label>
              <select id={table} name="table" className="rounded-md border bg-background p-2">
                {(["settings", "lock", "cap"] as const).map((name) => (
                  <option key={name} value={name}>
                    {i18n._(TABLES[name])}
                  </option>
                ))}
              </select>
            </div>
            <div className="flex flex-col gap-1">
              <label htmlFor={key} className="font-medium">
                <Trans>Setting</Trans>
              </label>
              <input
                id={key}
                name="key"
                required
                translate="no"
                className="rounded-md border bg-background p-2 font-mono text-sm"
              />
            </div>
            <div className="flex flex-col gap-1">
              <label htmlFor={value} className="font-medium">
                <Trans>Value</Trans>
              </label>
              <input
                id={value}
                name="value"
                aria-describedby={valueHint}
                translate="no"
                className="rounded-md border bg-background p-2 font-mono text-sm"
              />
              <p id={valueHint} className="text-muted-foreground text-sm">
                <Trans>
                  JSON, such as <code translate="no">600000</code>; empty removes the setting from
                  the table.
                </Trans>
              </p>
            </div>
            <button
              type="submit"
              disabled={readOnly}
              className="self-start rounded-md bg-primary px-3 py-1.5 font-medium text-primary-foreground focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50"
            >
              <Trans>Write</Trans>
            </button>
          </Form>
        </>
      ) : null}
      {notice === undefined ? null : <Notice notice={notice} onReadAgain={onReadAgain} />}
    </section>
  );
}

/** The value `text` writes, `null` for no text, or `undefined` when it is no JSON. */
function valueOf(text: string): { readonly value: unknown } | undefined {
  if (text.trim() === "") {
    return { value: null };
  }
  try {
    return { value: JSON.parse(text) };
  } catch {
    return undefined;
  }
}

/** The Operator policy section over `client`; `onWritten` hears of each write. */
export function OperatorPolicy({
  client,
  onWritten,
}: {
  readonly client: ConsoleClient;
  readonly onWritten?: (() => void) | undefined;
}): ReactNode {
  const state = useSyncExternalStore(
    (changed) => client.watch(changed),
    () => client.state,
  );
  const [reads, setReads] = useState(0);
  const [read, setRead] = useState<PolicyRead>({ kind: "reading" });
  const [notice, setNotice] = useState<PolicyNotice | undefined>(undefined);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (state !== "connected") {
      return undefined;
    }
    let current = true;
    const load = async (): Promise<void> => {
      let next: PolicyRead;
      try {
        next = { kind: "read", policy: await client.call("config.policy_read", {}) };
      } catch (error) {
        next = { kind: "failed", error };
      }
      if (current) {
        setRead(next);
      }
    };
    void load();
    return () => {
      current = false;
    };
  }, [client, state, reads]);

  const write = async ({ table, key, value }: PolicyChangeRequest): Promise<void> => {
    if (read.kind !== "read") {
      return;
    }
    const given = valueOf(value);
    if (given === undefined) {
      setNotice({ kind: "invalid_value" });
      return;
    }
    setBusy(true);
    try {
      const written = await client.call("config.policy_write", {
        digest: read.policy.digest,
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a setting's value is any JSON value, which the generated type writes as an object.
        changes: [{ table, key, value: given.value as PolicyEntryValue }],
      });
      setNotice({ kind: "written", written });
      setReads((count) => count + 1);
      onWritten?.();
    } catch (error) {
      setNotice({ kind: "refused", error });
    } finally {
      setBusy(false);
    }
  };

  return (
    <OperatorPolicyView
      read={read}
      notice={notice}
      readOnly={state !== "connected" || busy}
      onWrite={(change) => {
        void write(change);
      }}
      onReadAgain={() => {
        setNotice(undefined);
        setReads((count) => count + 1);
      }}
    />
  );
}

/** A change's value as the generated type writes it. */
type PolicyEntryValue = OperatorPolicyRead["entries"][number]["value"];
