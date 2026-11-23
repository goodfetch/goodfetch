# Agent skills

The agent skills Goodfetch publishes for its Products. Their area fills this folder.

Each skill is a folder holding a `SKILL.md`, in the Agent Skills format, with its references beside it: one source per topic, rendered for each Product that carries it (ADRs 0095 and 0097). Collie's topics sit directly in this folder and ship with both Products; Foxhound's sit in `foxhound/` and ship with Foxhound alone. Each skill is a `skill:` Feature named by its folder, which `xtask codegen` adds to `features/inventory.json` (ADR 0072).

`xtask codegen` renders each topic into each carrying Product's `generated/skills/<name>/` with the docs pages' syntax (ADR 0097): `{{executable}}` and the other name markers write the Product's names, in the front matter too, and a `{/* products: … */}` part ships with the Products it lists. A topic's `name` renders to its folder's name for Collie. The rendering fails a topic that breaks one of these:

- The front matter holds `name`, at most 64 characters of lowercase letters, digits and single hyphens, and `description`, at most 1,024 characters. Quote a description that holds `: `.
- An operation named in a code span, such as `run.start`, is one the Product's agents can call: neither another Product's nor the SDKs' alone.
- A command named in a code span that starts with `{{executable}}`, or on a `$ {{executable}} …` line of a `console` block, is one the executable has. Each `console` line is an example an agent may run.
- A person-only answer, such as `{{executable}} usage raise`, is never an example, and is named only on a line that asks the person to give it.
- No word of `wording.toml` appears.
