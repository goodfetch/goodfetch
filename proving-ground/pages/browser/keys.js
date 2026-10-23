// The key recorder's script: it logs every key, input and composition
// event the field receives, in order (type, key, code, data, input type,
// composition state, trusted), and shows the keyboard layout when the
// browser tells it. It reports to the site each key press of one
// character (its key and code), each input outside a composition (the
// field's value after it) and each composition's end (the text it
// committed), with the browser's `isTrusted`; the root element's
// `data-posting` counts the reports in flight.
(() => {
  "use strict";
  const root = document.documentElement;
  const field = document.getElementById("field");
  const log = document.getElementById("log");
  let posting = 0;
  const post = (effect) => {
    posting += 1;
    root.dataset.posting = String(posting);
    fetch("/effects", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(effect),
    })
      .catch(() => {})
      .finally(() => {
        posting -= 1;
        root.dataset.posting = String(posting);
      });
  };
  root.dataset.posting = "0";
  const report = (event, trusted, details) => post({ event, target: "field", trusted, details });
  const types = [
    "keydown",
    "keypress",
    "keyup",
    "beforeinput",
    "input",
    "compositionstart",
    "compositionupdate",
    "compositionend",
  ];
  // Chromium marks the end of a composition untrusted when a commit ends
  // it, as CDP's `Input.insertText` does, so a composition's end is
  // reported with its start's `isTrusted`.
  let compositionTrusted = false;
  for (const type of types) {
    field.addEventListener(type, (event) => {
      if (type === "compositionstart") {
        compositionTrusted = event.isTrusted;
      }
      const entry = document.createElement("li");
      const parts = [
        type,
        `key=${event.key ?? ""}`,
        `code=${event.code ?? ""}`,
        `data=${event.data ?? ""}`,
        `inputType=${event.inputType ?? ""}`,
        `composing=${String(event.isComposing ?? false)}`,
        `trusted=${String(event.isTrusted)}`,
      ];
      entry.textContent = parts.join(" ");
      log.append(entry);
      const trusted = event.isTrusted;
      if (type === "keydown" && [...event.key].length === 1) {
        report(type, trusted, { key: event.key, code: event.code });
      } else if (type === "input" && !event.isComposing) {
        report(type, trusted, { value: field.value });
      } else if (type === "compositionend") {
        report(type, compositionTrusted, { data: event.data });
      }
    });
  }
  // The layout's first letters (`qwerty`, `azerty`, `qwertz`), where the
  // browser tells it: the Keyboard API, in a secure context only.
  const shownLayout = document.getElementById("layout");
  const showLayout = async () => {
    const layout = await navigator.keyboard.getLayoutMap();
    shownLayout.textContent = ["KeyQ", "KeyW", "KeyE", "KeyR", "KeyT", "KeyY"]
      .map((code) => layout.get(code) ?? "?")
      .join("");
  };
  if (typeof navigator.keyboard?.getLayoutMap === "function") {
    showLayout().catch(() => {});
  }
})();
