---
description: "Use when implementing or testing the btrn core model, thread relationships, read ranges, session state, overview loading, pagination state, or article navigation."
applyTo:
  - "src/model/**"
  - "src/thread/**"
  - "src/state/**"
  - "tests/**/*model*"
  - "tests/**/*thread*"
---

# Model implementation rules

- Identify articles by normalized Message-ID plus known server-number/group context; never use subject or local cache identity as identity.
- Unknown parents remain unresolved in the currently loaded view and use literal `(?)`. Resolve them later only from loaded metadata.
- Do not infer parent relationships from subject lines or article position.
- Read state belongs to `NewsrcState`; model presentation must distinguish loaded metadata from presented/read articles.
- Pagination fields belong to layout state, not articles or threads.
- Server article numbers are scoped by group and are not assumed continuous.
- Preserve unknown overview fields for diagnostics without using a non-Message-ID field as article identity.
- Keep raw header values and article body bytes undecoded; unfold headers and report malformed lines at the model parser boundary.
- Preserve duplicate headers in wire order; single-value header lookup should be case-insensitive and deterministically use the last occurrence.
- Normalize valid message IDs by removing their surrounding angle brackets, and retain malformed identifier bytes in metadata diagnostics rather than using them for relationships.
- Session operations must report explicit success/failure and mutate state only after a successful operation result.
- Read [architecture](../../docs/architecture.md) for the complete ownership and relationship rules.
