// A failure as the Console shows it (ADRs 0073 and 0079): the catalog's
// message for its Error code, the operation first, then the program's own
// exception when the code names one, the target, URL, observed, limit and
// fix, then the call log; or, for a code the catalog does not know, the
// engine's English message. Every value the engine sent sits under
// `translate="no"`, so browser translation leaves it alone.
import { Trans, useLingui } from "@lingui/react/macro";
import type { ReactNode } from "react";
import { formatDuration } from "#format.ts";
import type { EndingError } from "@goodfetch/collie-client/generated";
import { describeEnding, describeError, type ErrorDescription, type ErrorLimit } from "./errors.ts";

/** A JSON value as text. */
function json(value: unknown): string {
  return JSON.stringify(value) ?? "null";
}

/** A limit's value: a duration for a `timeout.` setting's milliseconds, else its JSON. */
function LimitValue({ limit }: { readonly limit: ErrorLimit }): ReactNode {
  const { key, value } = limit;
  if (key.startsWith("timeout.") && typeof value === "number") {
    return (
      <data value={String(value)} translate="no">
        {formatDuration(value)}
      </data>
    );
  }
  return <code translate="no">{json(value)}</code>;
}

/** The setting, value and layer that bounded the call. */
function Limit({ limit }: { readonly limit: ErrorLimit }): ReactNode {
  const { key, layer } = limit;
  const value = <LimitValue limit={limit} />;
  if (layer === undefined) {
    return (
      <Trans>
        <code translate="no">{key}</code> is {value}
      </Trans>
    );
  }
  return (
    <Trans>
      <code translate="no">{key}</code> is {value}, set by {layer}
    </Trans>
  );
}

/** One field of a failure: its name, then its value. */
function Field({
  name,
  children,
}: {
  readonly name: ReactNode;
  readonly children: ReactNode;
}): ReactNode {
  return (
    <div className="flex flex-wrap gap-x-2">
      <dt className="font-medium">{name}</dt>
      <dd className="min-w-0 break-words">{children}</dd>
    </div>
  );
}

/** How the Console shows `error`: the catalog's message with its fields, or the engine's English message. */
export function ErrorMessage({ error }: { readonly error: unknown }): ReactNode {
  const { i18n } = useLingui();
  return <Described described={describeError(error, i18n)} />;
}

/** How the Console shows the error a Run report's ending raised, as `ErrorMessage` shows a failed call's. */
export function EndingErrorMessage({ ending }: { readonly ending: EndingError }): ReactNode {
  const { i18n } = useLingui();
  return <Described described={describeEnding(ending, i18n)} />;
}

/** A failure as `described` gives it. */
function Described({ described }: { readonly described: ErrorDescription }): ReactNode {
  if (described.kind === "engine") {
    return <p translate="no">{described.message}</p>;
  }
  const { code, summary, operation, target, url, observed, limit, fix, callLog, exception } =
    described;
  return (
    <div className="space-y-1">
      <p>
        {operation === undefined ? (
          summary
        ) : (
          <Trans>
            <code translate="no">{operation}</code>: {summary}
          </Trans>
        )}
      </p>
      {exception !== undefined && (
        <p>
          <Trans>
            The program's own exception: <code translate="no">{exception.type}</code>,{" "}
            <span translate="no">{exception.message}</span>
          </Trans>
        </p>
      )}
      <dl className="text-sm">
        {target !== undefined && (
          <Field name={<Trans>Target</Trans>}>
            <code translate="no">{target}</code>
          </Field>
        )}
        {url !== undefined && (
          <Field name={<Trans>URL</Trans>}>
            <span translate="no">{url}</span>
          </Field>
        )}
        {observed !== undefined && (
          <Field name={<Trans>Observed</Trans>}>
            <span translate="no">{observed}</span>
          </Field>
        )}
        {limit !== undefined && (
          <Field name={<Trans>Limit</Trans>}>
            <Limit limit={limit} />
          </Field>
        )}
        {fix !== undefined && (
          <Field name={<Trans>Fix</Trans>}>
            <span translate="no">{fix}</span>
          </Field>
        )}
        <Field name={<Trans>Error code</Trans>}>
          <code translate="no">{code}</code>
        </Field>
      </dl>
      {callLog.length > 0 && (
        <details className="text-sm">
          <summary>
            <Trans>Call log</Trans>
          </summary>
          <ol className="list-decimal ps-6 font-mono" translate="no">
            {callLog.map((step, index) => (
              <li key={index}>{step}</li>
            ))}
          </ol>
        </details>
      )}
    </div>
  );
}
