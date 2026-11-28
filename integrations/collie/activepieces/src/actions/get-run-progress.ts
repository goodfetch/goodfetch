// Activepieces action: `get-run-progress` (`run.progress`; spec #608,
// issue 1409). Built with `@activepieces/pieces-framework`'s `createAction`.
import {
  createAction,
  Property,
  type Action,
  type ShortTextProperty,
} from "@activepieces/pieces-framework";
import { createCollieClient, goodfetchCollieAuth, toActivepiecesError } from "../common/client.ts";

export interface GetRunProgressProps extends Record<string, unknown> {
  readonly runId: string;
}

export const getRunProgressAction: Action<
  typeof goodfetchCollieAuth,
  { runId: ShortTextProperty<true> }
> = createAction({
  auth: goodfetchCollieAuth,
  name: "get-run-progress",
  displayName: "Get Run Progress",
  description: "Read the current progress for a Collie Run.",
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
      return await client.getRunProgress(context.propsValue.runId.trim());
    } catch (error) {
      throw toActivepiecesError(error);
    }
  },
});
