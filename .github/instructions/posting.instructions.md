---
description: "Use when implementing or testing followup posting: the posting check, warning screen, composition file, editor hand-off, header checks, POST, or the finalization menu."
applyTo:
  - "src/posting/**"
  - "src/nntp/post.rs"
  - "src/terminal/posting.rs"
---

# Posting implementation rules

- Posting never changes read state or newsrc.
- The temporary composition file is the only place an article body may touch disk: create it with a random name and mode `0600`, and delete it on send, abort, editor-launch failure, and program exit, including Ctrl-C.
- Launch the editor and `password_command` directly, never through a shell; split a string value on whitespace, and use an array value as given.
- Never retry `POST` automatically, directly or after a reconnect. A user-initiated resend reuses the article's generated `Message-ID`.
- A failed `POST`'s server response text may be shown to the user, with control characters replaced and limited to one line, but must never be logged.
- Keep header building and header checks testable without an editor, terminal, or socket; the terminal layer only displays prompts and results.
- Read the [posting specification](../../docs/posting.md) for header rules, the warning text, and the finalization menu.
