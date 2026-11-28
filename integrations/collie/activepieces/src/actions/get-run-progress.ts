// Activepieces action: `get-run-progress` (`run.progress`; spec #608,
// issue 1409).
import type { ActionDefinition, ActivepiecesCollieAuth } from "../ap-types.ts";
import { createCollieClient, goodfetchCollieAuth, toActivepiecesError } from "../common/client.ts";

export interface GetRunProgressProps extends Record<string, unknown> {
  readonly runId: string;
}

export const getRunProgressAction: ActionDefinition<ActivepiecesCollieAuth, GetRunProgressProps> = {
  name: "get-run-progress",
  displayName: "Get Run Progress",
  description: "Read the current progress for a Collie Run.",
  auth: goodfetchCollieAuth,
  props: {
    runId: {
      displayName: "Run ID",
      description: "Identifier of the Collie Run to inspect.",
      type: "SHORT_TEXT",
      required: true,
    },
  },
  async run(context) {
    try {
      const client = createCollieClient(context.auth, context.fetchImpl);
      return await client.getRunProgress(context.propsValue.runId.trim());
    } catch (error) {
      throw toActivepiecesError(error);
    }
  },
};
