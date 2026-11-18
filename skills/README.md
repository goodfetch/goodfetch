# Agent skills

The agent skills Goodfetch publishes for its Products. Their area fills this folder.

Each skill is a folder holding a `SKILL.md`, in the Agent Skills format, with its references beside it: one source per topic, rendered for each Product that carries it (ADRs 0095 and 0097). Collie's topics sit directly in this folder and ship with both Products; Foxhound's sit in `foxhound/` and ship with Foxhound alone. Each skill is a `skill:` Feature named by its folder, which `xtask codegen` adds to `features/inventory.json` (ADR 0072).
