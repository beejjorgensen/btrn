---
description: "Use when editing specifications in docs/, AGENTS.md, CLAUDE.md, rules files in .github/instructions, or skills in .github/skills."
applyTo:
  - "docs/**"
  - "AGENTS.md"
  - "CLAUDE.md"
  - ".github/instructions/**"
  - ".github/skills/**"
---

# Documentation maintenance rules

Every fact has one home. Elsewhere, link to it instead of copying or
paraphrasing it. A rules file may condense a spec rule into one imperative
line, but must link to the spec for detail.

| Kind of fact | Home |
| --- | --- |
| Rules for every change | `AGENTS.md` |
| Rules for one path | `.github/instructions/*.instructions.md` |
| Procedures and checklists | `.github/skills/<name>/` (`SKILL.md` plus `references/`) |
| Behavior | `docs/<subsystem>.md` |
| Config keys and defaults | `docs/persistence.md` |
| Keys and screen layouts | `docs/screens.md` |
| Pseudo-terminal journeys | `docs/testing.md` |
| Module and path map | `docs/architecture.md` |

- Update the spec in the same change as the code that alters behavior. If a rule changed, also update its rules file and any skill checklist that mentions it.
- Every spec section heading carries a status label defined in `docs/README.md`. Change the label when a section becomes implemented or partly implemented.
- Describe current code with its real type, function, and path names. Keep target designs in a separate `Planned` section and never describe them as existing.
- State requirements with must, never, or should. Put tradeoffs and open questions under a heading ending in `— not binding`, at the end of the document.
- Keep rules files short. The `description` says when to use the file, and every `applyTo` pattern must match an existing path.
- Adding, removing, or renaming a rules file or skill means updating the "Subsystem rules" table in `AGENTS.md`.
- A skill has frontmatter `name` and `description` (when to use it), a numbered procedure, and checklists in `references/`. Skills live only in `.github/skills/`; `.claude/skills` is a symlink to it.
- Use relative links. Anchors follow GitHub heading slugs, so an em dash leaves a double hyphen, as in `testing.md#critical-e2e-journeys--planned`.
- Do not reflow text you are not otherwise changing.
- Never write secrets, sample passwords, or real account names into docs; use placeholders such as `your-account-name`.
- After editing, run `python3 scripts/check-docs.py`.
