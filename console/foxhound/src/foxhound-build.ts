// Foxhound's Console build (spec #593; ADRs 0087, 0094 and 0098): Collie's
// screens, every route of Collie's build unchanged, composed with
// Foxhound's views of a run that needs Foxhound, which a run's page shows
// in tabs beside the views every run has.
import type { ConsoleBuild } from "@goodfetch/console/console-build";
import { FOXHOUND_RUN_VIEWS } from "#runs/run-views.tsx";

/** Foxhound's build. */
export const FOXHOUND_BUILD: ConsoleBuild = { product: "foxhound", runViews: FOXHOUND_RUN_VIEWS };
