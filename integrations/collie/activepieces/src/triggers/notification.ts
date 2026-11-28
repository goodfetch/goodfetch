// Activepieces trigger: `notification` (`notification.subscribe` /
// `notification.unsubscribe` + fail-closed Standard Webhooks verification and
// single-fire deduplication by `event_id`; spec #608, issue 1409).
// Built with `@activepieces/pieces-framework`'s `createTrigger`.
import {
  createTrigger,
  Property,
  TriggerStrategy,
  type ShortTextProperty,
  type StaticMultiSelectDropdownProperty,
  type Trigger,
} from "@activepieces/pieces-framework";
import { verifyNotificationDelivery } from "@goodfetch/collie-integrations-core";
import {
  ActivepiecesCollieError,
  createCollieClient,
  goodfetchCollieAuth,
  toActivepiecesError,
} from "../common/client.ts";

const STORE_KEY_CHANNEL = "goodfetch_collie_channel";
const STORE_KEY_SECRET = "goodfetch_collie_secret";
const STORE_KEY_SEEN = "goodfetch_collie_seen_event_ids";
const SEEN_CAP = 1024;

export interface NotificationTriggerProps extends Record<string, unknown> {
  readonly channel: string;
  readonly events?: readonly string[] | undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function normalizeHeaders(
  headers: Record<string, string | string[] | undefined>,
): Record<string, string | undefined> {
  const out: Record<string, string | undefined> = {};
  for (const [key, value] of Object.entries(headers)) {
    if (typeof value === "string") {
      out[key] = value;
    } else if (Array.isArray(value) && value.length > 0) {
      out[key] = value[0];
    }
  }
  return out;
}

export const notificationTrigger: Trigger<
  typeof goodfetchCollieAuth,
  {
    channel: ShortTextProperty<true>;
    events: StaticMultiSelectDropdownProperty<string, false>;
  },
  TriggerStrategy
> = createTrigger({
  auth: goodfetchCollieAuth,
  name: "notification",
  displayName: "Collie Notification",
  description: "Fire when a Collie Run emits a selected Notification event.",
  type: TriggerStrategy.WEBHOOK,
  props: {
    channel: Property.ShortText({
      displayName: "Channel Name",
      description: "Stable Notification subscription channel identifier on the Collie Daemon.",
      required: true,
      defaultValue: "activepieces-collie-trigger",
    }),
    events: Property.StaticMultiSelectDropdown({
      displayName: "Events",
      description: "Notification events that fire this trigger.",
      required: false,
      defaultValue: ["run_finished", "run_failed"],
      options: {
        options: [
          { label: "Run Started", value: "run_started" },
          { label: "Run Finished", value: "run_finished" },
          { label: "Run Failed", value: "run_failed" },
          { label: "Run Stopped by Limit", value: "run_stopped_by_limit" },
          { label: "Approval Requested", value: "approval_requested" },
          { label: "Human Pause Waiting", value: "human_pause_waiting" },
        ],
      },
    }),
  },
  sampleData: {
    type: "notification.run_finished",
    format: "0.1",
    data: {
      event_id: "evt_01JACTIVEPIECESSAMPLE0001",
      event: "run_finished",
      run_id: "r_01JACTIVEPIECESSAMPLE00001",
      schedule_id: null,
      product: "collie",
      link: null,
    },
  },
  async onEnable(context) {
    try {
      const client = createCollieClient(context.auth);
      const channel = context.propsValue.channel.trim();
      const rawEvents = context.propsValue.events;
      const events = Array.isArray(rawEvents)
        ? rawEvents.filter((e): e is string => typeof e === "string" && e.length > 0)
        : undefined;
      const result = await client.subscribeNotification({
        channel,
        url: context.webhookUrl,
        ...(events === undefined || events.length === 0 ? {} : { events }),
      });
      await context.store.put(STORE_KEY_CHANNEL, result.channel);
      if (result.secret !== undefined && result.secret.length > 0) {
        await context.store.put(STORE_KEY_SECRET, result.secret);
      } else {
        const existingSecret = await context.store.get<string>(STORE_KEY_SECRET);
        if (typeof existingSecret !== "string" || existingSecret.length === 0) {
          throw new ActivepiecesCollieError({
            code: "webhook_secret_missing",
            group: "program",
            fix: "Unsubscribe and re-create the Notification channel so the Collie Daemon issues a fresh signing secret.",
            message:
              "Collie Daemon reported an existing subscription without a signing secret, and no secret is stored locally.",
          });
        }
      }
    } catch (error) {
      throw toActivepiecesError(error);
    }
  },
  async onDisable(context) {
    try {
      const client = createCollieClient(context.auth);
      const storedChannel = await context.store.get<string>(STORE_KEY_CHANNEL);
      const channel =
        typeof storedChannel === "string" && storedChannel.length > 0
          ? storedChannel
          : context.propsValue.channel.trim();
      await client.unsubscribeNotification(channel);
      await context.store.delete(STORE_KEY_CHANNEL);
      await context.store.delete(STORE_KEY_SECRET);
    } catch (error) {
      throw toActivepiecesError(error);
    }
  },
  async run(context) {
    const secret = await context.store.get<string>(STORE_KEY_SECRET);
    // Fail closed whenever no signing secret is stored (S2).
    if (typeof secret !== "string" || secret.length === 0) {
      return [];
    }

    const bodyObj = isRecord(context.payload.body) ? context.payload.body : undefined;
    if (bodyObj === undefined) {
      return [];
    }

    const rawBody =
      typeof context.payload.rawBody === "string"
        ? context.payload.rawBody
        : context.payload.rawBody instanceof Uint8Array
          ? new TextDecoder().decode(context.payload.rawBody)
          : JSON.stringify(bodyObj);
    try {
      await verifyNotificationDelivery(rawBody, normalizeHeaders(context.payload.headers), secret);
    } catch {
      return [];
    }

    const data = isRecord(bodyObj["data"]) ? bodyObj["data"] : undefined;
    const eventName = typeof data?.["event"] === "string" ? data["event"] : undefined;
    const rawEvents = context.propsValue.events;
    const selectedEvents = Array.isArray(rawEvents)
      ? rawEvents.filter((e): e is string => typeof e === "string" && e.length > 0)
      : [];
    if (
      selectedEvents.length > 0 &&
      eventName !== undefined &&
      !selectedEvents.includes(eventName)
    ) {
      return [];
    }

    const eventId = typeof data?.["event_id"] === "string" ? data["event_id"] : undefined;
    if (eventId !== undefined) {
      const rawSeen = await context.store.get<string[]>(STORE_KEY_SEEN);
      const existing = Array.isArray(rawSeen)
        ? rawSeen.filter((item): item is string => typeof item === "string")
        : [];
      if (existing.includes(eventId)) {
        return [];
      }
      const next = [...existing, eventId];
      if (next.length > SEEN_CAP) {
        next.splice(0, next.length - SEEN_CAP);
      }
      await context.store.put(STORE_KEY_SEEN, next);
    }

    return [bodyObj];
  },
});
