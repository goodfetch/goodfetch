// Activepieces action: `get-run-report` (`run.report`; spec #608, issue 1409).
// Built with `@activepieces/pieces-framework`'s `createAction`.
import {
  createAction,
  Property,
  type Action,
  type ShortTextProperty,
} from "@activepieces/pieces-framework";
import { createCollieClient, goodfetchCollieAuth, toActivepiecesError } from "../common/client.ts";

export interface GetRunReportProps extends Record<string, unknown> {
  readonly runId: string;
}

export const getRunReportAction: Action<
  typeof goodfetchCollieAuth,
  { runId: ShortTextProperty<true> }
> = createAction({
  auth: goodfetchCollieAuth,
  name: "get-run-report",
  displayName: "Get Run Report",
  description: "Read the Run report for a Collie Run.",
  props: {
    runId: Property.ShortText({
      displayName: "Run ID",
      description: "Identifier of the Collie Run to inspect.",
      required: true,
    }),
  },
  async run(context) {
    try {
      const client = createCollieClient(context.auth);
      return await client.getRunReport(context.propsValue.runId.trim());
    } catch (error) {
      throw toActivepiecesError(error);
    }
  },
});
