// The engine that owns a live run (ADRs 0031 and 0098), as its Run index
// row names it: a Daemon at its endpoint, or an Attached engine at its
// private endpoint, which the Runs screen and a run's page both show.
import type { DaemonEndpoint, RunOwner } from "@goodfetch/collie-client/generated";
import { Trans } from "@lingui/react/macro";
import type { ReactNode } from "react";

/** Where `endpoint` listens: its socket's path or its address. */
function address(endpoint: DaemonEndpoint): string {
  return "unix" in endpoint ? endpoint.unix : endpoint.tcp;
}

/** The engine `owner` names, with where it is reached. */
export function Owner({ owner }: { readonly owner: RunOwner }): ReactNode {
  if ("daemon" in owner) {
    const at = address(owner.daemon);
    return (
      <Trans>
        the Daemon at{" "}
        <code translate="no" className="font-mono">
          {at}
        </code>
      </Trans>
    );
  }
  const at = address(owner.attached.address);
  return (
    <Trans>
      the Attached engine at{" "}
      <code translate="no" className="font-mono">
        {at}
      </code>
    </Trans>
  );
}
