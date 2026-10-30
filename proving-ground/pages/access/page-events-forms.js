// The page events forms page (#703): beside its sign-in form, which posts
// its fields as a browser posts a form, urlencoded, to
// `/page-events/session`, and its search form, whose query carries its
// field, a button posts the sign-in form's fields, urlencoded the same
// way, to `/page-events/session` on `elsewhere.test`, which a Run's egress
// policy may refuse.
(() => {
  "use strict";
  document.querySelector("#away").addEventListener("click", () => {
    const body = new URLSearchParams(new FormData(document.querySelector("#sign-in")));
    void fetch(`http://elsewhere.test:${location.port}/page-events/session`, {
      method: "POST",
      body,
      mode: "no-cors",
    }).catch(() => null);
  });
})();
