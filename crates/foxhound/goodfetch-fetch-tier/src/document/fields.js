// The first element each of a code-free crawl's field locators finds (ADR
// 0123), called in the guard's isolated world of Collie's page: the
// document's base URL, and for each locator, in order, `null` when it finds
// none, else the element's tag name, lower-cased, its text content and its
// attributes. A locator is `{ kind, selector }`: `css`, a CSS selector, or
// `xpath`, an XPath 1.0 expression whose first node in document order is
// read. A selector or expression the page refuses throws.
/* eslint-disable unicorn/consistent-function-scoping -- CDP sends the function alone */
// eslint-disable-next-line no-unused-vars -- `Runtime.callFunctionOn` calls it
function fieldElements({ locators }) {
  const first = ({ kind, selector }) => {
    if (kind === "xpath") {
      const found = document.evaluate(
        selector,
        document,
        null,
        XPathResult.FIRST_ORDERED_NODE_TYPE,
        null,
      ).singleNodeValue;
      return found instanceof Element ? found : null;
    }
    return document.querySelector(selector);
  };
  const elements = locators.map((locator) => {
    const element = first(locator);
    if (!element) return null;
    return {
      tag: element.localName,
      text: element.textContent ?? "",
      attributes: Array.from(element.attributes, (attribute) => [attribute.name, attribute.value]),
    };
  });
  return { baseUrl: document.baseURI, elements };
}
