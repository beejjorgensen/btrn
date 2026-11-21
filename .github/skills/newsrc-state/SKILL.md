---
name: newsrc-state
description: "Implement or test newsrc parsing, subscriptions, read ranges, atomic writes, range normalization, comments, malformed entries, and server article-number read state."
---

# Newsrc state workflow

## Procedure

1. Parse subscription markers separately from read ranges.
2. Distinguish missing entries, subscribed `:`, and unsubscribed `!`.
3. Normalize reversed, duplicate, out-of-order, open-ended, and malformed ranges without corrupting state.
4. Keep read ranges keyed by canonical group name.
5. Synchronize group read state from newsrc state using server article numbers.
6. Write atomically and preserve unrelated data/comments only if round-trip behavior is intentionally supported.
7. Test malformed input and range normalization without a live server or terminal.

Use [newsrc checklist](./references/checklist.md) and
[persistence specification](../../../docs/persistence.md).
