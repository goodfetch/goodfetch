// What makes this package Foxhound's (ADRs 0090 and 0095): the Product its
// `hello` names, its executable, the one engine-path variable it reads and
// its platform packages. Collie's variable is another Product's and is never
// read here. Foxhound contains Collie (ADR 0084), so its Connection raises
// the classes of Collie's Error codes and of its own.
import { collieErrorClasses } from "@goodfetch/collie-client";
import type { ProductPackage } from "@goodfetch/collie-client/runtime";
import { FOXHOUND_ENGINE_PATH_VARIABLE, FOXHOUND_EXECUTABLE } from "./generated/connection.ts";
import { foxhoundErrorClasses } from "./generated/errors.ts";

export const foxhound: ProductPackage = {
  needs: "foxhound",
  name: "Foxhound",
  package: "@goodfetch/foxhound",
  executable: FOXHOUND_EXECUTABLE,
  enginePathVariable: FOXHOUND_ENGINE_PATH_VARIABLE,
  errorClasses: { ...collieErrorClasses, ...foxhoundErrorClasses },
  moduleUrl: import.meta.url,
};
