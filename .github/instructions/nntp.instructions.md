---
description: "Use when implementing or testing NNTP transport, wire parsing, XOVER/overview, authentication, greetings, capabilities, article HEAD/BODY/ARTICLE retrieval, or fake NNTP servers."
applyTo:
  - "src/nntp/**"
  - "src/protocol/**"
  - "tests/**/*nntp*"
  - "tests/fixtures/**/*"
---

# NNTP implementation rules

- Implement implicit TLS with system trust; never silently downgrade a configured TLS connection.
- Serialize one command and one response at a time.
- Bound command lines at 64 KiB and decoded multiline responses at 16 MiB.
- Unstuff exact dot lines and stop only at an exact `.\r\n` terminator.
- Keep status code, full first line, multiline data, and an error category in protocol responses.
- Do not convert NNTP bytes to UTF-8 in transport or protocol parsing.
- Parse `LIST ACTIVE` rows into typed group listings; preserve unknown posting flags and report malformed rows without discarding valid groups.
- Parse `GROUP` success responses into typed selections; retain server ranges when the reported count disagrees, and treat `411` as an explicit unknown-group failure.
- Parse `CAPABILITIES` into typed capabilities and prefer `XOVER` over `OVER` when both are advertised. Use the standard overview format only when the server does not provide one.
- Parse overview rows by the server-provided field order; preserve article numbers when optional fields are malformed, keep unknown/malformed field bytes for diagnostics, ignore out-of-range rows, and resolve duplicate numbers to the last complete row.
- Plan initial overview requests from the group's unread newsrc article-number ranges, bounded by `initial_overview_limit` and split into requests no larger than `overview_chunk_size`.
- Retrieve headers with `HEAD` and bodies with `BODY`; preserve response bytes and distinguish `423`/`430` article-not-found statuses from transport and unexpected-status failures.
- Never put passwords or authentication arguments in logs or errors.
- When reconnect/retry behavior is implemented, reconnect once after transport loss and retry temporary `4xx` or `480` authentication challenges at most once. Tests for operations that do not exercise retry behavior need not cover these cases.
- Never create a `Group` or cached `Article` after an unsuccessful operation.
- Read protocol fixtures and status-handling details from [architecture](../../docs/architecture.md).
