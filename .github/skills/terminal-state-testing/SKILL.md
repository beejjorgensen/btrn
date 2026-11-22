---
name: terminal-state-testing
description: "Test btrn terminal state, key dispatch, pagination, rendering snapshots, Crossterm frames, and pseudo-terminal end-to-end journeys without over-testing terminal combinations."
---

# Terminal state testing workflow

## Procedure

1. Implement or exercise named operations and screen transitions before terminal rendering.
2. Add state-transition tests for key dispatch, pagination, resize, marking, and article navigation.
3. Add representative snapshot tests for only meaningful layouts or inverse rendering states.
4. Use a pseudo-terminal only for the critical journeys in the [testing specification](../../../docs/testing.md#critical-e2e-journeys--planned).
5. Assert text and inverse state where practical; keep snapshots deterministic.
6. Run narrow state/snapshot tests before pseudo-terminal journeys.

Use [terminal coverage](./references/coverage.md) and
[testing specification](../../../docs/testing.md).
