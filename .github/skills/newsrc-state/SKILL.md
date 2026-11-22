---
name: newsrc-state
description: "Implement or test newsrc parsing, subscriptions, read ranges, atomic writes, range normalization, comments, malformed entries, and server article-number read state."
---

# Newsrc state workflow

The rules are in the
[persistence instructions](../../instructions/persistence.instructions.md);
this skill is the working procedure.

## Procedure

1. Parse subscription markers separately from read ranges.
2. Change range handling through the normalizer and the overview planners in `src/persistence/newsrc.rs`; do not normalize ranges elsewhere.
3. Synchronize group read state from newsrc state using server article numbers.
4. Test each case in the [newsrc checklist](./references/checklist.md) that the change touches, without a live server or terminal.
5. When changing overview planning, also run the fake-server tests in `src/group_loader.rs`.

See the [persistence specification](../../../docs/persistence.md).
