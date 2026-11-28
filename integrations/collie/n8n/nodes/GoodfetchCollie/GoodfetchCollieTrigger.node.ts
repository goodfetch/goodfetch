// Goodfetch Collie n8n Notification trigger node (`@goodfetch/n8n-nodes-collie`;
// ADRs 0027, 0070, 0090, 0095; spec #608, issue 1405).
//
// Subscribes via `notification.subscribe`, unsubscribes via
// `notification.unsubscribe`, verifies Standard Webhooks HMAC-SHA256 signatures
// using Web Crypto (`crypto.subtle`, fail-closed when secret is absent), and
// deduplicates deliveries by `event_id` so each Notification fires at most once.
import {
  NodeConnectionTypes,
  type IDataObject,
  type IHookFunctions,
  type INodeType,
  type INodeTypeDescription,
  type IWebhookFunctions,
  type IWebhookResponseData,
} from "n8n-workflow";
import {
  callCollieRpc,
  createCollieNodeError,
  isRecord,
  parseCredentials,
  readStringParam,
  verifyStandardWebhookSignature,
} from "./rpc.ts";

const SEEN_EVENT_IDS_CAP = 1024;

function readStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const result: string[] = [];
  for (const item of value) {
    if (typeof item === "string" && item.trim().length > 0) {
      result.push(item.trim());
    }
  }
  return result;
}

export class GoodfetchCollieTrigger implements INodeType {
  readonly description: INodeTypeDescription = {
    displayName: "Goodfetch Collie Trigger",
    name: "goodfetchCollieTrigger",
    icon: {
      light: "file:goodfetchCollie.svg",
      dark: "file:goodfetchCollie.dark.svg",
    },
    group: ["trigger"],
    version: 1,
    subtitle: '={{$parameter["channel"]}}',
    description: "Trigger an n8n workflow when a Collie Run emits a selected Notification event",
    defaults: {
      name: "Goodfetch Collie Trigger",
    },
    inputs: [],
    outputs: [NodeConnectionTypes.Main],
    usableAsTool: true,
    credentials: [
      {
        name: "goodfetchCollieApi",
        required: true,
      },
    ],
    webhooks: [
      {
        name: "default",
        httpMethod: "POST",
        responseMode: "onReceived",
        path: "webhook",
      },
    ],
    properties: [
      {
        displayName: "Channel Name",
        name: "channel",
        type: "string",
        default: "n8n-collie-trigger",
        required: true,
        description: "Stable Notification subscription channel identifier on the Collie Daemon",
      },
      {
        displayName: "Events",
        name: "events",
        type: "multiOptions",
        default: ["run_finished", "run_failed"],
        options: [
          { name: "Approval Requested", value: "approval_requested" },
          { name: "Human Pause Waiting", value: "human_pause_waiting" },
          { name: "Run Failed", value: "run_failed" },
          { name: "Run Finished", value: "run_finished" },
          { name: "Run Started", value: "run_started" },
          { name: "Run Stopped by Limit", value: "run_stopped_by_limit" },
        ],
        description: "Notification events that fire this trigger",
      },
    ],
  };

  readonly webhookMethods = {
    default: {
      async checkExists(this: IHookFunctions): Promise<boolean> {
        const staticData = this.getWorkflowStaticData("node");
        return (
          typeof staticData["channel"] === "string" &&
          staticData["channel"].length > 0 &&
          typeof staticData["webhookSecret"] === "string" &&
          staticData["webhookSecret"].length > 0
        );
      },

      async create(this: IHookFunctions): Promise<boolean> {
        const node = this.getNode();
        const webhookUrl = this.getNodeWebhookUrl("default");
        if (webhookUrl === undefined || webhookUrl.length === 0) {
          throw createCollieNodeError(
            {
              code: "webhook_url_missing",
              group: "program",
              fix: "Ensure n8n has a reachable webhook URL configured before activating the trigger.",
              message: "n8n did not provide a webhook URL for Goodfetch Collie Trigger.",
            },
            node,
          );
        }
        const channel = readStringParam(
          this.getNodeParameter("channel", "n8n-collie-trigger"),
          "n8n-collie-trigger",
        ).trim();
        const events = readStringArray(this.getNodeParameter("events", []));
        const rawCredentials = await this.getCredentials("goodfetchCollieApi");
        const credentials = parseCredentials(rawCredentials, node);

        const payload: IDataObject = { channel, url: webhookUrl };
        if (events.length > 0) {
          payload["events"] = events;
        }
        const result = await callCollieRpc({
          helpers: this.helpers,
          credentials,
          method: "notification.subscribe",
          params: payload,
          node,
        });

        const staticData = this.getWorkflowStaticData("node");
        staticData["channel"] = typeof result["channel"] === "string" ? result["channel"] : channel;
        if (typeof result["secret"] === "string" && result["secret"].length > 0) {
          staticData["webhookSecret"] = result["secret"];
        } else if (
          typeof staticData["webhookSecret"] !== "string" ||
          staticData["webhookSecret"].length === 0
        ) {
          throw createCollieNodeError(
            {
              code: "webhook_secret_missing",
              group: "program",
              fix: "Unsubscribe and re-create the Notification channel so the Collie Daemon issues a fresh signing secret.",
              message:
                "Collie Daemon reported an existing subscription without a signing secret, and no secret is stored locally.",
            },
            node,
          );
        }
        return true;
      },

      async delete(this: IHookFunctions): Promise<boolean> {
        const node = this.getNode();
        const staticData = this.getWorkflowStaticData("node");
        const channel =
          typeof staticData["channel"] === "string" && staticData["channel"].length > 0
            ? staticData["channel"]
            : readStringParam(
                this.getNodeParameter("channel", "n8n-collie-trigger"),
                "n8n-collie-trigger",
              ).trim();
        const rawCredentials = await this.getCredentials("goodfetchCollieApi");
        const credentials = parseCredentials(rawCredentials, node);

        await callCollieRpc({
          helpers: this.helpers,
          credentials,
          method: "notification.unsubscribe",
          params: { channel },
          node,
        });

        delete staticData["channel"];
        delete staticData["webhookSecret"];
        return true;
      },
    },
  };

  async webhook(this: IWebhookFunctions): Promise<IWebhookResponseData> {
    const req = this.getRequestObject() as { readonly rawBody?: Uint8Array | string | undefined };
    const body = this.getBodyData();
    const headers = this.getHeaderData() as Record<string, string | string[] | undefined>;
    const staticData = this.getWorkflowStaticData("node");
    const secret =
      typeof staticData["webhookSecret"] === "string" ? staticData["webhookSecret"] : undefined;

    // Fail closed whenever no signing secret is stored (S2).
    if (secret === undefined || secret.length === 0) {
      return {};
    }

    let rawBody: string;
    if (typeof req.rawBody === "string") {
      rawBody = req.rawBody;
    } else if (req.rawBody instanceof Uint8Array) {
      rawBody = new TextDecoder().decode(req.rawBody);
    } else {
      rawBody = JSON.stringify(body);
    }
    const valid = await verifyStandardWebhookSignature({
      rawBody,
      headers,
      secret,
    });
    if (!valid) {
      return {};
    }

    const data = isRecord(body["data"]) ? body["data"] : undefined;
    const eventName = typeof data?.["event"] === "string" ? data["event"] : undefined;
    const selectedEvents = readStringArray(this.getNodeParameter("events", []));
    if (
      selectedEvents.length > 0 &&
      eventName !== undefined &&
      !selectedEvents.includes(eventName)
    ) {
      return {};
    }

    const eventId = typeof data?.["event_id"] === "string" ? data["event_id"] : undefined;
    if (eventId !== undefined) {
      const seen = readStringArray(staticData["seenEventIds"]);
      if (seen.includes(eventId)) {
        return {};
      }
      seen.push(eventId);
      if (seen.length > SEEN_EVENT_IDS_CAP) {
        seen.splice(0, seen.length - SEEN_EVENT_IDS_CAP);
      }
      staticData["seenEventIds"] = seen;
    }

    const items = this.helpers.returnJsonArray([body]);
    return {
      workflowData: [items],
    };
  }
}
