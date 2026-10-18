// Collie's Console build importing Foxhound's screens' package, which it does
// not declare: the build fails to resolve it.
import { datasets } from "@goodfetch/console-foxhound-screens";

export const screens: readonly string[] = [datasets];
