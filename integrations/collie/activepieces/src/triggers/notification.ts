// Activepieces trigger: `notification` (`notification.subscribe` /
// `notification.unsubscribe` + Standard Webhooks verification and single-fire
// deduplication by `event_id`; spec #608, issue 1409).
import { verifyNotificationDelivery } from "@goodfetch/collie-integrations-core";
import type { ActivepiecesCollieAuth, TriggerDefinition } from "../ap-types.ts";
import { createCollieClient, goodfetchCollieAuth, toActivepiecesError } from "../common/client.ts";

const STORE_KEY_CHANNEL = "goodfetch_collie_channel";
const STORE_KEY_SECRET = "goodfetch_collie_secret";
const STORE_KEY_SEEN = "goodfetch_collie_seen_event_ids";
const SEEN_CAP = 1024;

export interface NotificationTriggerProps extends Record<string, unknown> {
  readonly channel: string;
  readonly events?: readonly string[];
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

export const notificationTrigger: TriggerDefinition<
  ActivepiecesCollieAuth,
  NotificationTriggerProps
> = {
  name: "notification",
  displayName: "Collie Notification",
  description: "Fire when a Collie Run emits a selected Notification event.",
  type: "WEBHOOK",
  auth: goodfetchCollieAuth,
  props: {
    channel: {
      displayName: "Channel Name",
      description: "Stable Notification subscription channel identifier on the Collie Daemon.",
      type: "SHORT_TEXT",
      required: true,
      defaultValue: "activepieces-collie-trigger",
    },
    events: {
      displayName: "Events",
      description: "Notification events that fire this trigger.",
      type: "STATIC_MULTI_SELECT_DROPDOWN",
      required: false,
      defaultValue: ["run_finished", "run_failed"],
      options: [
        { label: "Run Started", value: "run_started" },
        { label: "Run Finished", value: "run_finished" },
        { label: "Run Failed", value: "run_failed" },
        { label: "Run Stopped by Limit", value: "run_stopped_by_limit" },
        { label: "Approval Requested", value: "approval_requested" },
        { label: "Human Pause Waiting", value: "human_pause_waiting" },
      ],
    },
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
      const client = createCollieClient(context.auth, context.fetchImpl);
      const channel = context.propsValue.channel.trim();
      const events = Array.isArray(context.propsValue.events)
        ? context.propsValue.events.filter(
            (e): e is string => typeof e === "string" && e.length > 0,
          )
        : undefined;
      const result = await client.subscribeNotification({
        channel,
        url: context.webhookUrl,
        ...(events === undefined || events.length === 0 ? {} : { events }),
      });
      await context.store.put(STORE_KEY_CHANNEL, result.channel);
      if (result.secret !== undefined) {
        await context.store.put(STORE_KEY_SECRET, result.secret);
      }
    } catch (error) {
      throw toActivepiecesError(error);
    }
  },
  async onDisable(context) {
    try {
      const client = createCollieClient(context.auth, context.fetchImpl);
      const storedChannel = await context.store.get(STORE_KEY_CHANNEL);
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
    const secret = await context.store.get(STORE_KEY_SECRET);
    const bodyObj = isRecord(context.payload.body) ? context.payload.body : undefined;
    if (bodyObj === undefined) {
      return [];
    }

    if (typeof secret === "string" && secret.length > 0) {
      const rawBody = context.payload.rawBody ?? JSON.stringify(bodyObj);
      try {
        await verifyNotificationDelivery(
          rawBody,
          normalizeHeaders(context.payload.headers),
          secret,
        );
      } catch {
        return [];
      }
    }

    const data = isRecord(bodyObj["data"]) ? bodyObj["data"] : undefined;
    const eventName = typeof data?.["event"] === "string" ? data["event"] : undefined;
    const selectedEvents = Array.isArray(context.propsValue.events)
      ? context.propsValue.events.filter((e): e is string => typeof e === "string" && e.length > 0)
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
      const rawSeen = await context.store.get(STORE_KEY_SEEN);
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
};
