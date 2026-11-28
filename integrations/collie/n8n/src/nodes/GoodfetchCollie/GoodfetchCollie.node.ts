// Goodfetch Collie n8n action node (`@goodfetch/n8n-nodes-collie`; ADRs 0027,
// 0070, 0090, 0095; spec #608, issue 1405).
//
// Exposes Collie's four Integration actions:
// - `startRecipe` (`recipe.start`, detached)
// - `startCatalogRun` (`catalog_run.start`, detached)
// - `getRunReport` (`run.report`)
// - `getRunProgress` (`run.progress`)
import type {
  IDataObject,
  IExecuteFunctions,
  INodeExecutionData,
  INodeType,
  INodeTypeDescription,
} from "../../n8n-types.ts";
import {
  callCollieRpc,
  N8nCollieError,
  parseCredentials,
  parseJsonObjectParam,
} from "../../rpc.ts";

export class GoodfetchCollie implements INodeType {
  readonly description: INodeTypeDescription = {
    displayName: "Goodfetch Collie",
    name: "goodfetchCollie",
    group: ["transform"],
    version: 1,
    subtitle: '={{$parameter["operation"]}}',
    description: "Start Collie browser automation runs and read their report or progress.",
    defaults: {
      name: "Goodfetch Collie",
    },
    inputs: ["main"],
    outputs: ["main"],
    usableAsTool: true,
    credentials: [
      {
        name: "goodfetchCollieApi",
        required: true,
      },
    ],
    properties: [
      {
        displayName: "Operation",
        name: "operation",
        type: "options",
        noDataExpression: true,
        default: "startRecipe",
        options: [
          {
            name: "Start Recipe Run",
            value: "startRecipe",
            description: "Start a detached Recipe run on the Collie Daemon.",
            action: "Start a detached Recipe run",
          },
          {
            name: "Start Catalog Run",
            value: "startCatalogRun",
            description: "Start a detached code-free catalog run on the Collie Daemon.",
            action: "Start a detached code-free catalog run",
          },
          {
            name: "Get Run Report",
            value: "getRunReport",
            description: "Read the Run report for a Collie Run.",
            action: "Get a Run report",
          },
          {
            name: "Get Run Progress",
            value: "getRunProgress",
            description: "Read the current progress for a Collie Run.",
            action: "Get a Run progress",
          },
        ],
      },
      {
        displayName: "Recipe",
        name: "recipe",
        type: "string",
        default: "",
        required: true,
        displayOptions: {
          show: {
            operation: ["startRecipe"],
          },
        },
        description: "Recipe identifier or path to execute on the Collie Daemon.",
      },
      {
        displayName: "Recipe Parameters (JSON)",
        name: "recipeParams",
        type: "json",
        default: "",
        displayOptions: {
          show: {
            operation: ["startRecipe"],
          },
        },
        description: "Optional JSON object of Recipe input parameters.",
      },
      {
        displayName: "Catalog Target (JSON)",
        name: "catalogTarget",
        type: "json",
        default: "{}",
        required: true,
        displayOptions: {
          show: {
            operation: ["startCatalogRun"],
          },
        },
        description: "JSON object describing the code-free catalog run target.",
      },
      {
        displayName: "Settings (JSON)",
        name: "settings",
        type: "json",
        default: "",
        displayOptions: {
          show: {
            operation: ["startRecipe", "startCatalogRun"],
          },
        },
        description: "Optional JSON object of non-locked run settings.",
      },
      {
        displayName: "Run ID",
        name: "runId",
        type: "string",
        default: "",
        required: true,
        displayOptions: {
          show: {
            operation: ["getRunReport", "getRunProgress"],
          },
        },
        description: "Identifier of the Collie Run to inspect.",
      },
    ],
  };

  async execute(this: IExecuteFunctions): Promise<INodeExecutionData[][]> {
    const items = this.getInputData();
    const node = this.getNode();
    const rawCredentials = await this.getCredentials("goodfetchCollieApi");
    const credentials = parseCredentials(rawCredentials, node);
    const returnItems: INodeExecutionData[] = [];

    for (let itemIndex = 0; itemIndex < items.length; itemIndex += 1) {
      try {
        const operation = String(this.getNodeParameter("operation", itemIndex));
        let result: IDataObject;

        if (operation === "startRecipe") {
          const recipe = String(this.getNodeParameter("recipe", itemIndex)).trim();
          const recipeParams = parseJsonObjectParam(
            this.getNodeParameter("recipeParams", itemIndex, ""),
            "recipeParams",
            node,
          );
          const settings = parseJsonObjectParam(
            this.getNodeParameter("settings", itemIndex, ""),
            "settings",
            node,
          );
          const payload: IDataObject = { recipe, detached: true };
          if (recipeParams !== undefined) {
            payload["params"] = recipeParams;
          }
          if (settings !== undefined) {
            payload["settings"] = settings;
          }
          result = await callCollieRpc({
            helpers: this.helpers,
            credentials,
            method: "recipe.start",
            params: payload,
            node,
          });
        } else if (operation === "startCatalogRun") {
          const target = parseJsonObjectParam(
            this.getNodeParameter("catalogTarget", itemIndex, "{}"),
            "catalogTarget",
            node,
          );
          const settings = parseJsonObjectParam(
            this.getNodeParameter("settings", itemIndex, ""),
            "settings",
            node,
          );
          const payload: IDataObject = { target: target ?? {}, detached: true };
          if (settings !== undefined) {
            payload["settings"] = settings;
          }
          result = await callCollieRpc({
            helpers: this.helpers,
            credentials,
            method: "catalog_run.start",
            params: payload,
            node,
          });
        } else if (operation === "getRunReport") {
          const runId = String(this.getNodeParameter("runId", itemIndex)).trim();
          result = await callCollieRpc({
            helpers: this.helpers,
            credentials,
            method: "run.report",
            params: { run_id: runId },
            node,
          });
        } else if (operation === "getRunProgress") {
          const runId = String(this.getNodeParameter("runId", itemIndex)).trim();
          result = await callCollieRpc({
            helpers: this.helpers,
            credentials,
            method: "run.progress",
            params: { run_id: runId },
            node,
          });
        } else {
          throw new N8nCollieError(
            {
              code: "unsupported_operation",
              group: "program",
              fix: "Select one of startRecipe, startCatalogRun, getRunReport, or getRunProgress.",
              message: `Unsupported Goodfetch Collie operation: ${operation}`,
            },
            node,
          );
        }

        returnItems.push({
          json: result,
          pairedItem: { item: itemIndex },
        });
      } catch (error) {
        if (this.continueOnFail()) {
          if (error instanceof N8nCollieError) {
            returnItems.push({
              json: {
                error: error.message,
                code: error.code,
                group: error.group,
                fix: error.fix,
              },
              pairedItem: { item: itemIndex },
            });
            continue;
          }
          const message = error instanceof Error ? error.message : String(error);
          returnItems.push({
            json: { error: message },
            pairedItem: { item: itemIndex },
          });
          continue;
        }
        throw error;
      }
    }

    return [returnItems];
  }
}
