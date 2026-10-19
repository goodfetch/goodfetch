// The tier-move site's Challenge: it computes the clearance from the page's
// nonce (each character folded into 7 as `hash * 31 + code`, kept to 32
// bits, in lowercase hex), sets it as a cookie and loads the page again. A
// browser runs it; a plain HTTP client never does.
(function () {
  "use strict";
  var nonce = document.currentScript.getAttribute("data-nonce");
  var hash = 7;
  for (var at = 0; at < nonce.length; at += 1) {
    hash = (hash * 31 + nonce.charCodeAt(at)) >>> 0;
  }
  document.cookie = "clearance=" + hash.toString(16) + "; path=/; SameSite=Lax";
  window.location.reload();
})();
