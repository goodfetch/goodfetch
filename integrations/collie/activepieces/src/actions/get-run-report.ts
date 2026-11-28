// Activepieces action: `get-run-report` (`run.report`; spec #608, issue 1409).
import type { ActionDefinition, ActivepiecesCollieAuth } from "../ap-types.ts";
import { createCollieClient, goodfetchCollieAuth, toActivepiecesError } from "../common/client.ts";

export interface GetRunReportProps extends Record<string, unknown> {
  readonly runId: string;
}

export const getRunReportAction: ActionDefinition<ActivepiecesCollieAuth, GetRunReportProps> = {
  name: "get-run-report",
  displayName: "Get Run Report",
  description: "Read the Run report for a Collie Run.",
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
      return await client.getRunReport(context.propsValue.runId.trim());
    } catch (error) {
      throw toActivepiecesError(error);
    }
  },
};
