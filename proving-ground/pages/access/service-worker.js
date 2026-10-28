// The service worker page (#707): it registers `sw.js`, with the page's
// own query, which answers `cached.json` from its Cache Storage and passes
// `network.json` on with its own fetch, and makes three requests of its
// own as it installs (`own.json`, a body on `elsewhere.test` and a path a
// tracker list blocks). Once the worker controls the page, or registration
// failed, the page fetches both JSON calls, asks the worker to make its
// held request (which it makes only when its query holds `held`), then
// reports whether `register()` resolved, whether `register` is the
// browser's own function and whether a worker controls the page. The
// document's title is `done` once it reported.
(() => {
  "use strict";
  const script = document.currentScript;
  const reports = script.getAttribute("data-reports");
  const page = window.location.pathname + window.location.search;
  const container = navigator.serviceWorker;

  const send = (check, observed) =>
    fetch(reports, {
      method: "POST",
      body: JSON.stringify({ page, check, observed }),
      keepalive: true,
    }).catch(() => {});

  const controlled = () =>
    container.controller
      ? Promise.resolve()
      : new Promise((resolve) => {
          container.addEventListener("controllerchange", resolve, { once: true });
        });

  const run = async () => {
    // oxlint-disable-next-line typescript/unbound-method -- reads the method's source text, never calls it
    const native = String(container && container.register).includes("[native code]");
    let registered = "registered";
    try {
      await container.register(`sw.js${window.location.search}`);
      await controlled();
    } catch (error) {
      registered = `rejected ${error.name}`;
    }
    for (const call of ["cached.json", "network.json"]) {
      try {
        await (await fetch(`${call}${window.location.search}`)).text();
      } catch {
        // A failed call is what the case reads.
      }
    }
    if (container && container.controller) {
      container.controller.postMessage("hold");
    }
    await send("register", registered);
    await send("register_native", String(native));
    await send("controlled", String(Boolean(container && container.controller)));
    document.title = "done";
  };
  void run();
})();
