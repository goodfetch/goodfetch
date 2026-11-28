// Goodfetch Collie n8n action node (`@goodfetch/n8n-nodes-collie`; ADRs 0027,
// 0070, 0090, 0095; spec #608, issue 1405).
//
// Exposes Collie's four Integration actions:
// - `startRecipe` (`recipe.start`, detached)
// - `startCatalogRun` (`catalog_run.start`, detached)
// - `getRunReport` (`run.report`)
// - `getRunProgress` (`run.progress`)
import {
  NodeConnectionTypes,
  NodeOperationError,
  type IDataObject,
  type IExecuteFunctions,
  type INodeExecutionData,
  type INodeType,
  type INodeTypeDescription,
} from "n8n-workflow";
import {
  callCollieRpc,
  createCollieNodeError,
  isRecord,
  parseCredentials,
  parseJsonObjectParam,
  parseJsonStringArrayParam,
  readCollieErrorMetadata,
  readStringParam,
} from "./rpc.ts";

export class GoodfetchCollie implements INodeType {
  readonly description: INodeTypeDescription = {
    displayName: "Goodfetch Collie",
    name: "goodfetchCollie",
    icon: {
      light: "file:goodfetchCollie.svg",
      dark: "file:goodfetchCollie.dark.svg",
    },
    group: ["transform"],
    version: 1,
    subtitle: '={{$parameter["operation"]}}',
    description: "Start Collie browser automation runs and read their report or progress",
    defaults: {
      name: "Goodfetch Collie",
    },
    inputs: [NodeConnectionTypes.Main],
    outputs: [NodeConnectionTypes.Main],
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
            name: "Get Run Progress",
            value: "getRunProgress",
            description: "Read the current progress for a Collie Run",
            action: "Get a run progress",
          },
          {
            name: "Get Run Report",
            value: "getRunReport",
            description: "Read the Run report for a Collie Run",
            action: "Get a run report",
          },
          {
            name: "Start Catalog Run",
            value: "startCatalogRun",
            description: "Start a detached code-free catalog run on the Collie Daemon",
            action: "Start a catalog run",
          },
          {
            name: "Start Recipe Run",
            value: "startRecipe",
            description: "Start a detached Recipe run on the Collie Daemon",
            action: "Start a detached recipe run",
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
        description: "Recipe identifier or path to execute on the Collie Daemon",
      },
      {
        displayName: "Catalog Target",
        name: "catalogTarget",
        type: "json",
        default: "{}",
        required: true,
        displayOptions: {
          show: {
            operation: ["startCatalogRun"],
          },
        },
        description: "JSON object describing the code-free catalog run target",
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
        description: "Identifier of the Collie Run to inspect",
      },
      {
        displayName: "Additional Fields",
        name: "additionalFields",
        type: "collection",
        placeholder: "Add Field",
        default: {},
        displayOptions: {
          show: {
            operation: ["startRecipe", "startCatalogRun"],
          },
        },
        options: [
          {
            displayName: "Run ID",
            name: "runId",
            type: "string",
            default: "",
            description: "Optional explicit Run ID for idempotent detached starts",
          },
          {
            displayName: "Settings",
            name: "settings",
            type: "json",
            default: "",
            description: "Optional JSON object of non-locked run settings",
          },
          {
            displayName: "Tags",
            name: "tags",
            type: "json",
            default: "",
            description: "Optional JSON array of string tags attached to the Run",
          },
        ],
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
        const operation = readStringParam(this.getNodeParameter("operation", itemIndex));
        let result: IDataObject;

        if (operation === "startRecipe") {
          const recipe = readStringParam(this.getNodeParameter("recipe", itemIndex)).trim();
          const additionalRaw = this.getNodeParameter("additionalFields", itemIndex, {});
          const additional = isRecord(additionalRaw) ? additionalRaw : {};
          const customRunId =
            typeof additional["runId"] === "string" && additional["runId"].trim().length > 0
              ? additional["runId"].trim()
              : undefined;
          const tags = parseJsonStringArrayParam(
            additional["tags"] ?? this.getNodeParameter("tags", itemIndex, ""),
            "tags",
            node,
          );
          const settings = parseJsonObjectParam(
            additional["settings"] ?? this.getNodeParameter("settings", itemIndex, ""),
            "settings",
            node,
          );
          const payload: IDataObject = { recipe, detached: true };
          if (customRunId !== undefined) {
            payload["run_id"] = customRunId;
          }
          if (tags !== undefined) {
            payload["tags"] = tags;
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
          const additionalRaw = this.getNodeParameter("additionalFields", itemIndex, {});
          const additional = isRecord(additionalRaw) ? additionalRaw : {};
          const customRunId =
            typeof additional["runId"] === "string" && additional["runId"].trim().length > 0
              ? additional["runId"].trim()
              : undefined;
          const tags = parseJsonStringArrayParam(
            additional["tags"] ?? this.getNodeParameter("tags", itemIndex, ""),
            "tags",
            node,
          );
          const settings = parseJsonObjectParam(
            additional["settings"] ?? this.getNodeParameter("settings", itemIndex, ""),
            "settings",
            node,
          );
          const payload: IDataObject = { target: target ?? {}, detached: true };
          if (customRunId !== undefined) {
            payload["run_id"] = customRunId;
          }
          if (tags !== undefined) {
            payload["tags"] = tags;
          }
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
          const runId = readStringParam(this.getNodeParameter("runId", itemIndex)).trim();
          result = await callCollieRpc({
            helpers: this.helpers,
            credentials,
            method: "run.report",
            params: { run_id: runId },
            node,
          });
        } else if (operation === "getRunProgress") {
          const runId = readStringParam(this.getNodeParameter("runId", itemIndex)).trim();
          result = await callCollieRpc({
            helpers: this.helpers,
            credentials,
            method: "run.progress",
            params: { run_id: runId },
            node,
          });
        } else {
          throw createCollieNodeError(
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
        const meta = readCollieErrorMetadata(error);
        if (this.continueOnFail()) {
          if (error instanceof NodeOperationError && meta !== undefined) {
            returnItems.push({
              json: {
                error: error.message,
                code: meta.code,
                group: meta.group,
                fix: meta.fix,
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
        if (error instanceof NodeOperationError && meta !== undefined) {
          throw createCollieNodeError(
            {
              code: meta.code,
              group: meta.group,
              fix: meta.fix,
              message: error.message,
              ...(meta.httpCode === undefined ? {} : { httpStatus: Number(meta.httpCode) }),
              details: meta.details,
            },
            node,
          );
        }
        throw new NodeOperationError(
          node,
          error instanceof Error ? error : new Error(String(error)),
        );
      }
    }

    return [returnItems];
  }
}
