// A data table's own sideways scroll (ADR 0073, WCAG 1.4.10 and 2.1.1): at
// 320 CSS pixels a wide table scrolls inside this region, never the page,
// and the keyboard reaches the region to scroll it.
import type { ReactNode } from "react";

/** A region named `label` that scrolls `children`, a data table, sideways. */
export function ScrollRegion({
  label,
  children,
}: {
  readonly label: string;
  readonly children: ReactNode;
}): ReactNode {
  return (
    <section
      aria-label={label}
      tabIndex={0}
      className="overflow-x-auto rounded-sm focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none"
    >
      {children}
    </section>
  );
}
