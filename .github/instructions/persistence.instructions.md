---
description: "Use when implementing or testing configuration loading, config.toml keys, credential resolution, newsrc parsing, read ranges, subscriptions, or atomic newsrc writes."
applyTo:
  - "src/persistence/**"
  - "src/config.rs"
---

# Persistence implementation rules

- Newsrc stores only subscription markers and normalized read ranges of server article numbers, keyed by canonical group name. Never write article metadata, bodies, overview rows, headers, or thread structures.
- Distinguish a missing entry, a subscribed `:` entry, and an unsubscribed `!` entry. Marking an article read keeps an existing marker and creates a new entry as unsubscribed.
- Normalize reversed, duplicate, overlapping, and out-of-order ranges, and report malformed input without corrupting valid ranges.
- Write newsrc through a same-directory temporary file, flush it, then atomically replace the destination.
- Config accepts only the documented top-level keys; unknown keys, tables, and repeated keys are errors. Config errors report only a category and line number, never config text or values.
- Read passwords at connection time and never write them to disk. Configure exactly one of `password` and `password_command` when `username` is set.
- Read the [persistence specification](../../docs/persistence.md) for keys and defaults.
