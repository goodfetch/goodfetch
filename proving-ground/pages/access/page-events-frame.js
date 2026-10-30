// The page events page's frame (#703), on another site than its page, so
// the browser runs it out of process: it requests the page events page's
// JSON body from its own site, writes a console message once it read it,
// then throws an exception it does not catch.
(() => {
  "use strict";
  void fetch("/page-events/data.json")
    .then((response) => response.text())
    .then(() => {
      console.log("page-events frame");
      setTimeout(() => {
        throw new TypeError("page-events frame uncaught");
      }, 0);
    });
})();
