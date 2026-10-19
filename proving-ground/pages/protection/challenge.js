// The tier-move site's Challenge: it computes the clearance from the page's
// nonce (each character folded into 7 as `hash * 31 + code`, kept to 32
// bits, in lowercase hex), sets it as a cookie and loads the page again. A
// browser runs it; a plain HTTP client never does.
(() => {
  "use strict";
  const nonce = document.currentScript.getAttribute("data-nonce");
  let hash = 7;
  for (const character of nonce) {
    hash = (hash * 31 + character.charCodeAt(0)) >>> 0;
  }
  document.cookie = `clearance=${hash.toString(16)}; path=/; SameSite=Lax`;
  window.location.reload();
})();
