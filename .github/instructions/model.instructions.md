---
description: "Use when implementing or testing the btrn core model, thread relationships and placeholders, read ranges, session state, article navigation, or MIME/charset decoding."
applyTo:
  - "src/model/**"
  - "src/thread/**"
  - "src/state/**"
  - "src/decoder/**"
---

# Model implementation rules

- Identify articles by normalized Message-ID plus known server-number/group context; never use subject or local cache identity as identity.
- A parent missing from loaded metadata becomes a placeholder node (`ThreadRef::Missing`), chained through earlier `References` IDs; articles referencing the same missing ancestors share one thread. Placeholders are never read, unread, or navigable by next/previous, and are replaced only when the missing article's metadata is loaded (read-context rows or a fetch by Message-ID).
- A thread is identified by its first loaded article in walk order, not by its root node, because the root may be a placeholder.
- Do not infer parent relationships from subject lines or article position.
- Read state belongs to `NewsrcState`; model presentation must distinguish loaded metadata from presented/read articles.
- Pagination fields belong to layout state, not articles or threads.
- Server article numbers are scoped by group and are not assumed continuous.
- Preserve unknown overview fields for diagnostics without using a non-Message-ID field as article identity.
- Keep raw header values and article body bytes undecoded; unfold headers and report malformed lines at the model parser boundary.
- Preserve duplicate headers in wire order; single-value header lookup should be case-insensitive and deterministically use the last occurrence.
- Normalize valid message IDs by removing their surrounding angle brackets, and retain malformed identifier bytes in metadata diagnostics rather than using them for relationships.
- Build thread edges from Message-ID references only: prefer the last `References` ID, use `In-Reply-To` only when references are absent, and never infer parents from subjects or article order.
- Loading overview/article metadata must not change read state. Mark reads only through an explicit operation keyed by group and server article number; preserve an existing subscription marker and do not implicitly subscribe a group.
- Session operations must report explicit success/failure and mutate state only after a successful operation result.
- Read [core model specification](../../docs/model.md) for ownership and relationship rules and [decoding specification](../../docs/decoding.md) for MIME and charsets.
