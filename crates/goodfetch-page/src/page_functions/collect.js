// One step's read of a scroll collection (`locator.scroll_collect`, #807),
// evaluated in the world its page function runs in: the engine sends
// `(collect)(source)` as the page function the walker (`walk.js`) calls with
// every match of the locator (`targets.js`) and the argument. `read`, the
// program's function, is called with each match and the argument, in
// document order, and answers the row's `{ key, value }`, `key` a string, or
// `null` to skip the match; any other answer throws a `TypeError`, which the
// walker reports as the page function's. It answers the rows, how many
// elements matched, and the rows the list declares: the `aria-rowcount` of
// the first match's nearest ancestor that has one, else the first match's
// `aria-setsize`, `null` for none or a value that is no integer. Its
// answers have no prototype, so a page's `Object.prototype` cannot answer
// for them.
// oxlint-disable-next-line no-unused-vars -- the engine's expression calls it
function collect(read) {
  return async (elements, argument) => {
    if (typeof read !== "function") {
      throw new TypeError(
        "a scroll collection's page function is a function, such as `(row, arg) => ({ key, value })`",
      );
    }
    // oxlint-disable-next-line unicorn/consistent-function-scoping -- CDP sends `collect` alone
    const described = (row) => {
      if (row === undefined) {
        return "undefined";
      }
      return typeof row === "object" ? "an object without a string `key`" : `a ${typeof row}`;
    };
    const rows = [];
    for (let index = 0; index < elements.length; index += 1) {
      const row = await read(elements[index], argument);
      if (row === null) {
        continue;
      }
      if (typeof row !== "object" || typeof row.key !== "string") {
        throw new TypeError(
          `the page function answered ${described(row)} for match ${index}: answer \`{ key, value }\`, \`key\` a string, or \`null\` to skip the match`,
        );
      }
      rows[rows.length] = { __proto__: null, key: row.key, value: row.value };
    }
    const INTEGER = /^\s*-?\d+\s*$/u;
    const declaredBy = (element, attribute) => {
      const text = element?.getAttribute(attribute);
      return typeof text === "string" && INTEGER.test(text) && Number.isSafeInteger(+text)
        ? +text
        : null;
    };
    const first = elements[0];
    const declared =
      first === undefined
        ? null
        : (declaredBy(first.closest("[aria-rowcount]"), "aria-rowcount") ??
          declaredBy(first, "aria-setsize"));
    return { __proto__: null, rows, matched: elements.length, declared };
  };
}
