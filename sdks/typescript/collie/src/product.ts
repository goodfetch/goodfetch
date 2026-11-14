// What makes this package Collie's (ADRs 0090 and 0095): the Product its
// `hello` names, its executable, the one engine-path variable it reads and
// its platform packages. Foxhound's variable is another Product's and is
// never read here.
import { collieErrorClasses } from "@goodfetch/collie-client";
import {
  COLLIE_ENGINE_PATH_VARIABLE,
  COLLIE_EXECUTABLE,
  type ProductPackage,
} from "@goodfetch/collie-client/runtime";

export const collie: ProductPackage = {
  needs: "collie",
  name: "Collie",
  package: "@goodfetch/collie",
  executable: COLLIE_EXECUTABLE,
  enginePathVariable: COLLIE_ENGINE_PATH_VARIABLE,
  errorClasses: collieErrorClasses,
  moduleUrl: import.meta.url,
};
