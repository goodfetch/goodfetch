// The components a page's Markdown renders with: the theme's own, with each
// table in a group a keyboard can focus and scroll sideways (WCAG 2.1.1),
// since a table wider than the page scrolls inside it. A group, not a
// region: a page's regions are landmarks, each needing its own name.
import MDXComponents from "@theme-original/MDXComponents";
import type { ComponentProps, ReactNode } from "react";

function Table(props: ComponentProps<"table">): ReactNode {
  return (
    <div className="table-scroll" role="group" aria-label="Table" tabIndex={0}>
      <table {...props} />
    </div>
  );
}

const components: typeof MDXComponents = { ...MDXComponents, table: Table };

export default components;
