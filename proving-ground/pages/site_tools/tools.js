// The site-tools page (issue 1540): it registers four WebMCP tools and, half a
// second after the page has loaded, removes `track_parcel` and shows `Ready`.
// `look_up_order` is read-only and answers the order the page holds;
// `place_order` never answers; `send_message`'s input schema is only a `$ref`
// to a URL, which no client should fetch. A browser without WebMCP shows
// `WebMCP unavailable`.
(() => {
  "use strict";
  const status = document.getElementById("status");
  const context = document.modelContext;
  if (!context) {
    status.textContent = "WebMCP unavailable";
    return;
  }
  const order = JSON.parse(document.getElementById("order").textContent);
  const removal = new AbortController();
  const nothing = { type: "object", properties: {}, additionalProperties: false };

  const tools = [
    [
      {
        name: "look_up_order",
        description: "Looks up one of your orders by its ID.",
        inputSchema: {
          type: "object",
          properties: { id: { type: "string" } },
          required: ["id"],
          additionalProperties: false,
        },
        annotations: { readOnlyHint: true },
        execute: async ({ id }) => (id === order.id ? order : { error: "no_such_order" }),
      },
    ],
    [
      {
        name: "place_order",
        description: "Orders the same item again.",
        inputSchema: {
          type: "object",
          properties: { quantity: { type: "integer", minimum: 1 } },
          required: ["quantity"],
          additionalProperties: false,
        },
        execute: () => new Promise(() => {}),
      },
    ],
    [
      {
        name: "track_parcel",
        description: "Tracks the parcel of your order.",
        inputSchema: nothing,
        annotations: { readOnlyHint: true },
        execute: async () => ({ status: "in transit" }),
      },
      { signal: removal.signal },
    ],
    [
      {
        name: "send_message",
        description: "Sends a message to the shop.",
        inputSchema: { $ref: new URL("/schemas/message.json", window.location.href).href },
        execute: async () => ({ sent: true }),
      },
    ],
  ];

  const loaded = new Promise((resolve) => {
    if (document.readyState === "complete") {
      resolve();
    } else {
      window.addEventListener("load", resolve, { once: true });
    }
  });

  const run = async () => {
    await Promise.all(tools.map(([tool, options]) => context.registerTool(tool, options)));
    await loaded;
    await new Promise((resolve) => {
      setTimeout(resolve, 500);
    });
    removal.abort();
    status.textContent = "Ready";
  };
  void run();
})();
