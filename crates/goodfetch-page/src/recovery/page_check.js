// What the Page check reads of the main frame's document (ADRs 0006 and
// 0092), called in the guard's isolated world, which no page script
// reaches: whether each of `matches`, a CSS selector, an XPath expression
// or a text, finds anything, `null` for a selector or an expression the
// document refuses; and whether the document shows one password field that
// takes no new password, the built-in detector's sign of a login form; and
// the `content` of each robots `meta` element, which says whether the
// document is marked `noindex` (ADR 0035). A
// text matches the body's text with runs of white space read as one space.
// eslint-disable-next-line no-unused-vars -- `Runtime.callFunctionOn` calls it
function pageCheck(argument) {
  const body = document.body === null ? "" : document.body.innerText.replace(/\s+/g, " ");
  const found = (match) => {
    try {
      switch (match.kind) {
        case "css":
          return document.querySelector(match.selector) !== null;
        case "xpath":
          return (
            document.evaluate(
              match.selector,
              document,
              null,
              XPathResult.FIRST_ORDERED_NODE_TYPE,
              null,
            ).singleNodeValue !== null
          );
        case "text":
          return body.includes(match.text.replace(/\s+/g, " ").trim());
        default:
          return null;
      }
    } catch {
      return null;
    }
  };
  const passwords = [...document.querySelectorAll("input[type=password]")];
  const password = passwords.length === 1 ? passwords[0] : null;
  const box = password === null ? null : password.getBoundingClientRect();
  return {
    found: argument.matches.map(found),
    password:
      password !== null &&
      box.width > 0 &&
      box.height > 0 &&
      getComputedStyle(password).visibility === "visible" &&
      password.autocomplete !== "new-password",
    robots: [...document.querySelectorAll("meta[name]")]
      .filter((meta) => meta.name.toLowerCase() === "robots")
      .map((meta) => meta.content),
  };
}
