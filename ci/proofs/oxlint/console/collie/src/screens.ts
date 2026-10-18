// Collie's Console reaching Foxhound's screens through a relative path, which
// the Console's lint configuration refuses (ADR 0087).
import { datasets } from "../../foxhound/src/datasets.ts";

export const screens: readonly string[] = [datasets];
