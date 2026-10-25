// The live board's script: it holds a WebSocket and a server-sent events
// stream open and counts their messages, and makes periodic requests one
// after another, each starting when the one before it is answered, until
// `data-updates` of them are done ("unbounded": never). The board then says
// it settled. With `data-at-load`, the requests start at the document's
// `load` event, and the board waits until then; `data-requests` is the path
// before each request's number (`/updates/` without it). It is the page's
// only script, so its names stay its own.
"use strict";

const show = (id, value) => {
  document.getElementById(id).textContent = String(value);
};

const live = (limit, atLoad, requests) => {
  const last = limit === "unbounded" ? Number.POSITIVE_INFINITY : Number(limit);
  let messages = 0;
  let events = 0;
  const scheme = location.protocol === "https:" ? "wss:" : "ws:";
  const socket = new WebSocket(`${scheme}//${location.host}/socket`);
  socket.addEventListener("message", () => {
    messages += 1;
    show("socket", messages);
  });
  const source = new EventSource("/events");
  source.addEventListener("message", () => {
    events += 1;
    show("events", events);
  });
  const run = async () => {
    show("state", "updating");
    for (let number = 1; number <= last; number += 1) {
      const response = await fetch(`${requests}${number}`, { cache: "no-store" });
      const { reading } = await response.json();
      show("updates", number);
      show("reading", reading);
    }
    show("state", "settled");
  };
  const start = () => {
    run().catch(() => show("state", "failed"));
  };
  if (atLoad === undefined) {
    start();
  } else {
    show("state", "waiting");
    addEventListener("load", start);
  }
};

const { updates, atLoad, requests = "/updates/" } = document.currentScript.dataset;
live(updates, atLoad, requests);
