---
description: "Use when implementing terminal rendering, Crossterm input, key bindings, screen state, layouts, pagination, snapshots, or pseudo-terminal end-to-end tests."
applyTo:
  - "src/terminal/**"
  - "src/ui/**"
  - "src/screens/**"
  - "tests/**/*terminal*"
  - "tests/**/*snapshot*"
---

# Terminal implementation rules

- Use Crossterm directly; do not introduce a higher-level terminal framework.
- Translate raw keys into named operations before changing `SessionState`.
- Keep screen dispatch tables or binding definitions separate from rendering functions.
- Renderers must not open network connections or read/write files.
- Key handling and screen transitions must work without a real terminal or socket.
- Generated selection letters must coexist with reserved keys without scattering checks through renderers.
- Page data is derived layout state and must refresh after terminal resize.
- The currently presented article is fully inverse in the thread hierarchy; previously viewed articles invert only their numbers.
- Append `(?)` only to unresolved-parent hierarchy entries.
- Keep pseudo-terminal coverage limited to the critical journeys defined in [architecture](../../docs/architecture.md).
