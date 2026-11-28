// Activepieces action: `start-recipe` (`recipe.start`, detached; spec #608,
// issue 1409). Built with `@activepieces/pieces-framework`'s `createAction`.
import {
  createAction,
  Property,
  type Action,
  type JsonProperty,
  type ShortTextProperty,
} from "@activepieces/pieces-framework";
import {
  createCollieClient,
  goodfetchCollieAuth,
  parseOptionalObject,
  parseOptionalStringArray,
  toActivepiecesError,
} from "../common/client.ts";

export interface StartRecipeProps extends Record<string, unknown> {
  readonly recipe: string;
  readonly tags?: readonly string[] | string | undefined;
  readonly settings?: Record<string, unknown> | string | undefined;
}

export const startRecipeAction: Action<
  typeof goodfetchCollieAuth,
  {
    recipe: ShortTextProperty<true>;
    tags: JsonProperty<false>;
    settings: JsonProperty<false>;
  }
> = createAction({
  auth: goodfetchCollieAuth,
  name: "start-recipe",
  displayName: "Start Recipe Run",
  description: "Start a detached Recipe run on the Collie Daemon.",
  props: {
    recipe: Property.ShortText({
      displayName: "Recipe",
      description: "Recipe identifier or path to execute on the Collie Daemon.",
      required: true,
    }),
    tags: Property.Json({
      displayName: "Tags",
      description: "Optional JSON array of string tags attached to the Run.",
      required: false,
    }),
    settings: Property.Json({
      displayName: "Settings",
      description: "Optional JSON object of non-locked run settings.",
      required: false,
    }),
  },
  async run(context) {
    try {
      const client = createCollieClient(context.auth);
      const recipe = context.propsValue.recipe.trim();
      const tags = parseOptionalStringArray(context.propsValue.tags, "tags");
      const settings = parseOptionalObject(context.propsValue.settings, "settings");
      return await client.startRecipe({
        recipe,
        ...(tags === undefined ? {} : { tags }),
        ...(settings === undefined ? {} : { settings }),
      });
    } catch (error) {
      throw toActivepiecesError(error);
    }
  },
});
