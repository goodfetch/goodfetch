# Vendored skills

Third-party agent skills for the languages of [`STYLE.md`](../../STYLE.md), copied unchanged at the commits below so every agent and contributor reads the same version. `STYLE.md` wins wherever a skill disagrees. Codex reads this folder; Claude Code reads the same skills through the links in `.claude/skills/`. A personal skill with the same name takes precedence on that machine.

| Skill | Language | Source | Commit | License |
|---|---|---|---|---|
| `typescript-best-practices` | TypeScript | [michael-denyer/pstack-claude](https://github.com/michael-denyer/pstack-claude/tree/046341e3aaf2405083973a37f860d0d2ea98e18a/plugins/pstack/skills/typescript-best-practices) | `046341e` | MIT |
| `principle-type-system-discipline` | TypeScript (named by `typescript-best-practices`) | [michael-denyer/pstack-claude](https://github.com/michael-denyer/pstack-claude/tree/046341e3aaf2405083973a37f860d0d2ea98e18a/plugins/pstack/skills/principle-type-system-discipline) | `046341e` | MIT |
| `principle-boundary-discipline` | TypeScript (named by the two above) | [michael-denyer/pstack-claude](https://github.com/michael-denyer/pstack-claude/tree/046341e3aaf2405083973a37f860d0d2ea98e18a/plugins/pstack/skills/principle-boundary-discipline) | `046341e` | MIT |
| `principle-encode-lessons-in-structure` | TypeScript (named by `principle-type-system-discipline`) | [michael-denyer/pstack-claude](https://github.com/michael-denyer/pstack-claude/tree/046341e3aaf2405083973a37f860d0d2ea98e18a/plugins/pstack/skills/principle-encode-lessons-in-structure) | `046341e` | MIT |
| `ruff` | Python | [astral-sh/claude-code-plugins](https://github.com/astral-sh/claude-code-plugins/tree/f3ce88a7ba830f53afd6d944c1d0278ed318e142/plugins/astral/skills/ruff) | `f3ce88a` | MIT or Apache-2.0 |
| `uv` | Python | [astral-sh/claude-code-plugins](https://github.com/astral-sh/claude-code-plugins/tree/f3ce88a7ba830f53afd6d944c1d0278ed318e142/plugins/astral/skills/uv) | `f3ce88a` | MIT or Apache-2.0 |

Left out:

- **Astral's `ty` skill:** Goodfetch type-checks Python with pyright and mypy.
- **Rust skills:** the best one reviewed, Apollo's `rust-best-practices`, contradicts the Design and Rust sections of `STYLE.md` (trait-first generics, typestates for protocol state, `Arc<Mutex<…>>`, `std::thread::spawn`, global statics, `mod.rs`, `println!`). Rust uses the references named in `STYLE.md` instead.

Known disagreements, where `STYLE.md` wins:

- `typescript-best-practices` validates RPC payloads at the boundary, uses `zod` in its examples and throws `new Error`; the SDKs leave engine responses unvalidated beyond the protocol version check, copy the Standard Schema types instead of depending on a schema library, and throw the generated class of each Error code.

To update a skill, copy its folder again from a newer commit, update its row, and check the diff against `STYLE.md`.
