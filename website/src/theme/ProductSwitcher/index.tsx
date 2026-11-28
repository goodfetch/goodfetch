// The header's switcher between the Products (ADR 0097): each Product's
// link leads to the page being read in that Product's section, or to its
// home when the section lacks it, in the version being read.
import Link from "@docusaurus/Link";
import { useActiveDocContext, useLatestVersion } from "@docusaurus/plugin-content-docs/client";
import type { ReactNode } from "react";
import { products, sectionOf, switchTarget } from "../../product-switch.ts";

const labels = { collie: "Collie", foxhound: "Foxhound" } as const;

export default function ProductSwitcher({ mobile }: { mobile?: boolean }): ReactNode {
  const { activeVersion, activeDoc } = useActiveDocContext(undefined);
  const latest = useLatestVersion(undefined);
  const version = activeVersion ?? latest;
  const current = sectionOf(activeDoc?.id);
  return (
    <nav
      aria-label="Product"
      className={mobile === true ? "menu__list-item product-switcher" : "product-switcher"}
    >
      {products.map((product) => (
        <Link
          key={product}
          to={switchTarget(version.docs, activeDoc?.id, product)}
          aria-current={product === current ? "page" : undefined}
          className={
            mobile === true
              ? "menu__link product-switcher__link"
              : "navbar__item navbar__link product-switcher__link"
          }
        >
          {labels[product]}
        </Link>
      ))}
    </nav>
  );
}
