// Activepieces action: `start-catalog-run` (`catalog_run.start`, detached;
// spec #608, issue 1409).
import type { ActionDefinition, ActivepiecesCollieAuth } from "../ap-types.ts";
import {
  createCollieClient,
  goodfetchCollieAuth,
  parseOptionalObject,
  toActivepiecesError,
} from "../common/client.ts";

export interface StartCatalogRunProps extends Record<string, unknown> {
  readonly target: Record<string, unknown> | string;
  readonly settings?: Record<string, unknown> | string;
}

export const startCatalogRunAction: ActionDefinition<ActivepiecesCollieAuth, StartCatalogRunProps> =
  {
    name: "start-catalog-run",
    displayName: "Start Catalog Run",
    description: "Start a detached code-free catalog run on the Collie Daemon.",
    auth: goodfetchCollieAuth,
    props: {
      target: {
        displayName: "Catalog Target",
        description: "JSON object describing the code-free catalog run target.",
        type: "JSON",
        required: true,
      },
      settings: {
        displayName: "Settings",
        description: "Optional JSON object of non-locked run settings.",
        type: "JSON",
        required: false,
      },
    },
    async run(context) {
      try {
        const client = createCollieClient(context.auth, context.fetchImpl);
        const target = parseOptionalObject(context.propsValue.target, "target") ?? {};
        const settings = parseOptionalObject(context.propsValue.settings, "settings");
        return await client.startCatalogRun({
          target,
          ...(settings === undefined ? {} : { settings }),
        });
      } catch (error) {
        throw toActivepiecesError(error);
      }
    },
  };
