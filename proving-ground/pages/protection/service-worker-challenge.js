// The service-worker Challenge (#1539): it registers the worker, waits until
// the worker controls the page, or for 5 s, and posts the check, which a
// controlling worker answers with its proof. A passed check loads the page
// again; a refused one titles the document `refused`.
(() => {
  "use strict";
  const container = navigator.serviceWorker;

  const controlled = () =>
    new Promise((resolve) => {
      if (container.controller) {
        resolve();
        return;
      }
      container.addEventListener("controllerchange", resolve, { once: true });
      setTimeout(resolve, 5000);
    });

  const register = async () => {
    try {
      await container.register("/service-worker.js", { scope: "/service-worker" });
      await controlled();
    } catch {
      // A browser without service workers, or one that refuses them, posts
      // the check itself.
    }
  };

  const run = async () => {
    await register();
    const check = await fetch("/service-worker/check", { method: "POST" });
    if (check.ok) {
      window.location.reload();
    } else {
      document.title = "refused";
    }
  };
  void run();
})();
