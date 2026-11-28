// Activepieces action: `start-catalog-run` (`catalog_run.start`, detached;
// spec #608, issue 1409). Built with `@activepieces/pieces-framework`.
import {
  createAction,
  Property,
  type Action,
  type JsonProperty,
} from "@activepieces/pieces-framework";
import {
  createCollieClient,
  goodfetchCollieAuth,
  parseOptionalObject,
  parseOptionalStringArray,
  toActivepiecesError,
} from "../common/client.ts";

export interface StartCatalogRunProps extends Record<string, unknown> {
  readonly target: Record<string, unknown> | string;
  readonly tags?: readonly string[] | string | undefined;
  readonly settings?: Record<string, unknown> | string | undefined;
}

export const startCatalogRunAction: Action<
  typeof goodfetchCollieAuth,
  {
    target: JsonProperty<true>;
    tags: JsonProperty<false>;
    settings: JsonProperty<false>;
  }
> = createAction({
  auth: goodfetchCollieAuth,
  name: "start-catalog-run",
  displayName: "Start Catalog Run",
  description: "Start a detached code-free catalog run on the Collie Daemon.",
  props: {
    target: Property.Json({
      displayName: "Catalog Target",
      description: "JSON object describing the code-free catalog run target.",
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
      const target = parseOptionalObject(context.propsValue.target, "target") ?? {};
      const tags = parseOptionalStringArray(context.propsValue.tags, "tags");
      const settings = parseOptionalObject(context.propsValue.settings, "settings");
      return await client.startCatalogRun({
        target,
        ...(tags === undefined ? {} : { tags }),
        ...(settings === undefined ? {} : { settings }),
      });
    } catch (error) {
      throw toActivepiecesError(error);
    }
  },
});
