// The elements each of a page's declared fields finds (ADRs 0123 and
// 0147), called in the guard's isolated world of Collie's page with
// `{ fields }`, each `{ name, kind, locator }`, `kind` being `one`, `list`
// or `rows`, and rows' `fields` each `{ name, locator }`. A locator is
// `{ kind, selector }`: `css`, a CSS selector, or `xpath`, an XPath 1.0
// expression, of whose nodes those that are elements are found, in
// document order. Every locator is checked first, on an empty element
// outside the document (XPath takes no fragment as its context), so
// that one the page refuses answers `{ invalid: { field, kind, selector,
// message } }` whatever the page holds, a row's field named `rows/field`.
// Otherwise it answers `{ baseUrl, found }`, each field's in order: a value
// field's first element or `null`; a list's every element; rows' every
// element their locator finds, each the list of its fields' first element
// among its descendants or `null`. An element is its tag name, lower-cased,
// its text content and its attributes.
// oxlint-disable-next-line no-unused-vars -- `Runtime.callFunctionOn` calls it
function readFields({ fields }) {
  // oxlint-disable-next-line unicorn/consistent-function-scoping -- CDP sends `readFields` alone
  const all = ({ kind, selector }, from) => {
    if (kind === "xpath") {
      const snapshot = document.evaluate(
        selector,
        from,
        null,
        XPathResult.ORDERED_NODE_SNAPSHOT_TYPE,
        null,
      );
      const found = [];
      for (let index = 0; index < snapshot.snapshotLength; index += 1) {
        const node = snapshot.snapshotItem(index);
        if (node instanceof Element) found.push(node);
      }
      return found;
    }
    return Array.from(from.querySelectorAll(selector));
  };
  // oxlint-disable-next-line unicorn/consistent-function-scoping -- CDP sends `readFields` alone
  const first = ({ kind, selector }, from) => {
    if (kind === "xpath") {
      const node = document.evaluate(
        selector,
        from,
        null,
        XPathResult.FIRST_ORDERED_NODE_TYPE,
        null,
      ).singleNodeValue;
      return node instanceof Element ? node : null;
    }
    return from.querySelector(selector);
  };
  const empty = document.createElement("div");
  for (const field of fields) {
    const named = [
      [field.name, field.locator],
      ...(field.fields ?? []).map((each) => [`${field.name}/${each.name}`, each.locator]),
    ];
    for (const [name, locator] of named) {
      try {
        all(locator, empty);
      } catch (error) {
        return {
          invalid: {
            field: name,
            kind: locator.kind,
            selector: locator.selector,
            message: String(error?.message ?? error),
          },
        };
      }
    }
  }
  // oxlint-disable-next-line unicorn/consistent-function-scoping -- CDP sends `readFields` alone
  const read = (element) =>
    element && {
      tag: element.localName,
      text: element.textContent ?? "",
      attributes: Array.from(element.attributes, (attribute) => [attribute.name, attribute.value]),
    };
  const found = fields.map((field) => {
    if (field.kind === "one") return read(first(field.locator, document));
    if (field.kind === "list") return all(field.locator, document).map(read);
    return all(field.locator, document).map((row) =>
      field.fields.map((each) => read(first(each.locator, row))),
    );
  });
  return { baseUrl: document.baseURI, found };
}
