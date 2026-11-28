// The navbar item types, with the Product switcher (ADR 0097) beside the
// theme's own.
import ComponentTypes from "@theme-original/NavbarItem/ComponentTypes";
import type { ComponentTypesObject } from "@theme/NavbarItem/ComponentTypes";
import ProductSwitcher from "../ProductSwitcher/index.tsx";

const types: ComponentTypesObject = {
  ...ComponentTypes,
  "custom-productSwitcher": ProductSwitcher,
};

export default types;
