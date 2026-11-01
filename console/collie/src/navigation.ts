// Moving between the Console's screens without loading the page again
// (ADR 0031): a page whose Daemon stopped cannot load any page, so the
// Console follows its own links itself, and a screen it opens with no
// connection shows what the offline cache keeps. A click on a link to the
// Console's own origin pushes the link's address onto the history and tells
// the page; going back or forward tells it too. A link to another origin,
// to another tab or window, to a download, or clicked with a modifier or
// another button is left to the browser.

/** The parts of `window` following links reads and changes. */
export type NavigatedWindow = Pick<
  Window,
  "addEventListener" | "removeEventListener" | "location" | "history"
>;

/** The link a click landed on, if any. */
function linkOf(target: EventTarget | null): HTMLAnchorElement | undefined {
  const element = target instanceof Element ? target.closest("a[href]") : null;
  return element instanceof HTMLAnchorElement ? element : undefined;
}

/** Follows the Console's own links in `window`, calling `navigated` after each move; the function it answers stops it. */
export function followLinks(window: NavigatedWindow, navigated: () => void): () => void {
  const clicked = (event: MouseEvent): void => {
    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    ) {
      return;
    }
    const link = linkOf(event.target);
    if (
      link === undefined ||
      (link.target !== "" && link.target !== "_self") ||
      link.hasAttribute("download")
    ) {
      return;
    }
    const url = new URL(link.href);
    if (url.origin !== window.location.origin) {
      return;
    }
    event.preventDefault();
    window.history.pushState(null, "", `${url.pathname}${url.search}${url.hash}`);
    navigated();
  };
  const popped = (): void => navigated();
  window.addEventListener("click", clicked);
  window.addEventListener("popstate", popped);
  return () => {
    window.removeEventListener("click", clicked);
    window.removeEventListener("popstate", popped);
  };
}
