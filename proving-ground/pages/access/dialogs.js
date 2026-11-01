// The dialogs page (#719): a button for each JavaScript dialog a page
// opens, `alert`, `confirm` and `prompt`, each reporting what its dialog
// returned under the button's ID once it closed: `closed` for an alert,
// `true` or `false` for a confirm, and for a prompt the text it returned
// as JSON, `null` when it was dismissed. `#guard-leaving` arms a
// `beforeunload` handler that asks to stay, which a browser shows as a
// dialog only once the page had a click, as that button's own gives it.
(() => {
  "use strict";
  const script = document.currentScript;
  const reports = script.getAttribute("data-reports");
  const page = window.location.pathname;
  const main = document.querySelector("main");

  const send = (check, observed) =>
    fetch(reports, {
      method: "POST",
      body: JSON.stringify({ page, check, observed }),
      keepalive: true,
    }).catch(() => {});

  const button = (id, run) => {
    const element = document.createElement("button");
    element.id = id;
    element.type = "button";
    element.textContent = id;
    element.addEventListener("click", () => {
      void send(id, run());
    });
    main.append(element);
  };

  let guarded = false;
  window.addEventListener("beforeunload", (event) => {
    if (guarded) {
      event.preventDefault();
    }
  });

  button("alert", () => {
    window.alert("An alert");
    return "closed";
  });
  button("confirm", () => String(window.confirm("Confirm this?")));
  button("prompt", () => JSON.stringify(window.prompt("Your name?", "a default")));
  button("guard-leaving", () => {
    guarded = true;
    return "guarded";
  });
})();
