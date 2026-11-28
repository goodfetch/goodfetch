// Activepieces action: `start-recipe` (`recipe.start`, detached; spec #608,
// issue 1409).
import type { ActionDefinition, ActivepiecesCollieAuth } from "../ap-types.ts";
import {
  createCollieClient,
  goodfetchCollieAuth,
  parseOptionalObject,
  toActivepiecesError,
} from "../common/client.ts";

export interface StartRecipeProps extends Record<string, unknown> {
  readonly recipe: string;
  readonly params?: Record<string, unknown> | string;
  readonly settings?: Record<string, unknown> | string;
}

export const startRecipeAction: ActionDefinition<ActivepiecesCollieAuth, StartRecipeProps> = {
  name: "start-recipe",
  displayName: "Start Recipe Run",
  description: "Start a detached Recipe run on the Collie Daemon.",
  auth: goodfetchCollieAuth,
  props: {
    recipe: {
      displayName: "Recipe",
      description: "Recipe identifier or path to execute on the Collie Daemon.",
      type: "SHORT_TEXT",
      required: true,
    },
    params: {
      displayName: "Recipe Parameters",
      description: "Optional JSON object of Recipe input parameters.",
      type: "JSON",
      required: false,
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
      const recipe = context.propsValue.recipe.trim();
      const params = parseOptionalObject(context.propsValue.params, "params");
      const settings = parseOptionalObject(context.propsValue.settings, "settings");
      return await client.startRecipe({
        recipe,
        ...(params === undefined ? {} : { params }),
        ...(settings === undefined ? {} : { settings }),
      });
    } catch (error) {
      throw toActivepiecesError(error);
    }
  },
};
