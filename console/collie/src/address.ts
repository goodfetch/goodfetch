// The page's address, as a screen that keeps its view there reads and
// replaces it (ADR 0117: a filter kept in the address can be shared). The
// build entry passes the window's; tests pass their own.

/** The query of the page's address, which a screen reads and replaces. */
export interface Address {
  /** The query, `?…`, or empty. */
  readonly search: string;
  /** Replaces the query with `search`, adding no history entry. */
  replace(search: string): void;
}

/** The address of a page with no view to keep. */
export const NO_ADDRESS: Address = { search: "", replace: () => {} };

/** The address of the window `location` and `history` belong to. */
export function windowAddress(location: Location, history: History): Address {
  return {
    get search() {
      return location.search;
    },
    replace(search: string) {
      history.replaceState(history.state, "", `${location.pathname}${search}${location.hash}`);
    },
  };
}
