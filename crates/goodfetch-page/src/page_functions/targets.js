// The matches of a locator's page function (ADR 0085), evaluated in the world
// the page function runs in: the engine sends `(targets)(selector, one)`, whose
// result the walker calls before the page function. Its CSS matches inside
// open shadow roots too, as the guard's does. It answers the array of every
// match, or for `one` the one match, as `target`; for `one`, `missing` with
// the number of matches when they are not one; and `unparsed` for a selector
// the page does not parse. Its answers have no prototype, so a page's
// `Object.prototype` cannot answer for them.
// eslint-disable-next-line no-unused-vars -- the engine's expression calls it
function targets(selector, one) {
  return () => {
    const found = [];
    const visit = (root) => {
      const matches = root.querySelectorAll(selector);
      for (let index = 0; index < matches.length; index += 1) {
        found[found.length] = matches[index];
      }
      const all = root.querySelectorAll("*");
      for (let index = 0; index < all.length; index += 1) {
        if (all[index].shadowRoot) {
          visit(all[index].shadowRoot);
        }
      }
    };
    try {
      visit(document);
    } catch (error) {
      return { __proto__: null, unparsed: { selector, message: String(error?.message) } };
    }
    if (!one) {
      return { __proto__: null, target: found };
    }
    return found.length === 1
      ? { __proto__: null, target: found[0] }
      : { __proto__: null, missing: found.length };
  };
}
